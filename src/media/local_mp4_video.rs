//! Descriptor checks only. Video is never selected for product decoding.
use super::*;

const MAX_VIDEO_DIMENSION: u16 = 8192;
const MAX_AVC_CONFIG_BYTES: usize = 64 * 1024;

pub(super) fn validate_stsd(data: &[u8], cancel: &ResolverCancellation) -> Result<(), MediaError> {
    full_zero(data)?;
    require(u32_at(data, 4)? == 1)?;
    let entries = children(
        data.get(8..).ok_or(MediaError::UnsupportedMedia)?,
        &[b"avc1"],
        cancel,
    )?;
    let entry = get(&entries, b"avc1")?;
    require(entry.len() >= 78)?;
    require(entry[..6] == [0; 6] && entry[6..8] == [0, 1] && entry[8..24] == [0; 16])?;
    let dimension = |at| u16::from_be_bytes([entry[at], entry[at + 1]]);
    require(
        (1..=MAX_VIDEO_DIMENSION).contains(&dimension(24))
            && (1..=MAX_VIDEO_DIMENSION).contains(&dimension(26)),
    )?;
    // VisualSampleEntry's compressor name is a bounded Pascal string. No
    // external data reference or unknown native extension is passed through.
    require(entry[40..42] == [0, 1] && entry[42] <= 31 && entry[74..78] == [0, 24, 255, 255])?;
    let extensions = children(&entry[78..], &[b"avcC", b"btrt", b"pasp"], cancel)?;
    if let Some(bitrate) = extensions.iter().find(|a| a.kind == b"btrt") {
        require(bitrate.body.len() == 12)?;
    }
    if let Some(aspect) = extensions.iter().find(|a| a.kind == b"pasp") {
        require(
            aspect.body.len() == 8 && u32_at(aspect.body, 0)? > 0 && u32_at(aspect.body, 4)? > 0,
        )?;
    }
    validate_config(get(&extensions, b"avcC")?, cancel)
}

fn validate_config(data: &[u8], cancel: &ResolverCancellation) -> Result<(), MediaError> {
    check(cancel)?;
    require((7..=MAX_AVC_CONFIG_BYTES).contains(&data.len()))?;
    // Ordinary 8-bit profiles only. AVC framing reserves lengthSizeMinusOne=2.
    require(
        data[0] == 1
            && matches!(data[1], 66 | 77 | 100)
            && data[2] & 3 == 0
            && data[3] > 0
            && data[4] & 0xfc == 0xfc
            && data[4] & 3 != 2
            && data[5] & 0xe0 == 0xe0,
    )?;
    let sequence_count = data[5] & 31;
    require(sequence_count > 0)?;
    let mut offset = 6;
    for _ in 0..sequence_count {
        parameter(data, &mut offset, 7, cancel)?;
    }
    let picture_count = *data.get(offset).ok_or(MediaError::UnsupportedMedia)?;
    require(picture_count > 0)?;
    offset += 1;
    for _ in 0..picture_count {
        parameter(data, &mut offset, 8, cancel)?;
    }
    if data[1] == 100 && offset < data.len() {
        // Native muxers may omit this extension. When present, accept only
        // the verified 4:2:0 / 8-bit / no SPS-ext form, not opaque trailing data.
        require(data.get(offset..) == Some(&[0xfd, 0xf8, 0xf8, 0]))?;
        offset += 4;
    }
    require(offset == data.len())
}

fn parameter(
    data: &[u8],
    offset: &mut usize,
    kind: u8,
    cancel: &ResolverCancellation,
) -> Result<(), MediaError> {
    check(cancel)?;
    let length = data
        .get(*offset..*offset + 2)
        .ok_or(MediaError::UnsupportedMedia)?;
    let length = usize::from(u16::from_be_bytes([length[0], length[1]]));
    *offset += 2;
    let body = data
        .get(*offset..*offset + length)
        .ok_or(MediaError::UnsupportedMedia)?;
    require(
        body.first()
            .is_some_and(|b| b & 0x80 == 0 && b & 31 == kind),
    )?;
    // SPS's profile/compatibility/level must match the configuration header.
    if kind == 7 {
        require(body.get(1..4) == Some(&data[1..4]))?;
        if data[1] == 100 {
            let mut bits = SpsHeaderBits::new(&body[4..]);
            require(bits.unsigned_exp_golomb()? <= 31)?; // SPS ID
            require(bits.unsigned_exp_golomb()? == 1)?; // 4:2:0 chroma_format_idc
            require(bits.unsigned_exp_golomb()? == 0)?; // luma depth minus eight
            require(bits.unsigned_exp_golomb()? == 0)?; // chroma depth minus eight
        }
    }
    *offset += length;
    Ok(())
}

// Only four High-profile format fields, with bounded bit traversal and
// on-the-fly emulation-prevention removal. No RBSP copy or video decoder.
struct SpsHeaderBits<'a> {
    bytes: &'a [u8],
    position: usize,
    current: u8,
    remaining: u8,
    zeros: u8,
}
impl<'a> SpsHeaderBits<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            position: 0,
            current: 0,
            remaining: 0,
            zeros: 0,
        }
    }
    fn bit(&mut self) -> Result<u8, MediaError> {
        if self.remaining == 0 {
            let mut byte = *self
                .bytes
                .get(self.position)
                .ok_or(MediaError::UnsupportedMedia)?;
            self.position += 1;
            if self.zeros >= 2 && byte == 3 {
                byte = *self
                    .bytes
                    .get(self.position)
                    .ok_or(MediaError::UnsupportedMedia)?;
                require(byte <= 3)?;
                self.position += 1;
                self.zeros = 0;
            }
            self.zeros = if byte == 0 {
                (self.zeros + 1).min(2)
            } else {
                0
            };
            self.current = byte;
            self.remaining = 8;
        }
        self.remaining -= 1;
        Ok((self.current >> self.remaining) & 1)
    }
    fn unsigned_exp_golomb(&mut self) -> Result<u32, MediaError> {
        // Accepted fields fit in five value bits (SPS IDs <=31). Refuse long
        // prefixes before scanning attacker-controlled parameter-set bytes.
        const MAX_HEADER_PREFIX_ZEROS: u8 = 5;
        let mut zeros = 0;
        while self.bit()? == 0 {
            zeros += 1;
            require(zeros <= MAX_HEADER_PREFIX_ZEROS)?;
        }
        let mut value = 1u32;
        for _ in 0..zeros {
            value = (value << 1) | u32::from(self.bit()?);
        }
        Ok(value - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn avc_config_is_bounded_exact_and_cancellable() {
        let bytes = [1, 66, 0, 10, 255, 225, 0, 4, 103, 66, 0, 10, 1, 0, 1, 104];
        let cancel = ResolverCancellation::default();
        assert_eq!(validate_config(&bytes, &cancel), Ok(()));
        for end in 0..bytes.len() {
            assert_eq!(
                validate_config(&bytes[..end], &cancel),
                Err(MediaError::UnsupportedMedia)
            );
        }
        let mut trailing = bytes.to_vec();
        trailing.push(0);
        assert_eq!(
            validate_config(&trailing, &cancel),
            Err(MediaError::UnsupportedMedia)
        );
        let mut oversized = bytes.to_vec();
        oversized.resize(MAX_AVC_CONFIG_BYTES + 1, 0);
        assert_eq!(
            validate_config(&oversized, &cancel),
            Err(MediaError::UnsupportedMedia)
        );
        cancel.cancel();
        assert_eq!(validate_config(&bytes, &cancel), Err(MediaError::Cancelled));
    }
    fn high_config(format: &[u8]) -> Vec<u8> {
        let sequence = [vec![103, 100, 0, 10], format.to_vec()].concat();
        [
            vec![1, 100, 0, 10, 255, 225],
            (sequence.len() as u16).to_be_bytes().to_vec(),
            sequence,
            vec![1, 0, 1, 104],
        ]
        .concat()
    }
    fn format_fields(values: &[u32]) -> Vec<u8> {
        let mut bits = Vec::new();
        for value in values {
            let code = value + 1;
            let width = 32 - code.leading_zeros();
            bits.extend(std::iter::repeat_n(0, (width - 1) as usize));
            for shift in (0..width).rev() {
                bits.push(((code >> shift) & 1) as u8);
            }
        }
        while !bits.len().is_multiple_of(8) {
            bits.push(0);
        }
        bits.as_chunks::<8>()
            .0
            .iter()
            .map(|bits| bits.iter().fold(0, |value, bit| (value << 1) | bit))
            .collect()
    }
    #[test]
    fn avc_main_high_and_optional_high_extension_are_checked() {
        let cancel = ResolverCancellation::default();
        let main = [1, 77, 0, 10, 255, 225, 0, 4, 103, 77, 0, 10, 1, 0, 1, 104];
        assert_eq!(validate_config(&main, &cancel), Ok(()));
        let high = high_config(&[0xac]); // SPS ID=0, 4:2:0, two 8-bit depths.
        assert_eq!(validate_config(&high, &cancel), Ok(()));
        for (extension, valid) in [
            (vec![0xfd, 0xf8, 0xf8, 0], true),
            (vec![0xfc, 0xf8, 0xf8, 0], false),
            (vec![0xfe, 0xf8, 0xf8, 0], false),
            (vec![0xfd, 0xf9, 0xf8, 0], false),
            (vec![0xfd, 0xf8, 0xf9, 0], false),
            (vec![0xfd, 0xf8, 0xf8, 1], false),
            (vec![0xfd, 0xf8, 0xf8], false),
            (vec![0xfd, 0xf8, 0xf8, 0, 0], false),
        ] {
            let config = [high.clone(), extension].concat();
            assert_eq!(validate_config(&config, &cancel).is_ok(), valid);
        }
        for profile in [0, 44, 88, 110, 122, 244] {
            let mut config = main;
            config[1] = profile;
            config[9] = profile;
            assert_eq!(
                validate_config(&config, &cancel),
                Err(MediaError::UnsupportedMedia)
            );
        }
        let mut extra = main.to_vec();
        extra.extend([0xfd, 0xf8, 0xf8, 0]);
        assert_eq!(
            validate_config(&extra, &cancel),
            Err(MediaError::UnsupportedMedia)
        );
    }
    #[test]
    fn avc_high_sps_verifies_chroma_depth_and_bounded_parameter_id() {
        let cancel = ResolverCancellation::default();
        for (id, chroma, luma, depth, valid) in [
            (0, 1, 0, 0, true),
            (31, 1, 0, 0, true),
            (32, 1, 0, 0, false),
            (255, 1, 0, 0, false),
            (0, 0, 0, 0, false),
            (0, 2, 0, 0, false),
            (0, 3, 0, 0, false),
            (0, 1, 2, 0, false),
            (0, 1, 0, 2, false),
        ] {
            let config = high_config(&format_fields(&[id, chroma, luma, depth]));
            assert_eq!(validate_config(&config, &cancel).is_ok(), valid);
        }
        assert_eq!(
            validate_config(&high_config(&[]), &cancel),
            Err(MediaError::UnsupportedMedia)
        );
        assert_eq!(
            validate_config(&high_config(&[0; 100]), &cancel),
            Err(MediaError::UnsupportedMedia)
        );
        let mut wrong = high_config(&[0xac]);
        wrong[9] = 77;
        assert_eq!(
            validate_config(&wrong, &cancel),
            Err(MediaError::UnsupportedMedia)
        );
    }
    #[test]
    fn sps_header_bit_reader_removes_only_valid_emulation_prevention() {
        for value in 0..=3u8 {
            let input = [0, 0, 3, value, 128];
            let mut reader = SpsHeaderBits::new(&input);
            let mut output = Vec::new();
            for _ in 0..4 {
                let mut byte = 0;
                for _ in 0..8 {
                    byte = (byte << 1) | reader.bit().unwrap();
                }
                output.push(byte);
            }
            assert_eq!(output, [0, 0, value, 128]);
            assert_eq!(reader.bit(), Err(MediaError::UnsupportedMedia));
        }
        for input in [&[0, 0, 3][..], &[0, 0, 3, 4][..]] {
            let mut reader = SpsHeaderBits::new(input);
            for _ in 0..16 {
                assert_eq!(reader.bit(), Ok(0));
            }
            assert_eq!(reader.bit(), Err(MediaError::UnsupportedMedia));
        }
    }
    #[test]
    fn avc_high_truncation_mutation_and_cancellation_are_bounded() {
        let config = high_config(&[0xac]);
        let cancel = ResolverCancellation::default();
        for end in 0..config.len() {
            assert_eq!(
                validate_config(&config[..end], &cancel),
                Err(MediaError::UnsupportedMedia)
            );
        }
        for index in 0..config.len() {
            for value in [0, 255] {
                let mut changed = config.clone();
                changed[index] = value;
                assert!(std::panic::catch_unwind(|| validate_config(&changed, &cancel)).is_ok());
            }
        }
        cancel.cancel();
        for bytes in [&[][..], &config[..]] {
            assert_eq!(validate_config(bytes, &cancel), Err(MediaError::Cancelled));
        }
    }
}
