use windows::Win32::Foundation::HWND;

pub fn get_now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

pub fn is_transient_windows_io_error(err: &std::io::Error) -> bool {
    err.kind() == std::io::ErrorKind::PermissionDenied
        || matches!(err.raw_os_error(), Some(5 | 32 | 33))
}

pub fn retry_io<T, F>(mut op: F) -> std::io::Result<T>
where
    F: FnMut() -> std::io::Result<T>,
{
    const DELAYS_MS: [u64; 5] = [15, 30, 60, 120, 200];
    let mut attempt = 0usize;
    loop {
        match op() {
            Ok(val) => return Ok(val),
            Err(err) if attempt < DELAYS_MS.len() && is_transient_windows_io_error(&err) => {
                std::thread::sleep(std::time::Duration::from_millis(DELAYS_MS[attempt]));
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

/// Makes a window click-through or not by its WS_EX_TRANSPARENT bit alone, keeping WS_EX_LAYERED set
/// in both states.
///
/// tao's `set_ignore_cursor_events` sets both bits when ignoring and clears both otherwise, and a
/// window with a region (window_region.rs) that isn't layered paints its clear pixels opaque. So tao's
/// call is only used once, on window creation: its flag then stays on, and if tao rewrites the extended
/// style (it does on show and hide) the window ends up click-through and layered, which is the safe
/// state, until the style guard restores the real one. Writes only when the style differs.
pub fn set_click_through(hwnd: HWND, ignore: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE};
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let wanted = click_through_style(current, ignore);
        if wanted != current {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted as isize);
        }
    }
}

pub fn set_window_click_through(window: &tauri::WebviewWindow, ignore: bool) {
    if let Ok(hwnd) = window.hwnd() {
        set_click_through(hwnd, ignore);
    }
}

fn click_through_style(style: u32, ignore: bool) -> u32 {
    use windows::Win32::UI::WindowsAndMessaging::{WS_EX_LAYERED, WS_EX_TRANSPARENT};
    let layered = style | WS_EX_LAYERED.0;
    if ignore { layered | WS_EX_TRANSPARENT.0 } else { layered & !WS_EX_TRANSPARENT.0 }
}

/// Re-assert HWND_TOPMOST without activating the window.
///
/// Tauri's `set_always_on_top(true)` calls `SetWindowPos(HWND_TOPMOST)` without
/// `SWP_NOACTIVATE`, which causes Windows to send `WM_ACTIVATE` to the WebView2 window.
/// This activation message makes the WebView compositor briefly blank/hide the window,
/// and can also strip the `WS_EX_NOACTIVATE` extended style.
///
/// This helper uses the raw Win32 call with the correct flags and re-stamps
/// `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` to prevent both problems.
pub fn re_assert_topmost(hwnd: HWND) {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, GetWindowLongPtrW, SetWindowLongPtrW,
            HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SWP_NOACTIVATE, SWP_NOSENDCHANGING, SWP_ASYNCWINDOWPOS,
            GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        };
        // Set topmost without activating or notifying the window
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_ASYNCWINDOWPOS,
        );
        // Re-stamp NOACTIVATE + TOOLWINDOW — HWND_TOPMOST can cause these to be reset
        // by the shell on some Windows builds
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as usize;
        let keep_focusable = crate::state::KEYBOARD_FOCUS_LABEL.lock().ok().is_some_and(|l| l.is_some());
        let noactivate = if keep_focusable { 0 } else { WS_EX_NOACTIVATE.0 as usize };
        let new_ex = ex | noactivate | WS_EX_TOOLWINDOW.0 as usize;
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_ex as isize);
    }
}

/// True when a maximized window still has standard window chrome (caption or resize frame),
/// distinguishing normal maximized apps (including custom-chrome browsers) from borderless
/// fullscreen windows that happen to retain `WS_MAXIMIZE` / `IsZoomed`.
pub fn is_captioned_maximized(is_maximized: bool, style: u32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{WS_CAPTION, WS_THICKFRAME};
    is_maximized && ((style & WS_CAPTION.0) == WS_CAPTION.0 || (style & WS_THICKFRAME.0) != 0)
}

/// True for real fullscreen: client area covers the monitor and it isn't a captioned maximized window.
pub fn is_window_fullscreen(hwnd: HWND) -> bool {
    unsafe {
        use windows::Win32::Foundation::{POINT, RECT};
        use windows::Win32::Graphics::Gdi::{ClientToScreen, GetMonitorInfoA, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
        use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowLongW, IsZoomed, GWL_STYLE, WS_MAXIMIZE};

        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        if monitor.is_invalid() { return false; }
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoA(monitor, &mut info).as_bool() { return false; }
        let screen = info.rcMonitor;

        let mut client = RECT::default();
        if GetClientRect(hwnd, &mut client).is_err() { return false; }
        let mut top_left = POINT { x: client.left, y: client.top };
        let mut bottom_right = POINT { x: client.right, y: client.bottom };
        let _ = ClientToScreen(hwnd, &mut top_left);
        let _ = ClientToScreen(hwnd, &mut bottom_right);

        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        let is_maximized = IsZoomed(hwnd).as_bool() || (style & WS_MAXIMIZE.0) != 0;

        // Tolerance for subpixel DPI rounding (2px)
        let client_fullscreen = top_left.x <= screen.left + 2 && top_left.y <= screen.top + 2
            && bottom_right.x >= screen.right - 2 && bottom_right.y >= screen.bottom - 2;

        let is_d3d_fs = windows::Win32::UI::Shell::SHQueryUserNotificationState()
            .is_ok_and(|state| state == windows::Win32::UI::Shell::QUNS_RUNNING_D3D_FULL_SCREEN);

        (client_fullscreen && !is_captioned_maximized(is_maximized, style)) || is_d3d_fs
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn captioned_maximized_excludes_borderless_fullscreen_retaining_maximize() {
        use super::is_captioned_maximized;
        use windows::Win32::UI::WindowsAndMessaging::{WS_CAPTION, WS_MAXIMIZE, WS_POPUP, WS_THICKFRAME};

        // Normal maximized window (including custom-chrome Chromium with WS_CAPTION | WS_THICKFRAME).
        assert!(is_captioned_maximized(true, WS_MAXIMIZE.0 | WS_CAPTION.0 | WS_THICKFRAME.0));
        assert!(is_captioned_maximized(true, WS_MAXIMIZE.0 | WS_CAPTION.0));
        assert!(is_captioned_maximized(true, WS_MAXIMIZE.0 | WS_THICKFRAME.0));

        // Borderless fullscreen window that still has WS_MAXIMIZE / IsZoomed set.
        assert!(!is_captioned_maximized(true, WS_MAXIMIZE.0 | WS_POPUP.0));
        assert!(!is_captioned_maximized(true, WS_MAXIMIZE.0));

        // Non-maximized windows are never captioned-maximized.
        assert!(!is_captioned_maximized(false, WS_CAPTION.0 | WS_THICKFRAME.0));
    }
}
