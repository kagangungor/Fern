use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::Manager;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, WS_EX_TOOLWINDOW};
use crate::types::*;
use crate::state::*;
use crate::utils::*;

/// Computes the vertical top coordinate for a notch window on a monitor.
/// When `fern-notch-respect-workarea` is disabled (default), returns `monitor_pos_y` unchanged.
/// When enabled, returns `rcWork.top.max(monitor_pos_y)` so the notch sits below top status bars
/// such as YASB or Zebar instead of overlapping them.
pub fn notch_target_top(monitor_pos_x: i32, monitor_pos_y: i32, monitor_w: i32, monitor_h: i32) -> i32 {
    if !crate::utils::setting_flag("fern-notch-respect-workarea") {
        return monitor_pos_y;
    }
    unsafe {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        let pt = POINT {
            x: monitor_pos_x + (monitor_w / 2),
            y: monitor_pos_y + (monitor_h / 2),
        };
        let h_monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(h_monitor, &mut mi).as_bool() {
            return mi.rcWork.top.max(monitor_pos_y);
        }
    }
    monitor_pos_y
}

/// Ensures a notch window has `WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, is positioned at
/// `(m_pos.x, target_top, m_size.width, ph)` without activating it, and updates `main_window_rects`.
/// Returns `true` if a matching monitor was found and positioning was applied/verified.
pub fn position_notch_window(window: &tauri::WebviewWindow) -> bool {
    let label = window.label().to_string();
    let Some(monitor) = crate::monitors::monitor_for_window_label(
        window.app_handle(),
        crate::types::WindowKind::Notch,
        &label,
    ) else {
        return false;
    };
    let Ok(hwnd) = window.hwnd() else {
        return false;
    };
    let m_size = monitor.size();
    let m_pos = monitor.position();
    let scale = monitor.scale_factor();
    let fern_scale = crate::utils::get_fern_scale(window.app_handle());
    let ph = ((640.0 * fern_scale) * scale) as i32;
    let target_left = m_pos.x;
    let target_top = notch_target_top(m_pos.x, m_pos.y, m_size.width as i32, m_size.height as i32);
    let target_width = m_size.width as i32;

    unsafe {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE,
            SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, WS_EX_NOACTIVATE as WS_EX_NA,
            WS_EX_TOOLWINDOW,
        };

        let mut ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as usize;
        ex_style |= (WS_EX_TOOLWINDOW.0 | WS_EX_NA.0) as usize;
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);

        let mut current_rect = RECT::default();
        let mut already_positioned = false;
        if GetWindowRect(hwnd, &mut current_rect).is_ok() {
            let current_width = current_rect.right - current_rect.left;
            let current_height = current_rect.bottom - current_rect.top;
            if current_rect.left == target_left
                && current_rect.top == target_top
                && current_width == target_width
                && current_height == ph
            {
                already_positioned = true;
            }
        }

        if !already_positioned {
            let _ = SetWindowPos(
                hwnd,
                None,
                target_left,
                target_top,
                target_width,
                ph,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }

    if let Ok(mut map) = crate::state::main_window_rects().lock() {
        map.insert(
            label,
            (
                tauri::PhysicalPosition::new(target_left, target_top),
                tauri::PhysicalSize::new(target_width as u32, ph.max(0) as u32),
            ),
        );
    }

    true
}

pub fn register_appbar(window: tauri::WebviewWindow) {
    let label = window.label().to_string();
    if label != "main" && label != "dock" && !crate::state::SPLASH_FINISHED.load(Ordering::Relaxed) {
        return;
    }
    if position_notch_window(&window) {
        crate::state::set_flag(crate::state::main_appbar_registered(), &label, true);
        if let Ok(hwnd) = window.hwnd() {
            // Re-assert topmost after repositioning — use re_assert_topmost instead of
            // set_always_on_top(true) to include SWP_NOACTIVATE and re-stamp WS_EX_NOACTIVATE.
            // to blank/hide when other windows were minimized or closed.
            re_assert_topmost(hwnd);
        }

        if !window.is_visible().unwrap_or(false) {
            let _ = window.show();
        }
    } else {
        let w = window.clone();
        tauri::async_runtime::spawn(async move {
            for _ in 0..10 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if crate::monitors::monitor_for_window_label(w.app_handle(), crate::types::WindowKind::Notch, w.label()).is_some() {
                    register_appbar(w);
                    break;
                }
            }
        });
    }
}

pub fn register_dock_appbar(window: tauri::WebviewWindow) {
    register_dock_appbar_inner(window, 0);
}

fn register_dock_appbar_inner(window: tauri::WebviewWindow, attempt: i32) {
    let label = window.label().to_string();
    if label != "main" && label != "dock" && !crate::state::SPLASH_FINISHED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(monitor) = crate::monitors::monitor_for_window_label(window.app_handle(), crate::types::WindowKind::Dock, &label) {
        let m_size = monitor.size();
        let m_pos = monitor.position();
        let hwnd = window.hwnd().unwrap();
        let scale = monitor.scale_factor();
        let fern_scale = crate::utils::get_fern_scale(window.app_handle());
        
        // ph = full physical window height. outer_size() can return 0 before the
        // window has rendered. Never guess a value — bail and let the retry wrapper handle it.
        let ph = window.outer_size().map(|s| s.height as i32).unwrap_or(0);
        if ph <= 0 {
            if attempt < 10 {
                let w = window.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    register_dock_appbar_inner(w, attempt + 1);
                });
            }
            return;
        }

        let pr = ((56.0 * fern_scale) * scale) as i32;
        let reserve_space = crate::utils::setting_flag_default_true("fern-dock-reserve-space");

        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, GetWindowRect, SWP_NOZORDER, SWP_NOACTIVATE, GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TOOLWINDOW, WS_EX_NOACTIVATE as WS_EX_NA, SWP_FRAMECHANGED};
            use windows::Win32::Foundation::RECT;
            use windows::Win32::UI::Shell::{SHAppBarMessage, APPBARDATA, ABM_NEW, ABM_QUERYPOS, ABM_SETPOS, ABE_BOTTOM};
            
            // Set extended styles (ToolWindow and NoActivate)
            let mut ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as usize;
            ex_style |= (WS_EX_TOOLWINDOW.0 | WS_EX_NA.0) as usize;
            let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);

            // Hide native taskbar first to free up space
            set_taskbar_visibility(false, false);

            let (final_left, final_y, final_width) = if reserve_space {
                let mut abd = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: hwnd, ..Default::default() };

                if !crate::state::get_flag(crate::state::dock_appbar_registered(), &label) {
                    SHAppBarMessage(ABM_NEW, &mut abd);
                    crate::state::set_flag(crate::state::dock_appbar_registered(), &label, true);
                }

                abd.uEdge = ABE_BOTTOM;
                abd.rc = RECT { 
                    left: m_pos.x, 
                    top: m_pos.y + m_size.height as i32 - pr, 
                    right: m_pos.x + m_size.width as i32, 
                    bottom: m_pos.y + m_size.height as i32 
                };
                
                SHAppBarMessage(ABM_QUERYPOS, &mut abd);
                SHAppBarMessage(ABM_SETPOS, &mut abd);
                
                // Critical: Force the window to the actual bottom of the screen, 
                // ignoring what ABM_SETPOS might have tried to "correct" (like stacking on invisible taskbar)
                (abd.rc.left, m_pos.y + m_size.height as i32 - ph, abd.rc.right - abd.rc.left)
            } else {
                if crate::state::get_flag(crate::state::dock_appbar_registered(), &label) {
                    unregister_appbar_native(hwnd);
                    crate::state::set_flag(crate::state::dock_appbar_registered(), &label, false);
                }
                (m_pos.x, m_pos.y + m_size.height as i32 - ph, m_size.width as i32)
            };
            
            let mut current_rect = RECT::default();
            let mut already_positioned = false;
            if GetWindowRect(hwnd, &mut current_rect).is_ok() {
                let current_width = current_rect.right - current_rect.left;
                let current_height = current_rect.bottom - current_rect.top;
                if current_rect.left == final_left 
                    && current_rect.top == final_y 
                    && current_width == final_width 
                    && current_height == ph 
                {
                    already_positioned = true;
                }
            }
            
            if !already_positioned {
                let _ = SetWindowPos(hwnd, None, final_left, final_y, final_width, ph, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
            }

            // Re-assert topmost — same reason as register_appbar.
            re_assert_topmost(hwnd);

            // Always ensure the window is visible — show() is idempotent
            if !window.is_visible().unwrap_or(false) {
                let _ = window.show();
            }
        }
    } else {
        let w = window.clone();
        tauri::async_runtime::spawn(async move {
            for _ in 0..10 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if crate::monitors::monitor_for_window_label(w.app_handle(), crate::types::WindowKind::Dock, w.label()).is_some() {
                    register_dock_appbar(w);
                    break;
                }
            }
        });
    }
}

pub fn unregister_appbar_native(hwnd: HWND) {
    unsafe {
        use windows::Win32::UI::Shell::{SHAppBarMessage, APPBARDATA, ABM_REMOVE};
        let mut abd = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: hwnd, ..Default::default() };
        SHAppBarMessage(ABM_REMOVE, &mut abd);
    }
}
