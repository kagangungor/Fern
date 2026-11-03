//! The settings window only exists while it is open: it is built on demand and destroyed on close,
//! so its WebView2 renderer doesn't sit in memory unused. Closing it any other way (Alt+F4) also
//! destroys it, since nothing prevents the default close.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

const LABEL: &str = "settings";

static CREATING: AtomicBool = AtomicBool::new(false);

struct Navigation {
    /// The current page has its `settings-navigate` listener in place.
    listening: bool,
    /// A page requested before that, handed over by `take_page`.
    pending: Option<String>,
}

static NAVIGATION: Mutex<Navigation> = Mutex::new(Navigation { listening: false, pending: None });

/// Runs `f` with the settings window, creating the window first if needed. If a creation is already
/// under way `f` is dropped, so it should only do what the first caller's closure does as well.
pub fn with_window<F>(app: &AppHandle, f: F)
where
    F: FnOnce(&WebviewWindow) + Send + 'static,
{
    if let Some(window) = app.get_webview_window(LABEL) {
        f(&window);
        return;
    }
    if CREATING.swap(true, Ordering::SeqCst) {
        return;
    }

    let app = app.clone();
    // Building a webview from a sync command or event handler deadlocks on Windows, and those are
    // where the callers run. So build off the main thread.
    tauri::async_runtime::spawn(async move {
        let built = build(&app);
        CREATING.store(false, Ordering::SeqCst);
        match built {
            Ok(window) => {
                let target = window.clone();
                // SetWindowSubclass only works on the thread that owns the window.
                let scheduled = window.run_on_main_thread(move || {
                    let _ = window_vibrancy::apply_acrylic(&target, Some((10, 10, 15, 20)));
                    if let Ok(hwnd) = target.hwnd() {
                        crate::active_look::keep_active_look(hwnd);
                    }
                    f(&target);
                });
                if let Err(e) = scheduled {
                    crate::diagnostics::log(&format!("settings window setup failed: {e}"));
                }
            }
            Err(e) => crate::diagnostics::log(&format!("settings window creation failed: {e}")),
        }
    });
}

fn build(app: &AppHandle) -> Result<WebviewWindow, String> {
    let config = app.config().app.windows.iter()
        .find(|c| c.label == LABEL)
        .ok_or("settings window is missing from tauri.conf.json")?;
    if let Ok(mut nav) = NAVIGATION.lock() {
        nav.listening = false;
    }
    tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(|e| e.to_string())?
        .build()
        .map_err(|e| e.to_string())
}

pub fn open(app: &AppHandle) {
    let handle = app.clone();
    with_window(app, move |_| crate::commands::bring_window_to_front(&handle, LABEL));
}

/// Opens the window on `page`. A window created by this call has no listener yet, so the page is
/// kept until the frontend collects it with `take_page`.
pub fn open_page(app: &AppHandle, page: String) {
    if let Ok(mut nav) = NAVIGATION.lock() {
        if nav.listening && app.get_webview_window(LABEL).is_some() {
            let _ = app.emit_to(LABEL, "settings-navigate", page);
        } else {
            nav.pending = Some(page);
        }
    }
    open(app);
}

pub fn take_page() -> Option<String> {
    let mut nav = NAVIGATION.lock().ok()?;
    nav.listening = true;
    nav.pending.take()
}

pub fn close(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.destroy();
    }
}
