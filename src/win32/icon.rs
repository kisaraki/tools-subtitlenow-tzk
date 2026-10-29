//! Best-effort icon extraction to bounded RAM pixels. Native resources never reach a view.

use windows::{
    Win32::{
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, HBITMAP, HDC, HGDIOBJ, SelectObject,
        },
        Storage::FileSystem::GetDriveTypeW,
        System::WindowsProgramming::{DRIVE_CDROM, DRIVE_FIXED, DRIVE_RAMDISK, DRIVE_REMOVABLE},
        UI::{
            Shell::{ExtractIconExW, PathParseIconLocationW},
            WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON},
        },
    },
    core::{PCWSTR, PWSTR},
};

use crate::audio::sessions::{ApplicationIcon, MAX_ICON_SIDE};

/// One small source-picker icon per retained session, independent of display DPI.
const ICON_SIZE: usize = MAX_ICON_SIDE as usize;
const ICON_BYTES: usize = ICON_SIZE * ICON_SIZE * 4;

struct OwnedIcon(HICON);
impl Drop for OwnedIcon {
    fn drop(&mut self) {
        // SAFETY: ExtractIconEx returns an owned icon; it is never a shared/class icon.
        let _ = unsafe { DestroyIcon(self.0) };
    }
}

struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
}

impl Canvas {
    fn new() -> Option<Self> {
        // SAFETY: creates a private memory DC, used only on this worker.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            return None;
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: ICON_SIZE as i32,
                biHeight: -(ICON_SIZE as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        // SAFETY: initialized bitmap descriptor and valid writable pointer; fixed 32x32 BGRA.
        let bitmap = match unsafe {
            CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        } {
            Ok(bitmap) => bitmap,
            Err(_) => {
                // SAFETY: dc is still owned and has no selected bitmap.
                let _ = unsafe { DeleteDC(dc) };
                return None;
            }
        };
        // SAFETY: live compatible memory DC and owned DIB; retain prior selection for teardown.
        let previous = unsafe { SelectObject(dc, bitmap.into()) };
        let canvas = Self {
            dc,
            bitmap,
            previous,
            bits: bits.cast(),
        };
        if previous.is_invalid() || bits.is_null() {
            return None;
        }
        Some(canvas)
    }

    fn draw(&mut self, icon: HICON, background: u8) -> Option<Vec<u8>> {
        // SAFETY: the DIB allocation is ICON_BYTES large, only this thread accesses it, and
        // the prior draw was flushed before returning. The bitmap remains selected and owned.
        unsafe { std::slice::from_raw_parts_mut(self.bits, ICON_BYTES) }.fill(background);
        // SAFETY: all handles live for the draw; dimensions fit the allocated bitmap.
        unsafe {
            DrawIconEx(
                self.dc,
                0,
                0,
                icon,
                ICON_SIZE as i32,
                ICON_SIZE as i32,
                0,
                None,
                DI_NORMAL,
            )
        }
        .ok()?;
        // SAFETY: flush this thread's GDI batch before reading bitmap memory on the CPU.
        if !unsafe { GdiFlush() }.as_bool() {
            return None;
        }
        // SAFETY: flushed, initialized DIB of exactly ICON_BYTES, copied before any next draw.
        Some(unsafe { std::slice::from_raw_parts(self.bits, ICON_BYTES) }.to_vec())
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        // SAFETY: restore the prior bitmap before deleting our bitmap and DC. No other owner.
        unsafe {
            if !self.previous.is_invalid() {
                SelectObject(self.dc, self.previous);
            }
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

pub(super) fn extract(location: &str) -> Option<ApplicationIcon> {
    let mut path: Vec<u16> = location.encode_utf16().chain(Some(0)).collect();
    // SAFETY: writable NUL-terminated buffer; parser shortens in place, never grows it.
    let index = unsafe { PathParseIconLocationW(PWSTR(path.as_mut_ptr())) };
    let end = path.iter().position(|unit| *unit == 0)?;
    let parsed = String::from_utf16(&path[..end]).ok()?;
    let parsed = std::path::Path::new(&parsed);
    // Session-provided icons must not trigger UNC/URL or network-drive file access.
    if !parsed.is_absolute() {
        return None;
    }
    let Some(std::path::Component::Prefix(prefix)) = parsed.components().next() else {
        return None;
    };
    let (std::path::Prefix::Disk(drive) | std::path::Prefix::VerbatimDisk(drive)) = prefix.kind()
    else {
        return None;
    };
    let root = [u16::from(drive), b':' as u16, b'\\' as u16, 0];
    // SAFETY: local drive root is a valid terminated UTF-16 buffer, queried without opening a file.
    if !matches!(
        unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) },
        DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_CDROM | DRIVE_RAMDISK
    ) {
        return None;
    }
    let mut handle = HICON::default();
    // SAFETY: input remains terminated, output points to one initialized slot, count is one.
    let count = unsafe { ExtractIconExW(PCWSTR(path.as_ptr()), index, Some(&mut handle), None, 1) };
    if handle.is_invalid() {
        return None;
    }
    let owned = OwnedIcon(handle);
    if count != 1 {
        return None;
    }
    let mut canvas = Canvas::new()?;
    let mut black = canvas.draw(owned.0, 0)?;
    let white = canvas.draw(owned.0, 255)?;
    // Recover alpha from two backgrounds, including legacy masked icons with no alpha plane.
    for (pixel, white_pixel) in black
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(white.as_chunks::<4>().0)
    {
        pixel[3] = 255 - white_pixel[0].saturating_sub(pixel[0]);
    }
    Some(ApplicationIcon {
        width: ICON_SIZE as u32,
        height: ICON_SIZE as u32,
        bgra: black,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn rejects_remote_relative_and_device_icon_locations() {
        for path in [
            r"\\server\share\icon.dll,1",
            "https://example.com/icon.ico",
            "relative.exe",
            r"\\.\pipe\icon",
        ] {
            assert!(super::extract(path).is_none());
        }
    }
}
