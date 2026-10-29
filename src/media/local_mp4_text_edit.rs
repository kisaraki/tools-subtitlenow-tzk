//! One static rate-one tx3g interval, optionally preceded by an empty edit.
//! No repeat, dwell, speed change or playback clock.
use super::*;

const EDIT_V0_ENTRY_BYTES: usize = 12;
const EDIT_V1_ENTRY_BYTES: usize = 20;
const RATE_ONE: u32 = 0x0001_0000;

// The exact table length is checked before either fixed-width entry is read.
// Signed media_time is decoded at its declared width; never truncate v1 to v0.
fn entry(table: &[u8], offset: usize, version: u8) -> Result<(u64, i64, u32), MediaError> {
    match version {
        0 => Ok((
            u64::from(u32_at(table, offset)?),
            i64::from(u32_at(table, offset + 4)? as i32),
            u32_at(table, offset + 8)?,
        )),
        1 => Ok((
            u64_at(table, offset)?,
            u64_at(table, offset + 8)? as i64,
            u32_at(table, offset + 16)?,
        )),
        _ => Err(MediaError::UnsupportedMedia),
    }
}

#[derive(Clone, Copy)]
pub(super) struct TextEdit {
    movie_scale: u32,
    text_scale: u32,
    media_duration: u64,
    delay: u64,
    media_start: u64,
    active_duration: u64,
    edited: bool,
}
impl TextEdit {
    pub(super) fn parse(
        edts: Option<&[u8]>,
        movie_scale: u32,
        text_scale: u32,
        media_duration: u64,
        track_duration: u64,
        movie_duration: u64,
        cancel: &ResolverCancellation,
    ) -> Result<Self, MediaError> {
        check(cancel)?;
        require(movie_scale > 0 && text_scale > 0 && media_duration > 0)?;
        let scaled = u128::from(media_duration) * u128::from(movie_scale);
        let floor = scaled / u128::from(text_scale);
        let ceil = scaled.div_ceil(u128::from(text_scale));
        let mut edit = Self {
            movie_scale,
            text_scale,
            media_duration,
            delay: 0,
            media_start: 0,
            active_duration: 0,
            edited: edts.is_some(),
        };
        if let Some(edts) = edts {
            let atoms = children(edts, &[b"elst"], cancel)?;
            let table = get(&atoms, b"elst")?;
            let (version, entry_bytes) = match u32_at(table, 0)? {
                0 => (0, EDIT_V0_ENTRY_BYTES),
                0x0100_0000 => (1, EDIT_V1_ENTRY_BYTES),
                _ => return Err(MediaError::UnsupportedMedia), // Unknown version or flags.
            };
            let count = u32_at(table, 4)? as usize;
            require(matches!(count, 1 | 2))?;
            require(table.len() == 8 + count * entry_bytes)?;
            if count == 2 {
                let (delay, time, rate) = entry(table, 8, version)?;
                require(delay > 0 && time == -1 && rate == RATE_ONE)?;
                edit.delay = delay;
            }
            let active = 8 + (count - 1) * entry_bytes;
            let (duration, media_start, rate) = entry(table, active, version)?;
            require(duration > 0 && rate == RATE_ONE)?;
            edit.media_start =
                u64::try_from(media_start).map_err(|_| MediaError::UnsupportedMedia)?;
            edit.active_duration = duration;
            // Compare in exact common units; a fractional last media tick is
            // a real edit boundary, never a rounded extension past the source.
            require(edit.media_end() <= scaled)?;
            let total = edit
                .delay
                .checked_add(duration)
                .ok_or(MediaError::UnsupportedMedia)?;
            require(track_duration == total && movie_duration >= total)?;
        } else {
            // Preserve the pre-existing no-edit header rounding rule. The
            // actual samples keep their original media ticks in this branch.
            require(
                (u128::from(track_duration) == floor || u128::from(track_duration) == ceil)
                    && u128::from(movie_duration) >= ceil,
            )?;
        }
        // Include a leading empty edit in the existing bounded timeline limit.
        let presentation_end = if edit.edited {
            (u128::from(edit.delay) + u128::from(edit.active_duration))
                * u128::from(edit.text_scale)
        } else {
            scaled
        };
        edit.presentation_us(presentation_end)?;
        check(cancel)?;
        Ok(edit)
    }

    pub(super) fn media_timescale(self) -> u32 {
        self.text_scale
    }
    pub(super) fn is_edited(self) -> bool {
        self.edited
    }

    /// Presentation microseconds for the overlap; None is entirely outside the
    /// edit. Callers still decode/validate every original sample before filtering.
    pub(super) fn sample_time(
        self,
        start: u64,
        duration: u64,
    ) -> Result<Option<(u64, u64)>, MediaError> {
        let end = start
            .checked_add(duration)
            .filter(|end| duration > 0 && *end <= self.media_duration)
            .ok_or(MediaError::InvalidSubtitle)?;
        let unit = u128::from(self.movie_scale);
        let origin = u128::from(self.media_start) * unit;
        let first = (u128::from(start) * unit).max(origin);
        let last = (u128::from(end) * unit).min(if self.edited {
            self.media_end()
        } else {
            u128::from(self.media_duration) * unit
        });
        if first >= last {
            return Ok(None);
        }
        let delay = u128::from(self.delay) * u128::from(self.text_scale);
        let start_us = self.presentation_us(delay + first - origin)?;
        let end_us = self.presentation_us(delay + last - origin)?;
        let duration_us = end_us
            .checked_sub(start_us)
            .filter(|duration| *duration > 0)
            .ok_or(MediaError::InvalidSubtitle)?;
        Ok(Some((start_us, duration_us)))
    }
    fn media_end(self) -> u128 {
        u128::from(self.media_start) * u128::from(self.movie_scale)
            + u128::from(self.active_duration) * u128::from(self.text_scale)
    }
    fn presentation_us(self, common_ticks: u128) -> Result<u64, MediaError> {
        // Floor the complete rational endpoint once. Separately rounding the
        // delay and media endpoint can lose one microsecond of phase, while
        // accumulating rounded durations would introduce cumulative drift.
        let numerator = common_ticks
            .checked_mul(MICROS_PER_SECOND)
            .ok_or(MediaError::TooLarge)?;
        let denominator = u128::from(self.movie_scale) * u128::from(self.text_scale);
        let micros = numerator / denominator;
        u64::try_from(micros)
            .ok()
            .filter(|micros| *micros <= MAX_DECODED_MEDIA_DURATION_US)
            .ok_or(MediaError::TooLarge)
    }
}

#[cfg(test)]
#[path = "local_mp4_text_edit_v1_tests.rs"]
mod v1_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn table(entries: &[[u32; 3]]) -> Vec<u8> {
        let mut body = vec![0; 4];
        body.extend((entries.len() as u32).to_be_bytes());
        body.extend(entries.iter().flatten().flat_map(|word| word.to_be_bytes()));
        let mut atom = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        atom.extend(b"elst");
        atom.extend(body);
        atom
    }
    fn parse(edts: &[u8], track_duration: u64) -> Result<TextEdit, MediaError> {
        TextEdit::parse(
            Some(edts),
            1000,
            90_000,
            180_000,
            track_duration,
            track_duration,
            &ResolverCancellation::default(),
        )
    }
    #[test]
    fn mp4_text_crop_helper_uses_exact_half_open_intersections() {
        let data = table(&[[500, u32::MAX, RATE_ONE], [1400, 18000, RATE_ONE]]);
        let edit = parse(&data, 1900).unwrap(); // media_start=0.2s of a 2s source
        assert_eq!(edit.sample_time(0, 18000).unwrap(), None);
        assert_eq!(
            edit.sample_time(0, 63000).unwrap(),
            Some((500_000, 500_000))
        );
        assert_eq!(
            edit.sample_time(99000, 81000).unwrap(),
            Some((1_400_000, 500_000))
        );
        assert_eq!(edit.sample_time(144000, 36000).unwrap(), None);
    }
    #[test]
    fn mp4_text_crop_helper_rejects_negative_media_time_and_extension_past_media() {
        for (start, duration) in [(u32::MAX, 1), (0x80000000, 1), (180001, 1), (90000, 1001)] {
            assert!(parse(&table(&[[duration, start, RATE_ONE]]), u64::from(duration)).is_err());
        }
        let data = table(&[[1000, 90000, RATE_ONE]]);
        assert!(parse(&data, 1000).is_ok()); // exactly to media end
    }

    #[test]
    fn text_edit_full_track_and_leading_empty_map_absolute_endpoints() {
        for delay in [0, 123] {
            let active = [2000, 0, RATE_ONE];
            let data = if delay == 0 {
                table(&[active])
            } else {
                table(&[[delay, u32::MAX, RATE_ONE], active])
            };
            let edit = parse(&data, u64::from(2000 + delay)).unwrap();
            assert_eq!(edit.media_timescale(), 90_000);
            assert_eq!(
                edit.sample_time(0, 3001).unwrap().unwrap(),
                (u64::from(delay) * 1000, 33_344)
            );
            let (start, duration) = edit.sample_time(3001, 3001).unwrap().unwrap();
            assert_eq!(start, u64::from(delay) * 1000 + 33_344);
            assert_eq!(start + duration, u64::from(delay) * 1000 + 66_688);
        }
    }

    #[test]
    fn text_edit_fractional_delay_and_media_phase_are_rounded_together() {
        let data = table(&[[1, u32::MAX, RATE_ONE], [3, 0, RATE_ONE]]);
        let edit = TextEdit::parse(
            Some(&data),
            3,
            90_000,
            90_000,
            4,
            4,
            &ResolverCancellation::default(),
        )
        .unwrap();
        let (start, duration) = edit.sample_time(1, 1).unwrap().unwrap();
        assert_eq!(start, 333_344);
        assert_eq!(start + duration, 333_355);
        // floor(1/3s) + floor(2/90000s) would incorrectly produce 333354.
        let mut previous = edit.sample_time(0, 1).unwrap().unwrap();
        for index in 1..6000 {
            let next = edit.sample_time(index, 1).unwrap().unwrap();
            assert_eq!(previous.0 + previous.1, next.0);
            previous = next;
        }
    }

    #[test]
    fn text_edit_no_edit_preserves_original_ticks_and_header_rounding() {
        for declared in [23, 24] {
            let edit = TextEdit::parse(
                None,
                1000,
                90_000,
                2100,
                declared,
                24,
                &ResolverCancellation::default(),
            )
            .unwrap();
            assert_eq!(edit.media_timescale(), 90_000);
            assert_eq!(edit.sample_time(700, 400).unwrap().unwrap(), (7777, 4445));
        }
        let data = table(&[[23, 0, RATE_ONE]]);
        let trimmed = TextEdit::parse(
            Some(&data),
            1000,
            90_000,
            2100,
            23,
            24,
            &ResolverCancellation::default(),
        )
        .unwrap();
        assert_eq!(
            trimmed.sample_time(1100, 1000).unwrap(),
            Some((12222, 10778))
        );
    }

    #[test]
    fn text_edit_rejects_invalid_ranges_rates_empty_only_repeat_and_unknown_shape() {
        for entries in [
            vec![],
            vec![[2000, 1, RATE_ONE]],
            vec![[2000, u32::MAX, RATE_ONE]],
            vec![[0, 0, RATE_ONE]],
            vec![[1999, 0, RATE_ONE]],
            vec![[2001, 0, RATE_ONE]],
            vec![[2000, 0, 0]],
            vec![[2000, 0, 0x0000_ffff]],
            vec![[2000, 0, 0x0002_0000]],
            vec![[0, u32::MAX, RATE_ONE], [2000, 0, RATE_ONE]],
            vec![[1, u32::MAX, 0], [2000, 0, RATE_ONE]],
            vec![[1, 0, RATE_ONE], [2000, 0, RATE_ONE]],
            vec![[1, u32::MAX, RATE_ONE], [2000, u32::MAX, RATE_ONE]],
            vec![[1, u32::MAX, RATE_ONE], [2000, 1, RATE_ONE]],
            vec![
                [1, u32::MAX, RATE_ONE],
                [2000, 0, RATE_ONE],
                [1, 0, RATE_ONE],
            ],
        ] {
            assert!(matches!(
                parse(&table(&entries), 2000),
                Err(MediaError::UnsupportedMedia)
            ));
        }
    }

    #[test]
    fn text_edit_rejects_truncation_version_flags_duplicate_and_extra_boxes() {
        let valid = table(&[[2000, 0, RATE_ONE]]);
        for end in 0..valid.len() {
            assert!(matches!(
                parse(&valid[..end], 2000),
                Err(MediaError::UnsupportedMedia)
            ));
        }
        for at in [8, 9, 10, 11] {
            let mut changed = valid.clone();
            changed[at] = 1;
            assert!(matches!(
                parse(&changed, 2000),
                Err(MediaError::UnsupportedMedia)
            ));
        }
        let mut duplicate = valid.clone();
        duplicate.extend(&valid);
        assert!(matches!(
            parse(&duplicate, 2000),
            Err(MediaError::UnsupportedMedia)
        ));
        let mut extra = valid.clone();
        extra.extend([0, 0, 0, 8, b'f', b'r', b'e', b'e']);
        assert!(matches!(
            parse(&extra, 2000),
            Err(MediaError::UnsupportedMedia)
        ));
        let mut trailing = valid.clone();
        trailing.push(0);
        let size = trailing.len() as u32;
        trailing[..4].copy_from_slice(&size.to_be_bytes());
        assert!(matches!(
            parse(&trailing, 2000),
            Err(MediaError::UnsupportedMedia)
        ));
    }

    #[test]
    fn text_edit_track_duration_and_movie_coverage_are_exact() {
        let data = table(&[[123, u32::MAX, RATE_ONE], [2000, 0, RATE_ONE]]);
        for (track, movie) in [(2122, 2123), (2124, 2124), (2123, 2122)] {
            assert!(matches!(
                TextEdit::parse(
                    Some(&data),
                    1000,
                    90_000,
                    180_000,
                    track,
                    movie,
                    &ResolverCancellation::default(),
                ),
                Err(MediaError::UnsupportedMedia)
            ));
        }
    }

    #[test]
    fn text_edit_limit_includes_delay_and_samples_cannot_overrun_or_collapse() {
        let duration = MAX_DECODED_MEDIA_DURATION_US / 1_000_000;
        let data = table(&[[1, u32::MAX, RATE_ONE], [duration as u32 - 1, 0, RATE_ONE]]);
        let edit = TextEdit::parse(
            Some(&data),
            1,
            1,
            duration - 1,
            duration,
            duration,
            &ResolverCancellation::default(),
        )
        .unwrap();
        let (start, span) = edit.sample_time(0, duration - 1).unwrap().unwrap();
        assert_eq!(start + span, MAX_DECODED_MEDIA_DURATION_US);
        assert!(matches!(
            edit.sample_time(0, duration),
            Err(MediaError::InvalidSubtitle)
        ));
        assert!(matches!(
            edit.sample_time(u64::MAX, 1),
            Err(MediaError::InvalidSubtitle)
        ));
        assert!(matches!(
            edit.sample_time(0, 0),
            Err(MediaError::InvalidSubtitle)
        ));
        let data = table(&[[2, u32::MAX, RATE_ONE], [duration as u32 - 1, 0, RATE_ONE]]);
        assert!(matches!(
            TextEdit::parse(
                Some(&data),
                1,
                1,
                duration - 1,
                duration + 1,
                duration + 1,
                &ResolverCancellation::default(),
            ),
            Err(MediaError::TooLarge)
        ));
        let data = table(&[[1, 0, RATE_ONE]]);
        let edit = TextEdit::parse(
            Some(&data),
            1,
            2_000_000,
            2_000_000,
            1,
            1,
            &ResolverCancellation::default(),
        )
        .unwrap();
        assert!(matches!(
            edit.sample_time(0, 1),
            Err(MediaError::InvalidSubtitle)
        ));
    }

    #[test]
    fn text_edit_cancellation_and_invalid_scales_reject_before_mapping() {
        let data = table(&[[2000, 0, RATE_ONE]]);
        let cancel = ResolverCancellation::default();
        cancel.cancel();
        assert!(matches!(
            TextEdit::parse(Some(&data), 1000, 90_000, 180_000, 2000, 2000, &cancel),
            Err(MediaError::Cancelled)
        ));
        for (movie, text, duration) in [(0, 90_000, 180_000), (1000, 0, 180_000), (1000, 90_000, 0)]
        {
            assert!(matches!(
                TextEdit::parse(
                    None,
                    movie,
                    text,
                    duration,
                    2000,
                    2000,
                    &ResolverCancellation::default(),
                ),
                Err(MediaError::UnsupportedMedia)
            ));
        }
    }
}
