use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::AppHandle;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};
use crate::types::*;
use crate::state::*;
use crate::utils::*;
use super::hooks::VISIBILITY_THREAD;
use super::mouse_hit::CAPTURE_RECHECK;

pub fn setup_thumbnail_capture(_app_handle: AppHandle) {
    unsafe {
        use windows::Win32::UI::Accessibility::SetWinEventHook;
        use windows::Win32::UI::WindowsAndMessaging::{WINEVENT_OUTOFCONTEXT, EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_FOREGROUND};

        let hook = SetWinEventHook(
            EVENT_SYSTEM_MINIMIZESTART,
            EVENT_SYSTEM_MINIMIZEEND,
            None,
            Some(thumbnail_capture_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        Box::leak(Box::new(hook));

        let focus_hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(focus_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        Box::leak(Box::new(focus_hook));
    }
}

unsafe extern "system" fn focus_event_proc(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _ms_event_time: u32,
) {
    if hwnd.0.is_null() { return; }
    // Wake the visibility loop now so Start/Search updates the dock without waiting for the next tick.
    if let Some(t) = VISIBILITY_THREAD.get() { t.unpark(); }
    // Foreground moved — e.g. Snipping Tool opened/closed or got minimised.
    CAPTURE_RECHECK.store(true, Ordering::Relaxed);
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};

    if !IsWindow(Some(hwnd)).as_bool() { return; }
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 { return; }

    let my_pid = std::process::id();
    let mut pid = 0u32;
    windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == my_pid { return; }

    let hwnd_raw = hwnd.0 as isize;
    if let Some(map) = crate::state::FOCUS_TIMESTAMPS.get() {
        if let Ok(mut guard) = map.lock() {
            guard.insert(hwnd_raw, crate::utils::get_now_ms());
        }
    }

    // Keep the focused window's thumbnail warm so a later minimized one has a cached image.
    std::thread::spawn(move || {
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextW, IsIconic, IsWindow};

        std::thread::sleep(std::time::Duration::from_millis(450));
        let hwnd = HWND(hwnd_raw as *mut _);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() { return; }
        }

        // Skip fullscreen windows (games, video players): capturing them can hitch.
        if crate::utils::is_window_fullscreen(hwnd) { return; }

        let mut text = [0u16; 2];
        unsafe {
            if GetWindowTextW(hwnd, &mut text) == 0 { return; }
        }

        // Refresh at most once per window every two seconds
        if let Some(cache) = crate::state::THUMBNAIL_CACHE.get() {
            if let Ok(guard) = cache.lock() {
                if let Some((_, ts)) = guard.get(&hwnd_raw) {
                    if crate::utils::get_now_ms() - ts < 2000 { return; }
                }
            }
        }

        if THUMB_CAPTURE_IN_FLIGHT.swap(true, Ordering::Relaxed) { return; }
        let _guard = ThumbnailCaptureGuard;

        if let Some(img) = crate::utils::capture_hwnd_to_base64(hwnd, 320, 200) {
            if let Some(cache) = crate::state::THUMBNAIL_CACHE.get() {
                if let Ok(mut guard) = cache.lock() {
                    guard.insert(hwnd_raw, (img, crate::utils::get_now_ms()));
                }
            }
        }
    });
}

static THUMB_CAPTURE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

struct ThumbnailCaptureGuard;

impl Drop for ThumbnailCaptureGuard {
    fn drop(&mut self) {
        THUMB_CAPTURE_IN_FLIGHT.store(false, Ordering::Relaxed);
    }
}

unsafe extern "system" fn thumbnail_capture_proc(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _ms_event_time: u32,
) {
    if hwnd.0.is_null() { return; }

    // Only restore matters. On minimize start the window is mid-animation and
    // PrintWindow can grab a black frame over a good thumbnail.
    if event != windows::Win32::UI::WindowsAndMessaging::EVENT_SYSTEM_MINIMIZEEND { return; }

    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};

    if !IsWindow(Some(hwnd)).as_bool() { return; }
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 { return; }

    let my_pid = std::process::id();
    let mut pid = 0u32;
    windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == my_pid { return; }

    let mut text = [0u16; 512];
    let len = windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut text);
    if len == 0 { return; }

    let hwnd_raw = hwnd.0 as isize;

    std::thread::spawn(move || {
        use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};

        std::thread::sleep(std::time::Duration::from_millis(450));
        let hwnd = HWND(hwnd_raw as *mut _);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() { return; }
        }
        if crate::utils::is_window_fullscreen(hwnd) { return; }

        // Skip if another capture refreshed this window very recently
        if let Some(cache) = crate::state::THUMBNAIL_CACHE.get() {
            if let Ok(guard) = cache.lock() {
                if let Some((_, ts)) = guard.get(&hwnd_raw) {
                    if crate::utils::get_now_ms() - ts < 300 {
                        return;
                    }
                }
            }
        }

        if THUMB_CAPTURE_IN_FLIGHT.swap(true, Ordering::Relaxed) { return; }
        let _guard = ThumbnailCaptureGuard;

        if let Some(img) = crate::utils::capture_hwnd_to_base64(hwnd, 320, 200) {
            if let Some(cache) = crate::state::THUMBNAIL_CACHE.get() {
                if let Ok(mut guard) = cache.lock() {
                    guard.insert(hwnd_raw, (img, crate::utils::get_now_ms()));
                    if guard.len() > 15 {
                        guard.retain(|&k, _| {
                            unsafe { IsWindow(Some(HWND(k as *mut _))).as_bool() }
                        });
                    }
                }
            }
        }
    });
}
