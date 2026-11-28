use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GetCursorPos, SetForegroundWindow, ShowWindow, ASFW_ANY, SW_SHOW,
};

pub const LAUNCHER: &str = "launcher";
const LAUNCHER_LOGICAL_WIDTH: f64 = 728.0;
const LAUNCHER_LOGICAL_HEIGHT: f64 = 576.0;
const IDLE_DESTROY_SECS: u64 = 300; // 5 minutes when Memory Saver is enabled

static CREATING: AtomicBool = AtomicBool::new(false);
static PENDING_SHOW: AtomicBool = AtomicBool::new(false);
static LAST_HIDDEN: Mutex<Option<Instant>> = Mutex::new(None);
static TRIM_LOOP_STARTED: AtomicBool = AtomicBool::new(false);

pub(crate) fn compute_launcher_position(
    work_left: i32,
    work_top: i32,
    work_width: i32,
    work_height: i32,
    win_width: i32,
    win_height: i32,
) -> (i32, i32) {
    let x = work_left + ((work_width - win_width) / 2).max(0);
    let desired_y = work_top + ((work_height as f64) * 0.21).round() as i32;
    let min_y = work_top + 48; // keep clear of top notch
    let max_y = (work_top + work_height - win_height - 16).max(min_y);
    let y = desired_y.clamp(min_y, max_y);
    (x, y)
}

fn active_monitor_work_area() -> Option<(i32, i32, i32, i32)> {
    unsafe {
        let mut pt = POINT::default();
        GetCursorPos(&mut pt).ok()?;
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(hmon, &mut info).as_bool() {
            return None;
        }
        let r = info.rcWork;
        Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
    }
}

fn position_on_active_monitor(win: &WebviewWindow) {
    let scale = win.scale_factor().unwrap_or(1.0);
    let phys_w = (LAUNCHER_LOGICAL_WIDTH * scale).round() as i32;
    let phys_h = (LAUNCHER_LOGICAL_HEIGHT * scale).round() as i32;
    if let Some((left, top, width, height)) = active_monitor_work_area() {
        let (x, y) = compute_launcher_position(left, top, width, height, phys_w, phys_h);
        let _ = win.set_position(PhysicalPosition::new(x, y));
    } else {
        let _ = win.center();
    }
}

fn mark_hidden_now() {
    if let Ok(mut guard) = LAST_HIDDEN.lock() {
        *guard = Some(Instant::now());
    }
}

fn clear_hidden_timer() {
    if let Ok(mut guard) = LAST_HIDDEN.lock() {
        *guard = None;
    }
}

fn start_idle_trim_loop(app: &AppHandle) {
    if TRIM_LOOP_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let mem_saver = crate::utils::get_setting_str(&handle, "fern-memory-saver")
                .as_deref()
                == Some("true");
            if !mem_saver {
                continue;
            }
            let expired = LAST_HIDDEN
                .lock()
                .ok()
                .and_then(|g| *g)
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(IDLE_DESTROY_SECS));
            if expired {
                if let Some(win) = handle.get_webview_window(LAUNCHER) {
                    if !win.is_visible().unwrap_or(false) {
                        clear_hidden_timer();
                        let target = win.clone();
                        let _ = win.run_on_main_thread(move || {
                            if !target.is_visible().unwrap_or(false) {
                                let _ = target.destroy();
                                crate::memory_saver::trim_soon();
                            }
                        });
                    }
                }
            }
        }
    });
}

fn present(win: &WebviewWindow) {
    clear_hidden_timer();
    let target = win.clone();
    // Always run Win32 show/focus calls on the window's owning main thread, outside any
    // WebView2 IPC callback or background thread, to avoid COM re-entrancy / SendMessage deadlocks.
    let _ = win.run_on_main_thread(move || {
        position_on_active_monitor(&target);
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
        }
        let _ = target.show();
        let _ = target.set_focus();
        if let Ok(hwnd) = target.hwnd() {
            unsafe {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
        }
        let _ = target.emit("launcher-opened", ());
    });
}

fn build_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|c| c.label == LAUNCHER)
        .ok_or_else(|| "launcher window missing from tauri.conf.json".to_string())?;
    let window = tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;

    let dark_mode: i32 = match crate::utils::get_setting_str(app, "fern-theme-mode").as_deref() {
        Some(m) if m.trim().eq_ignore_ascii_case("light") => 0,
        _ => 1,
    };
    let target = window.clone();
    let _ = window.run_on_main_thread(move || {
        if let Ok(hwnd) = target.hwnd() {
            unsafe {
                let _ = DwmSetWindowAttribute(
                    hwnd,
                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                    (&dark_mode as *const i32).cast(),
                    std::mem::size_of::<i32>() as u32,
                );
            }
        }
    });

    let win_for_events = window.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::Focused(false) => {
            if win_for_events.is_visible().unwrap_or(false) {
                mark_hidden_now();
                let target = win_for_events.clone();
                let _ = win_for_events.run_on_main_thread(move || {
                    if target.is_visible().unwrap_or(false) {
                        let _ = target.hide();
                        let _ = target.emit("launcher-closed", ());
                    }
                });
            }
        }
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            mark_hidden_now();
            let target = win_for_events.clone();
            let _ = win_for_events.run_on_main_thread(move || {
                let _ = target.hide();
                let _ = target.emit("launcher-closed", ());
            });
        }
        _ => {}
    });

    crate::services::watch_webview_process(app, &window);
    start_idle_trim_loop(app);
    Ok(window)
}

fn ensure_and_maybe_show(app: &AppHandle, show_when_ready: bool) {
    if let Some(win) = app.get_webview_window(LAUNCHER) {
        if show_when_ready {
            present(&win);
        }
        return;
    }
    if show_when_ready {
        PENDING_SHOW.store(true, Ordering::SeqCst);
    }
    if CREATING.swap(true, Ordering::SeqCst) {
        return;
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let res = build_window(&handle);
        CREATING.store(false, Ordering::SeqCst);
        match res {
            Ok(win) => {
                if !PENDING_SHOW.load(Ordering::SeqCst) {
                    mark_hidden_now();
                } else {
                    // Fallback if frontend ready event is slow
                    let fallback_win = win.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(1200)).await;
                        if PENDING_SHOW.swap(false, Ordering::SeqCst) {
                            present(&fallback_win);
                        }
                    });
                }
            }
            Err(e) => {
                PENDING_SHOW.store(false, Ordering::SeqCst);
                crate::diagnostics::log(&format!("launcher window creation failed: {e}"));
            }
        }
    });
}

pub fn show(app: &AppHandle) {
    ensure_and_maybe_show(app, true);
}

pub fn hide(app: &AppHandle) {
    PENDING_SHOW.store(false, Ordering::SeqCst);
    if let Some(win) = app.get_webview_window(LAUNCHER) {
        mark_hidden_now();
        let target = win.clone();
        let _ = win.run_on_main_thread(move || {
            if target.is_visible().unwrap_or(false) {
                let _ = target.hide();
                let _ = target.emit("launcher-closed", ());
            }
        });
    }
}

pub fn toggle(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(LAUNCHER) {
        if win.is_visible().unwrap_or(false) {
            hide(app);
            return;
        }
    }
    let recently_hidden = LAST_HIDDEN
        .lock()
        .ok()
        .and_then(|g| *g)
        .is_some_and(|t| t.elapsed() < Duration::from_millis(180));
    if recently_hidden {
        return;
    }
    show(app);
}

#[tauri::command]
pub async fn toggle_launcher(app: AppHandle) {
    toggle(&app);
}

#[tauri::command]
pub async fn hide_launcher(app: AppHandle) {
    hide(&app);
}

#[tauri::command]
pub async fn launcher_ready(app: AppHandle) {
    if PENDING_SHOW.swap(false, Ordering::SeqCst) {
        if let Some(win) = app.get_webview_window(LAUNCHER) {
            present(&win);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::compute_launcher_position;

    #[test]
    fn centers_horizontally_and_places_near_upper_fifth() {
        let (x, y) = compute_launcher_position(0, 0, 1920, 1080, 640, 520);
        assert_eq!(x, 640);
        assert_eq!(y, 227);
    }

    #[test]
    fn clamps_on_short_displays_and_secondary_negative_origin_monitors() {
        let (x, y) = compute_launcher_position(-1920, 0, 1920, 600, 640, 520);
        assert_eq!(x, -1280);
        assert!(y >= 48 && y <= 600 - 520 - 16);
    }
}
