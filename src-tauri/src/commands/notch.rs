use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, Window};
use windows::Win32::Foundation::HWND;

use crate::services::{register_appbar, unregister_appbar_native};
use crate::state::*;
use crate::types::IntRect;
use crate::utils::*;

use super::windows::send_win_chord;

#[tauri::command]
pub async fn set_menu_open(open: bool, rect: Option<IntRect>) {
    MENU_IS_OPEN.store(open, Ordering::Relaxed);
    if let Ok(mut r) = MENU_RECT.lock() {
        *r = rect;
    }
}

#[tauri::command]
pub fn set_window_keyboard_focus(window: Window, enabled: bool) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowLongPtrW, SetForegroundWindow, SetWindowLongPtrW,
        SwitchToThisWindow, GWL_EXSTYLE, WS_EX_NOACTIVATE,
    };
    if let Ok(mut label) = crate::state::KEYBOARD_FOCUS_LABEL.lock() {
        if enabled {
            *label = Some(window.label().to_string());
        } else if label.as_deref() == Some(window.label()) {
            *label = None;
        }
    }
    let Ok(hwnd) = window.hwnd() else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as usize;
        let noactivate = WS_EX_NOACTIVATE.0 as usize;
        if enabled {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex & !noactivate) as isize);
            let _ = SetForegroundWindow(hwnd);
            let _ = window.set_focus();
            if GetForegroundWindow() != hwnd {
                SwitchToThisWindow(hwnd, true);
            }
        } else {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex | noactivate) as isize);
        }
    }
}

#[tauri::command]
pub async fn set_notch_hovered(window: Window, hovered: bool) {
    crate::state::set_flag(crate::state::notch_hovered(), window.label(), hovered);
}

#[tauri::command]
pub async fn set_notch_hidden(window: Window, hidden: bool) {
    crate::state::set_flag(crate::state::notch_hidden(), window.label(), hidden);
}

#[tauri::command]
pub async fn update_notch_rect(window: Window, rect: IntRect) {
    if let Ok(mut m) = crate::state::notch_rects().lock() {
        m.insert(window.label().to_string(), rect);
    }
}

#[tauri::command]
pub fn set_window_height(window: Window, height: f64) {
    if let Ok(scale_factor) = window.scale_factor() {
        if let Ok(physical_size) = window.inner_size() {
            let logical_width = physical_size.width as f64 / scale_factor;
            let _ = window.set_size(tauri::LogicalSize::new(logical_width, height));
        }
    }
}

#[tauri::command]
pub fn resize_settings_window(app: AppHandle, width: f64, height: f64) {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.set_size(tauri::LogicalSize::new(width, height));
        let _ = win.center();
    }
}

#[tauri::command]
pub fn set_ignore_cursor_events(window: Window, ignore: bool) {
    if let Ok(hwnd) = window.hwnd() {
        crate::utils::set_click_through(hwnd, ignore);
    }
}

pub(crate) async fn wait_for_splash_with(
    flag: &std::sync::atomic::AtomicBool,
    timeout: std::time::Duration,
) -> bool {
    let waited = std::time::Instant::now();
    while !flag.load(Ordering::Relaxed) {
        let elapsed = waited.elapsed();
        if elapsed >= timeout {
            return false;
        }
        let step = (timeout - elapsed).min(std::time::Duration::from_millis(100));
        tokio::time::sleep(step).await;
    }
    true
}

pub async fn wait_for_splash() {
    let _ = wait_for_splash_with(&crate::state::SPLASH_FINISHED, std::time::Duration::from_secs(8)).await;
}

#[tauri::command]
pub async fn change_notch_mode(app: AppHandle, mode: String) {
    wait_for_splash().await;
    let per_monitor = crate::monitors::is_per_monitor_active(&app, crate::types::WindowKind::Notch);
    let labels: Vec<String> = app.webview_windows().keys()
        .filter(|l| crate::state::is_notch_label(l))
        .cloned()
        .collect();
    for label in labels {
        let effective = if per_monitor {
            crate::monitors::resolve_mode_for_label(&app, crate::types::WindowKind::Notch, &label)
        } else {
            mode.clone()
        };
        if let Some(main_win) = app.get_webview_window(&label) {
            init_notch_window(&app, main_win, &effective).await;
        }
    }
}

pub async fn init_notch_window(app: &AppHandle, main_win: tauri::WebviewWindow, mode: &str) {
    let label = main_win.label().to_string();
    if mode == "fixed" {
        register_appbar(main_win.clone());
    } else {
        let _ = main_win.show();
        if let Ok(hwnd) = main_win.hwnd() {
            let hwnd_val = hwnd.0 as isize;
            tauri::async_runtime::spawn_blocking(move || {
                unregister_appbar_native(HWND(hwnd_val as *mut _));
            });
            crate::state::set_flag(crate::state::main_appbar_registered(), &label, false);

            let main_clone = main_win.clone();
            let label_clone = label.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                if !crate::state::get_flag(crate::state::main_appbar_registered(), &label_clone) {
                    if let Ok(hwnd) = main_clone.hwnd() { re_assert_topmost(hwnd); }
                }
            });
        }
    }
    if let Some(monitor) = crate::monitors::monitor_for_window_label(app, crate::types::WindowKind::Notch, &label) {
        let m_pos = monitor.position();
        let m_size = monitor.size();
        let scale = monitor.scale_factor();
        let fern_scale = crate::utils::get_fern_scale(app);
        let target_height = (640.0 * fern_scale * scale) as u32;
        let target_top = crate::services::notch_target_top(m_pos.x, m_pos.y, m_size.width as i32, m_size.height as i32);
        let _ = main_win.set_position(tauri::PhysicalPosition::new(m_pos.x, target_top));
        let _ = main_win.set_size(tauri::PhysicalSize::new(m_size.width, target_height));
    }

    let current = crate::state::get_overlap(crate::state::notch_overlap(), &label);
    if current != -1 {
        let _ = app.emit_to(label.as_str(), "notch-overlap", current == 1);
    }
}

#[tauri::command]
pub fn hide_native_osd() {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowA, ShowWindow, SW_HIDE};
        let class1 = windows::core::PCSTR(c"NativeHWNDHost".as_ptr() as *const u8);
        if let Ok(hwnd) = FindWindowA(class1, windows::core::PCSTR::null()) { let _ = ShowWindow(hwnd, SW_HIDE); }
    }
}

pub(crate) fn bring_window_to_front(app: &AppHandle, label: &str) {
    if let Some(win) = app.get_webview_window(label) {
        if label == "settings" || label == "calendar" {
            let _ = win.center();
        }
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        if let Ok(hwnd) = win.hwnd() {
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW};
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle, page: Option<String>) {
    match page {
        Some(page) => crate::on_demand_windows::open_settings_page(&app, page),
        None => crate::on_demand_windows::open(&app, crate::on_demand_windows::SETTINGS),
    }
}

// Async so it runs off the main thread: this destroys the webview that is making the call.
#[tauri::command]
pub async fn close_settings_window(app: AppHandle) {
    crate::on_demand_windows::close(&app, crate::on_demand_windows::SETTINGS);
}

#[tauri::command]
pub fn take_settings_page() -> Option<String> {
    crate::on_demand_windows::take_settings_page()
}

#[tauri::command]
pub fn open_calendar_window(app: AppHandle) {
    crate::on_demand_windows::open(&app, crate::on_demand_windows::CALENDAR);
}

// Async for the same reason as close_settings_window.
#[tauri::command]
pub async fn close_calendar_window(app: AppHandle) {
    crate::on_demand_windows::close(&app, crate::on_demand_windows::CALENDAR);
}

#[tauri::command]
pub fn open_tray_at_cursor(app: AppHandle) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut cursor = POINT::default();
    let anchor = unsafe { GetCursorPos(&mut cursor) }.ok().map(|_| (cursor.x, cursor.y - 4));
    std::thread::spawn(move || crate::tray_icons::open_native_tray(Some(app), anchor));
}

#[tauri::command]
pub fn show_desktop() {
    std::thread::spawn(|| send_win_chord(windows::Win32::UI::Input::KeyboardAndMouse::VK_D));
}

#[tauri::command]
pub fn open_tray_from_button(app: AppHandle, dx: f64, dy: f64) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut cursor = POINT::default();
    let anchor = unsafe { GetCursorPos(&mut cursor) }.ok().map(|_| (cursor.x + dx.round() as i32, cursor.y + dy.round() as i32 - 8));
    std::thread::spawn(move || crate::tray_icons::open_native_tray(Some(app), anchor));
}

#[tauri::command]
pub fn open_tray_window(app: AppHandle) {
    let anchor = notch_anchor(&app);
    std::thread::spawn(move || crate::tray_icons::open_native_tray(Some(app), anchor));
}

fn notch_anchor(app: &AppHandle) -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor).ok()?; }

    let windows = crate::state::main_window_rects().lock().ok()?.clone();
    let label = windows.iter()
        .find(|(_, (pos, size))| {
            cursor.x >= pos.x && cursor.x <= pos.x + size.width as i32 && cursor.y >= pos.y && cursor.y <= pos.y + size.height as i32
        })
        .map(|(label, _)| label.clone())
        .unwrap_or_else(|| "main".to_string());
    let (pos, _) = windows.get(&label).copied()?;
    let scale = app.get_webview_window(&label)?.scale_factor().ok()?;
    let rect = crate::state::notch_rects().lock().ok()?.get(&label).copied()?;

    let centre_x = pos.x + ((rect.x as f64 + rect.width as f64 / 2.0) * scale) as i32;
    let bottom = pos.y + ((rect.y as f64 + rect.height as f64) * scale) as i32;
    Some((centre_x, bottom))
}

#[tauri::command]
pub fn hide_overlay(app: AppHandle) {
    if let Some(win) = app.get_webview_window("overlay") { let _ = win.hide(); }
}

#[tauri::command]
pub fn set_splash_fullscreen(app: AppHandle, fullscreen: bool) {
    let already_fullscreen = crate::state::OVERLAY_IN_SPLASH.swap(fullscreen, Ordering::Relaxed);
    if let Some(win) = app.get_webview_window("overlay") {
        if fullscreen {
            if already_fullscreen && win.is_visible().unwrap_or(false) {
                return;
            }
            let _ = win.hide();
            if let Ok(Some(monitor)) = win.primary_monitor() {
                let size = monitor.size();
                let pos = monitor.position();
                let _ = win.set_position(tauri::PhysicalPosition::new(pos.x, pos.y));
                let _ = win.set_size(tauri::PhysicalSize::new(size.width, size.height));
            }
            let _ = win.show();
        } else {
            let _ = win.hide();
            crate::state::SPLASH_FINISHED.store(true, Ordering::Relaxed);
            crate::services::sync_overlays(&app);
        }
    }
}

#[tauri::command]
pub fn sync_overlay_position(app: AppHandle) {
    crate::services::sync_overlays(&app);
}

#[tauri::command]
pub fn open_wifi_settings() {
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::Win32::UI::Shell::ShellExecuteA;
        let _ = ShellExecuteA(Some(HWND(std::ptr::null_mut())), windows::core::PCSTR(c"open".as_ptr() as *const u8), windows::core::PCSTR(c"ms-availablenetworks:".as_ptr() as *const u8), windows::core::PCSTR::null(), windows::core::PCSTR::null(), SW_SHOWNORMAL);
    }
}

#[tauri::command]
pub fn open_sound_settings() {
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::Win32::UI::Shell::ShellExecuteA;
        let _ = ShellExecuteA(Some(HWND(std::ptr::null_mut())), windows::core::PCSTR(c"open".as_ptr() as *const u8), windows::core::PCSTR(c"ms-settings:sound".as_ptr() as *const u8), windows::core::PCSTR::null(), windows::core::PCSTR::null(), SW_SHOWNORMAL);
    }
}

#[tauri::command]
pub fn open_notification_center() {
    unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        use windows::Win32::UI::Shell::ShellExecuteA;
        let _ = ShellExecuteA(Some(HWND(std::ptr::null_mut())), windows::core::PCSTR(c"open".as_ptr() as *const u8), windows::core::PCSTR(c"ms-actioncenter:".as_ptr() as *const u8), windows::core::PCSTR::null(), windows::core::PCSTR::null(), SW_SHOWNORMAL);
    }
}

#[tauri::command]
pub fn pointer_over_notch(window: Window) -> bool {
    let label = window.label().to_string();
    let mut pt = windows::Win32::Foundation::POINT::default();
    if unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt) }.is_err() {
        return false;
    }
    let Some((win_pos, _)) = main_window_rects().lock().ok().and_then(|m| m.get(&label).copied()) else { return false };
    let Some(r) = notch_rects().lock().ok().and_then(|m| m.get(&label).copied()) else { return false };
    if r.width <= 0 || r.height <= 0 { return false; }
    let scale = window.scale_factor().unwrap_or(1.0);
    let left = win_pos.x + (r.x as f64 * scale) as i32;
    let right = left + (r.width as f64 * scale) as i32;
    let top = win_pos.y + (r.y.max(0) as f64 * scale) as i32;
    let bottom = top + (r.height as f64 * scale) as i32;
    pt.x >= left && pt.x <= right && pt.y >= top && pt.y <= bottom
}
