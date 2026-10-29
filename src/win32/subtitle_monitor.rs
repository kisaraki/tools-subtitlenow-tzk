//! Query-only display inventory. Names, handles and geometry are session preferences.
use crate::{
    error::{AppError, Result},
    ui::appearance::{self, SIDE_WIDTH_DIP, SideDock, SubtitleAppearance},
};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT},
        Graphics::Gdi::*,
        UI::WindowsAndMessaging::MONITORINFOF_PRIMARY,
    },
    core::BOOL,
};

pub(super) const MAX_MONITORS: usize = 32;
#[derive(Clone)]
pub(super) struct Monitor {
    pub name: String,
    pub work: RECT,
    pub primary: bool,
}
struct Inventory {
    monitors: Vec<Monitor>,
    failed: bool,
}
pub(super) fn enumerate() -> Result<Vec<Monitor>> {
    let mut inventory = Inventory {
        monitors: Vec::new(),
        failed: false,
    };
    // SAFETY: synchronous enumeration with a unique stack context. Callback never stores
    // its pointer or unwinds, and bounds the vector before accepting another monitor.
    let status = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(visit),
            LPARAM((&mut inventory as *mut Inventory) as isize),
        )
    };
    if !status.as_bool() || inventory.failed || inventory.monitors.is_empty() {
        return Err(AppError::SubtitleWindow);
    }
    inventory
        .monitors
        .sort_by(|a, b| b.primary.cmp(&a.primary).then(a.name.cmp(&b.name)));
    Ok(inventory.monitors)
}
unsafe extern "system" fn visit(monitor: HMONITOR, _: HDC, _: *mut RECT, context: LPARAM) -> BOOL {
    // SAFETY: context is the live exclusive Inventory above for this synchronous callback.
    let inventory = unsafe { &mut *(context.0 as *mut Inventory) };
    if inventory.monitors.len() >= MAX_MONITORS {
        inventory.failed = true;
        return false.into();
    }
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    // SAFETY: EXW starts with MONITORINFO; exact EXW size permits Windows to fill both
    // initialized fields and fixed device-name buffer. Handle is borrowed from enumeration.
    if !unsafe { GetMonitorInfoW(monitor, (&mut info as *mut MONITORINFOEXW).cast()) }.as_bool() {
        inventory.failed = true;
        return false.into();
    }
    let len = info
        .szDevice
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(info.szDevice.len());
    let name = String::from_utf16_lossy(&info.szDevice[..len]);
    if name.is_empty()
        || info.monitorInfo.rcWork.right <= info.monitorInfo.rcWork.left
        || info.monitorInfo.rcWork.bottom <= info.monitorInfo.rcWork.top
    {
        inventory.failed = true;
        return false.into();
    }
    inventory.monitors.push(Monitor {
        name,
        work: info.monitorInfo.rcWork,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    });
    true.into()
}
/// Bound the entire alpha surface within the selected monitor, including small work areas.
pub(super) fn placement(work: RECT, dpi: u32, style: SubtitleAppearance) -> RECT {
    let width = super::subtitle_window::scale(800, dpi)
        .min(work.right - work.left)
        .clamp(1, appearance::MAX_SURFACE_WIDTH as i32);
    let height_cap = (appearance::MAX_SURFACE_PIXELS / width as usize)
        .min(appearance::MAX_SURFACE_HEIGHT) as i32;
    let height = super::subtitle_window::scale(style.height_dip(), dpi)
        .min(work.bottom - work.top)
        .clamp(1, height_cap);
    let margin =
        super::subtitle_window::scale(24, dpi).min((work.bottom - work.top - height).max(0));
    let left = work.left + (work.right - work.left - width) / 2;
    let top = work.bottom - height - margin;
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}
pub(super) fn window_work_area(window: HWND) -> Result<RECT> {
    window_geometry(window).map(|(_, work)| work)
}
pub(super) fn window_geometry(window: HWND) -> Result<(RECT, RECT)> {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: query our own live window, initialized out structure, no borrowed ownership.
    unsafe {
        GetMonitorInfoW(
            MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST),
            &mut info,
        )
    }
    .ok()
    .map_err(|_| AppError::SubtitleWindow)?;
    Ok((info.rcMonitor, info.rcWork))
}
/// Physical screen proportions; do not scale monitor pixels a second time for DPI.
pub(super) fn initial_side_placement(screen: RECT, work: RECT) -> RECT {
    let height = ((screen.bottom - screen.top) / 2).clamp(1, (work.bottom - work.top).max(1));
    let width = (height / 3).clamp(1, (work.right - work.left).max(1));
    RECT {
        left: work.right - width,
        top: work.top,
        right: work.right,
        bottom: work.top + height,
    }
}
pub(super) fn side_placement(work: RECT, dpi: u32, side: SideDock) -> RECT {
    let width = super::subtitle_window::scale(SIDE_WIDTH_DIP, dpi)
        .min(work.right - work.left)
        .max(1);
    let left = if side == SideDock::Left {
        work.left
    } else {
        work.right - width
    };
    RECT {
        left,
        top: work.top,
        right: left + width,
        bottom: work.bottom,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn phase11_initial_floating_geometry_uses_half_screen_height_and_one_third_width() {
        for (left, top, width, height) in [
            (0, 0, 1920, 1080),
            (-2560, -100, 2560, 1440),
            (0, 0, 3840, 2160),
        ] {
            let screen = RECT {
                left,
                top,
                right: left + width,
                bottom: top + height,
            };
            let work = RECT {
                bottom: screen.bottom - 48,
                ..screen
            };
            let rect = initial_side_placement(screen, work);
            assert_eq!(rect.bottom - rect.top, height / 2);
            assert_eq!(rect.right - rect.left, height / 6);
            assert_eq!(rect.right, work.right);
            assert_eq!(rect.top, work.top);
        }
    }
    #[test]
    fn left_right_side_geometry_fits_five_dpi_levels_and_negative_work_areas() {
        for work in [
            RECT {
                left: -1920,
                top: -200,
                right: 0,
                bottom: 880,
            },
            RECT {
                left: 0,
                top: 0,
                right: 300,
                bottom: 600,
            },
        ] {
            for dpi in [96, 120, 144, 168, 192] {
                let left = side_placement(work, dpi, SideDock::Left);
                let right = side_placement(work, dpi, SideDock::Right);
                assert_eq!(left.left, work.left);
                assert_eq!(right.right, work.right);
                assert!(left.right <= work.right && right.left >= work.left);
                assert_eq!(left.top, work.top);
                assert_eq!(right.bottom, work.bottom);
            }
        }
    }
    #[test]
    fn placement_fits_negative_origins_small_work_areas_and_five_dpi_levels() {
        for work in [
            RECT {
                left: -1920,
                top: -100,
                right: 0,
                bottom: 980,
            },
            RECT {
                left: 20,
                top: 20,
                right: 180,
                bottom: 100,
            },
            RECT {
                left: 0,
                top: 0,
                right: 16000,
                bottom: 9000,
            },
        ] {
            for dpi in [96, 120, 144, 168, 192] {
                for max_lines in [1, 3, 6] {
                    let style = SubtitleAppearance {
                        font_size: 72,
                        max_lines,
                        ..Default::default()
                    };
                    let rect = placement(work, dpi, style);
                    assert!(rect.left >= work.left && rect.right <= work.right);
                    assert!(rect.top >= work.top && rect.bottom <= work.bottom);
                    assert!(
                        appearance::surface_len(
                            (rect.right - rect.left) as usize,
                            (rect.bottom - rect.top) as usize
                        )
                        .is_some()
                    );
                }
            }
        }
    }
    #[test]
    fn native_inventory_has_bounded_distinct_device_names_and_primary_display() {
        let monitors = enumerate().unwrap();
        assert!(monitors.len() <= MAX_MONITORS);
        assert!(monitors[0].primary);
        for (index, monitor) in monitors.iter().enumerate() {
            assert!(
                monitors[index + 1..]
                    .iter()
                    .all(|other| other.name != monitor.name)
            );
        }
    }
}
