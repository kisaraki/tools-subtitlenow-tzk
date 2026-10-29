//! Bounded subtitle appearance and premultiplied pixels, independent of native handles.

pub const FONT_SIZES: [u16; 8] = [16, 20, 24, 28, 36, 48, 64, 72];
pub const BACKGROUND_LEVELS: [u8; 5] = [0, 25, 50, 70, 100];
pub const MAX_LINES: u8 = 6;
pub const MAX_SURFACE_WIDTH: usize = 4096;
pub const MAX_SURFACE_HEIGHT: usize = 2048;
pub const MAX_SURFACE_PIXELS: usize = 4 * 1024 * 1024;
pub const PADDING_DIP: i32 = 12;
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum MonitorPreference {
    #[default]
    Primary,
    Specific(String),
    FollowSource,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SideDock {
    Left,
    Right,
    #[default]
    Floating,
}
pub const SIDE_WIDTH_DIP: i32 = 460;
const OUTLINE_DIP: u32 = 2;
const SHADOW_DIP: u32 = 3;
const SHADOW_ALPHA: u32 = 160;
const MAX_EFFECT_PIXELS: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SubtitleFont {
    #[default]
    JhengHeiUi,
    JhengHei,
    MingLiU,
}
impl SubtitleFont {
    pub const ALL: [Self; 3] = [Self::JhengHeiUi, Self::JhengHei, Self::MingLiU];
    pub fn face(self) -> &'static str {
        match self {
            Self::JhengHeiUi => "Microsoft JhengHei UI",
            Self::JhengHei => "Microsoft JhengHei",
            Self::MingLiU => "PMingLiU",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::JhengHeiUi => "微軟正黑體 UI",
            Self::JhengHei => "微軟正黑體",
            Self::MingLiU => "新細明體",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubtitleAppearance {
    pub font: SubtitleFont,
    /// Logical pixel height at 96 DPI, rather than typographic points.
    pub font_size: u16,
    pub outline: bool,
    pub shadow: bool,
    pub background_percent: u8,
    pub max_lines: u8,
}
impl Default for SubtitleAppearance {
    fn default() -> Self {
        Self {
            font: SubtitleFont::default(),
            font_size: 28,
            outline: true,
            shadow: true,
            background_percent: 70,
            max_lines: 3,
        }
    }
}
impl SubtitleAppearance {
    pub fn valid(self) -> bool {
        FONT_SIZES.contains(&self.font_size)
            && self.background_percent <= 100
            && (1..=MAX_LINES).contains(&self.max_lines)
    }
    pub fn height_dip(self) -> i32 {
        // Reserve 1.5 em per line for CJK metrics, plus padding and effects.
        i32::from(self.font_size) * 3 / 2 * i32::from(self.max_lines)
            + 2 * PADDING_DIP
            + 2 * SHADOW_DIP as i32
    }
}
pub fn surface_len(width: usize, height: usize) -> Option<usize> {
    if width == 0 || height == 0 || width > MAX_SURFACE_WIDTH || height > MAX_SURFACE_HEIGHT {
        return None;
    }
    width
        .checked_mul(height)
        .filter(|len| *len <= MAX_SURFACE_PIXELS)
}
fn over(foreground: u32, background: u32) -> u32 {
    foreground + (background * (255 - foreground) + 127) / 255
}
/// White-on-black grayscale GDI coverage becomes white text over black effects/background.
/// Text opacity is independent of background opacity. No screen pixels are an input.
pub fn composite(
    mask: &[u32],
    output: &mut [u32],
    width: usize,
    height: usize,
    dpi: u32,
    style: SubtitleAppearance,
) -> bool {
    let Some(len) = surface_len(width, height) else {
        return false;
    };
    if !style.valid() || mask.len() != len || output.len() != len {
        return false;
    }
    let coverage: Vec<u8> = mask.iter().map(|pixel| (pixel & 255) as u8).collect();
    let radius = ((OUTLINE_DIP * dpi.clamp(96, 384) / 96) as usize).min(MAX_EFFECT_PIXELS);
    let shift = ((SHADOW_DIP * dpi.clamp(96, 384) / 96) as usize).min(MAX_EFFECT_PIXELS);
    // Two separable dilation passes bound work to O(pixels * radius), not radius squared.
    let mut horizontal = vec![0u8; len];
    if style.outline {
        for y in 0..height {
            for x in 0..width {
                horizontal[y * width + x] = coverage[y * width + x.saturating_sub(radius)
                    ..=y * width + (x + radius).min(width - 1)]
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(0);
            }
        }
    }
    let background = u32::from(style.background_percent) * 255 / 100;
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let outline = if style.outline {
                (y.saturating_sub(radius)..=(y + radius).min(height - 1))
                    .map(|row| u32::from(horizontal[row * width + x]))
                    .max()
                    .unwrap_or(0)
            } else {
                0
            };
            let shadow = if style.shadow && x >= shift && y >= shift {
                u32::from(coverage[(y - shift) * width + x - shift]) * SHADOW_ALPHA / 255
            } else {
                0
            };
            let glyph = u32::from(coverage[i]);
            let alpha = over(glyph, over(outline, over(shadow, background)));
            output[i] = (alpha << 24) | (glyph << 16) | (glyph << 8) | glyph;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transparent_background_preserves_opaque_glyphs_and_premultiplied_edges() {
        let mask = [0, 0x00ffffff, 0x00808080];
        let mut output = [0; 3];
        let style = SubtitleAppearance {
            background_percent: 0,
            outline: false,
            shadow: false,
            ..Default::default()
        };
        assert!(composite(&mask, &mut output, 3, 1, 96, style));
        assert_eq!(output, [0, 0xffffffff, 0x80808080]);
        assert!(composite(
            &mask,
            &mut output,
            3,
            1,
            96,
            SubtitleAppearance {
                background_percent: 100,
                ..style
            }
        ));
        assert_eq!(output, [0xff000000, 0xffffffff, 0xff808080]);
    }
    #[test]
    fn outline_and_shadow_are_visible_without_changing_glyph_color() {
        let mut mask = [0u32; 81];
        mask[4 * 9 + 4] = 0x00ffffff;
        let mut output = [0u32; 81];
        let style = SubtitleAppearance {
            background_percent: 0,
            ..Default::default()
        };
        assert!(composite(&mask, &mut output, 9, 9, 96, style));
        assert_eq!(output[4 * 9 + 4], 0xffffffff);
        assert_eq!(output[4 * 9 + 2], 0xff000000);
        assert_eq!(output[7 * 9 + 7], 160 << 24);
        assert_eq!(output[0], 0);
        for pixel in output {
            assert!((pixel & 255) <= (pixel >> 24));
        }
    }
    #[test]
    fn malformed_preferences_or_surfaces_are_rejected_before_allocation() {
        assert!(surface_len(0, 10).is_none());
        assert!(surface_len(4096, 2048).is_none());
        assert!(surface_len(usize::MAX, 2).is_none());
        assert!(!composite(
            &[0],
            &mut [0],
            2,
            1,
            96,
            SubtitleAppearance::default()
        ));
        for style in [
            SubtitleAppearance {
                max_lines: 0,
                ..Default::default()
            },
            SubtitleAppearance {
                background_percent: 101,
                ..Default::default()
            },
            SubtitleAppearance {
                font_size: 1000,
                ..Default::default()
            },
        ] {
            assert!(!composite(&[0], &mut [0], 1, 1, 96, style));
        }
    }
}
