//! Clips a notch, dock or overlay window to the parts of the screen its UI actually draws on.
//!
//! These windows are large transparent rectangles. Wherever such a window catches input it swallows
//! clicks meant for what's underneath, even where it draws nothing. A window region limits both
//! drawing and hit testing to the given rectangles, so outside them the window doesn't exist for
//! input at all, whatever its click-through state. Each webview reports the rectangles it needs, in
//! its own CSS pixels, whenever they change. Adapted from Bloom (GPL-3.0).

use tauri::Window;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, SetWindowRgn, RGN_OR};

use crate::types::IntRect;

/// The rectangles in physical window pixels: scaled by the window's DPI scale, rounded outwards so
/// nothing drawn at a fractional edge is clipped, and with empty ones dropped.
fn physical_rects(rects: &[IntRect], scale: f64) -> Vec<RECT> {
    rects
        .iter()
        .filter(|r| r.width > 0 && r.height > 0)
        .map(|r| RECT {
            left: (r.x as f64 * scale).floor() as i32,
            top: (r.y as f64 * scale).floor() as i32,
            right: ((r.x + r.width) as f64 * scale).ceil() as i32,
            bottom: ((r.y + r.height) as f64 * scale).ceil() as i32,
        })
        .collect()
}

fn takes_regions(label: &str) -> bool {
    crate::state::is_notch_label(label) || crate::state::is_dock_label(label) || label == "overlay"
}

/// Sets the calling window's region to the union of `rects`.
#[tauri::command]
pub fn set_window_regions(window: Window, rects: Vec<IntRect>) {
    if !takes_regions(window.label()) {
        return;
    }
    let physical = physical_rects(&rects, window.scale_factor().unwrap_or(1.0));
    // An empty region would make the window vanish until the next call, with nothing to bring it back
    // if that call never comes. The frontend always has something to draw, so this is a bad request.
    if physical.is_empty() {
        return;
    }
    let Ok(hwnd) = window.hwnd() else { return };
    // A region on a window that isn't layered turns its transparent pixels opaque, so make sure it
    // is layered before the region lands, keeping the click-through state it has.
    let click_through = unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TRANSPARENT};
        GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT.0 != 0
    };
    crate::utils::set_click_through(hwnd, click_through);
    unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        for rect in physical {
            let part = CreateRectRgn(rect.left, rect.top, rect.right, rect.bottom);
            CombineRgn(Some(region), Some(region), Some(part), RGN_OR);
            let _ = DeleteObject(part.into());
        }
        // The system owns the region from here on, so it isn't deleted on success.
        if SetWindowRgn(hwnd, Some(region), true) == 0 {
            let _ = DeleteObject(region.into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> IntRect {
        IntRect { x, y, width, height }
    }

    #[test]
    fn scales_and_rounds_outwards() {
        let scaled = physical_rects(&[rect(10, 0, 101, 45)], 1.25);
        assert_eq!(scaled, vec![RECT { left: 12, top: 0, right: 139, bottom: 57 }]);
    }

    #[test]
    fn only_the_large_transparent_windows_take_regions() {
        assert!(takes_regions("main"));
        assert!(takes_regions("dock"));
        assert!(takes_regions("overlay"));
        assert!(!takes_regions("settings"));
        assert!(!takes_regions("calendar"));
    }

    #[test]
    fn drops_empty_rectangles() {
        assert!(physical_rects(&[rect(0, -100, 140, 0), rect(5, 5, 0, 10)], 2.0).is_empty());
    }
}
