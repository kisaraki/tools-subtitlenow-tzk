//! RAM-only alpha surface. GDI resources never leave their owning UI thread.
use crate::{
    error::{AppError, Result},
    ui::appearance::{self, PADDING_DIP, SubtitleAppearance},
};
use std::{marker::PhantomData, rc::Rc};
use windows::{
    Win32::{
        Foundation::{COLORREF, HWND, POINT, RECT, SIZE},
        Graphics::Gdi::*,
        UI::{
            HiDpi::GetDpiForWindow,
            WindowsAndMessaging::{GetWindowRect, ULW_ALPHA, UpdateLayeredWindow},
        },
    },
    core::HSTRING,
};

struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    len: usize,
    _same_thread: PhantomData<Rc<()>>,
}
impl Surface {
    fn new(width: usize, height: usize) -> Result<Self> {
        let len = appearance::surface_len(width, height).ok_or(AppError::SubtitleWindow)?;
        // SAFETY: private memory DC; no screen capture or external device resource.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.0.is_null() {
            return Err(AppError::SubtitleWindow);
        }
        let mut surface = Self {
            dc,
            bitmap: HBITMAP::default(),
            old: HGDIOBJ::default(),
            bits: std::ptr::null_mut(),
            len,
            _same_thread: PhantomData,
        };
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // top-down BGRA, exactly width * 4 bytes per row
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: bounded dimensions, exact initialized header, valid out pointer. No
        // file mapping; Windows owns RAM until this unique bitmap is deleted by Drop.
        surface.bitmap =
            unsafe { CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) }
                .map_err(|_| AppError::SubtitleWindow)?;
        if bits.is_null() {
            return Err(AppError::SubtitleWindow);
        }
        surface.bits = bits.cast();
        // SAFETY: unique bitmap selected into unique memory DC, restored before deletion.
        surface.old = unsafe { SelectObject(dc, HGDIOBJ(surface.bitmap.0)) };
        if surface.old.0.is_null() || surface.old.0 as isize == -1 {
            surface.old = HGDIOBJ::default();
            return Err(AppError::SubtitleWindow);
        }
        surface.pixels_mut().fill(0);
        Ok(surface)
    }
    fn pixels(&self) -> &[u32] {
        // SAFETY: creation validates 32-bit aligned, live system allocation of len pixels.
        // Caller flushes GDI before reads; same-thread marker prevents concurrent access.
        unsafe { std::slice::from_raw_parts(self.bits, self.len) }
    }
    fn pixels_mut(&mut self) -> &mut [u32] {
        // SAFETY: exclusive access to this live allocation, no outstanding GDI writes.
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.len) }
    }
}
impl Drop for Surface {
    fn drop(&mut self) {
        // SAFETY: unique UI-thread DC/bitmap. Restore borrowed original object first,
        // delete only our bitmap, then DC. Partial construction follows the same order.
        unsafe {
            if !self.old.0.is_null() {
                SelectObject(self.dc, self.old);
            }
            if !self.bitmap.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            }
            let _ = DeleteDC(self.dc);
        }
    }
}
struct Font(HFONT);
impl Drop for Font {
    fn drop(&mut self) {
        // SAFETY: unique font, Selection below is always dropped before this owner.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.0.0));
        }
    }
}
struct Selection {
    dc: HDC,
    old: HGDIOBJ,
}
impl Drop for Selection {
    fn drop(&mut self) {
        // SAFETY: restores this same live DC's borrowed previous font.
        unsafe {
            SelectObject(self.dc, self.old);
        }
    }
}
fn rasterize(
    width: usize,
    height: usize,
    dpi: u32,
    text: &str,
    style: SubtitleAppearance,
) -> Result<Surface> {
    if !style.valid() {
        return Err(AppError::SubtitleWindow);
    }
    let mask = Surface::new(width, height)?;
    let face = HSTRING::from(style.font.face());
    // SAFETY: name survives the call, dimensions bounded by validated appearance and DPI.
    // Grayscale antialiasing avoids ClearType RGB fringes in per-pixel alpha composition.
    let font = Font(unsafe {
        CreateFontW(
            -super::subtitle_window::scale(i32::from(style.font_size), dpi),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY,
            0,
            &face,
        )
    });
    if font.0.0.is_null() {
        return Err(AppError::SubtitleWindow);
    }
    // SAFETY: font and DC are owned here; guard restores the font on every exit.
    let old = unsafe { SelectObject(mask.dc, HGDIOBJ(font.0.0)) };
    if old.0.is_null() || old.0 as isize == -1 {
        return Err(AppError::SubtitleWindow);
    }
    let _selection = Selection { dc: mask.dc, old };
    let mut metrics = TEXTMETRICW::default();
    // SAFETY: initialized output and live memory DC with chosen font.
    unsafe { GetTextMetricsW(mask.dc, &mut metrics) }
        .ok()
        .map_err(|_| AppError::SubtitleWindow)?;
    let pad = super::subtitle_window::scale(PADDING_DIP, dpi);
    let line_height = metrics.tmHeight.max(1);
    let available_lines =
        ((height as i32 - 2 * pad).max(0) / line_height).min(i32::from(style.max_lines));
    if available_lines > 0 && width as i32 > 2 * pad && !text.is_empty() {
        let mut rect = RECT {
            left: pad,
            top: pad,
            right: width as i32 - pad,
            bottom: pad + available_lines * line_height,
        };
        let mut utf16: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: bounded timeline text, live vector and memory DC, no MODIFYSTRING.
        // DrawText clips to whole measured lines and adds ellipsis to the last visible line.
        unsafe {
            SetBkMode(mask.dc, TRANSPARENT);
            SetTextColor(mask.dc, COLORREF(0x00ffffff));
            if DrawTextW(
                mask.dc,
                &mut utf16,
                &mut rect,
                DT_CENTER | DT_WORDBREAK | DT_NOPREFIX | DT_END_ELLIPSIS,
            ) == 0
            {
                return Err(AppError::SubtitleWindow);
            }
        }
    }
    // SAFETY: complete all GDI writes before direct DIB reads/composition on this thread.
    unsafe { GdiFlush() }
        .ok()
        .map_err(|_| AppError::SubtitleWindow)?;
    let mut output = Surface::new(width, height)?;
    if !appearance::composite(
        mask.pixels(),
        output.pixels_mut(),
        width,
        height,
        dpi,
        style,
    ) {
        return Err(AppError::SubtitleWindow);
    }
    Ok(output)
}
pub(super) fn draw_icon(
    dc: HDC,
    rect: RECT,
    icon: &crate::audio::sessions::ApplicationIcon,
) -> Result<()> {
    if !icon.valid() || rect.right <= rect.left || rect.bottom <= rect.top {
        return Err(AppError::SubtitleWindow);
    }
    let mut surface = Surface::new(icon.width as usize, icon.height as usize)?;
    for (pixel, bytes) in surface
        .pixels_mut()
        .iter_mut()
        .zip(icon.bgra.as_chunks::<4>().0.iter())
    {
        *pixel = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    }
    // SAFETY: borrowed owner-draw destination DC and bounded private BGRA source DIB.
    // Caller retains destination ownership, our RAII surface restores/deletes only its DC.
    unsafe {
        AlphaBlend(
            dc,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            surface.dc,
            0,
            0,
            icon.width as i32,
            icon.height as i32,
            BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            },
        )
    }
    .ok()
    .map_err(|_| AppError::SubtitleWindow)
}
pub(super) fn render(hwnd: HWND, text: &str, style: SubtitleAppearance) -> Result<()> {
    let mut rect = RECT::default();
    // SAFETY: shell owns this HWND on UI thread, initialized coordinate out parameter.
    unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|_| AppError::SubtitleWindow)?;
    let width = usize::try_from(rect.right - rect.left).map_err(|_| AppError::SubtitleWindow)?;
    let height = usize::try_from(rect.bottom - rect.top).map_err(|_| AppError::SubtitleWindow)?;
    // SAFETY: same-thread app-owned window; renderer owns all temporary resources.
    let dpi = unsafe { GetDpiForWindow(hwnd) }.clamp(96, 384);
    let surface = rasterize(width, height, dpi, text, style)?;
    let position = POINT {
        x: rect.left,
        y: rect.top,
    };
    let source = POINT::default();
    let size = SIZE {
        cx: width as i32,
        cy: height as i32,
    };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    // SAFETY: borrowed live DC/DIB and stack input structures survive synchronous update.
    // Only our alpha surface is sent to compositor; no desktop pixels are read or stored.
    unsafe {
        UpdateLayeredWindow(
            hwnd,
            None,
            Some(&position),
            Some(&size),
            Some(surface.dc),
            Some(&source),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        )
    }
    .map_err(|_| AppError::SubtitleWindow)
}

#[cfg(test)]
pub(super) fn inspect_test_canvas(
    width: usize,
    height: usize,
    draw: impl FnOnce(HDC) -> Result<()>,
) -> Result<Vec<u32>> {
    let surface = Surface::new(width, height)?;
    draw(surface.dc)?;
    // SAFETY: inspect only the synthetic test's own DIB after flushing native drawing.
    unsafe { GdiFlush() }
        .ok()
        .map_err(|_| AppError::SubtitleWindow)?;
    Ok(surface.pixels().to_vec())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_icon_alpha_blend_uses_bounded_bgra_and_preserves_outside_pixels() {
        let mut surface = Surface::new(20, 20).unwrap();
        surface.pixels_mut().fill(0xff00ff00);
        let icon = crate::audio::sessions::ApplicationIcon {
            width: 1,
            height: 1,
            bgra: vec![0, 0, 255, 255],
        };
        draw_icon(
            surface.dc,
            RECT {
                left: 2,
                top: 2,
                right: 18,
                bottom: 18,
            },
            &icon,
        )
        .unwrap();
        // SAFETY: flush writes to our own DIB before direct CPU inspection, no screen read.
        unsafe {
            GdiFlush().ok().unwrap();
        }
        assert_eq!(surface.pixels()[0], 0xff00ff00);
        assert_eq!(surface.pixels()[10 * 20 + 10], 0xffff0000);
        let mut invalid = icon;
        invalid.bgra = vec![0, 0, 255, 1];
        assert!(
            draw_icon(
                surface.dc,
                RECT {
                    left: 0,
                    top: 0,
                    right: 10,
                    bottom: 10
                },
                &invalid
            )
            .is_err()
        );
    }
    #[test]
    fn native_raster_uses_measured_line_limit_and_leaves_background_independent() {
        for dpi in [96, 120, 144, 168, 192] {
            let style = SubtitleAppearance {
                max_lines: 1,
                background_percent: 0,
                outline: false,
                shadow: false,
                ..Default::default()
            };
            let surface = rasterize(
                500,
                300,
                dpi,
                "第一行測試\nSECOND LINE MUST BE CLIPPED\n第三行",
                style,
            )
            .unwrap();
            assert!(surface.pixels().iter().any(|pixel| pixel >> 24 > 0));
            // One glyph line always fits above row 100 at these scales; lower lines absent.
            assert!(
                surface.pixels()[100 * 500..]
                    .iter()
                    .all(|pixel| *pixel == 0)
            );
            assert!(
                surface
                    .pixels()
                    .iter()
                    .all(|pixel| (pixel & 255) <= (pixel >> 24))
            );
        }
    }
    #[test]
    fn native_fonts_and_dpi_render_without_file_mapping_or_unbounded_surface() {
        for font in appearance::SubtitleFont::ALL {
            let style = SubtitleAppearance {
                font,
                ..Default::default()
            };
            let surface =
                rasterize(600, 200, 144, "繁體中文 English 日本語 한국어", style).unwrap();
            assert!(surface.pixels().iter().any(|pixel| pixel & 255 > 0));
            assert_eq!(surface.pixels()[0] >> 24, 70 * 255 / 100);
        }
        assert!(rasterize(usize::MAX, 1, 96, "", SubtitleAppearance::default()).is_err());
    }
}
