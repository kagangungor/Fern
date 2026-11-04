//! Settings and Calendar only exist while they are open: each is built on demand and destroyed on
//! close, so its WebView2 renderer doesn't sit in memory unused. Closing one any other way (Alt+F4)
//! also destroys it, since nothing prevents the default close.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

pub const SETTINGS: &str = "settings";
pub const CALENDAR: &str = "calendar";

static SETTINGS_CREATING: AtomicBool = AtomicBool::new(false);
static CALENDAR_CREATING: AtomicBool = AtomicBool::new(false);

fn creating_flag(label: &str) -> Option<&'static AtomicBool> {
    match label {
        SETTINGS => Some(&SETTINGS_CREATING),
        CALENDAR => Some(&CALENDAR_CREATING),
        _ => None,
    }
}

struct Navigation {
    /// The current settings page has its `settings-navigate` listener in place.
    listening: bool,
    /// A page requested before that, handed over by `take_settings_page`.
    pending: Option<String>,
}

static NAVIGATION: Mutex<Navigation> = Mutex::new(Navigation { listening: false, pending: None });

/// Runs `f` with the window, creating the window first if needed. If a creation is already under way
/// `f` is dropped, so it should only do what the first caller's closure does as well.
fn with_window<F>(app: &AppHandle, label: &'static str, f: F)
where
    F: FnOnce(&WebviewWindow) + Send + 'static,
{
    if let Some(window) = app.get_webview_window(label) {
        f(&window);
        return;
    }
    let Some(creating) = creating_flag(label) else { return };
    if creating.swap(true, Ordering::SeqCst) {
        return;
    }

    let app = app.clone();
    // Building a webview from a sync command or event handler deadlocks on Windows, and those are
    // where the callers run. So build off the main thread.
    tauri::async_runtime::spawn(async move {
        let built = build(&app, label);
        creating.store(false, Ordering::SeqCst);
        match built {
            Ok(window) => {
                let target = window.clone();
                // SetWindowSubclass only works on the thread that owns the window.
                let scheduled = window.run_on_main_thread(move || {
                    let _ = window_vibrancy::apply_acrylic(&target, Some((10, 10, 15, 20)));
                    if let Ok(hwnd) = target.hwnd() {
                        crate::active_look::keep_active_look(hwnd);
                    }
                    let _ = target.center();
                });
                if let Err(e) = scheduled {
                    crate::diagnostics::log(&format!("{label} window setup failed: {e}"));
                }
                let fallback = window.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                    if let Ok(false) = fallback.is_visible() {
                        let _ = fallback.show();
                    }
                });
            }
            Err(e) => crate::diagnostics::log(&format!("{label} window creation failed: {e}")),
        }
    });
}

fn build(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    let config = app.config().app.windows.iter()
        .find(|c| c.label == label)
        .ok_or_else(|| format!("{label} window is missing from tauri.conf.json"))?;
    if label == SETTINGS {
        if let Ok(mut nav) = NAVIGATION.lock() {
            nav.listening = false;
        }
    }
    let window = tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())?;
    // Covers every way of closing, Alt+F4 included.
    let win = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = win.hide();
            let _ = win.destroy();
        }
        if matches!(event, tauri::WindowEvent::Destroyed) {
            crate::memory_saver::trim_soon();
        }
    });
    crate::services::watch_webview_process(app, &window);
    Ok(window)
}

pub fn open(app: &AppHandle, label: &'static str) {
    let handle = app.clone();
    with_window(app, label, move |_| crate::commands::bring_window_to_front(&handle, label));
}

/// Opens Settings on `page`. A window created by this call has no listener yet, so the page is kept
/// until the frontend collects it with `take_settings_page`.
pub fn open_settings_page(app: &AppHandle, page: String) {
    if let Ok(mut nav) = NAVIGATION.lock() {
        if nav.listening && app.get_webview_window(SETTINGS).is_some() {
            let _ = app.emit_to(SETTINGS, "settings-navigate", page);
        } else {
            nav.pending = Some(page);
        }
    }
    open(app, SETTINGS);
}

pub fn take_settings_page() -> Option<String> {
    let mut nav = NAVIGATION.lock().ok()?;
    nav.listening = true;
    nav.pending.take()
}

pub fn close(app: &AppHandle, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.hide();
        let _ = window.destroy();
    }
}

pub fn close_all(app: &AppHandle) {
    close(app, SETTINGS);
    close(app, CALENDAR);
}
