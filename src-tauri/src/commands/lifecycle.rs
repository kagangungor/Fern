use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};

use crate::services::unregister_appbar_native;
use crate::state::*;
use crate::utils::*;

/// Unregister all active notch and dock appbars across every monitor.
pub fn unregister_all_appbars(handle: &AppHandle) {
    for (label, w) in handle.webview_windows() {
        if crate::state::is_notch_label(&label) || crate::state::is_dock_label(&label) {
            if let Ok(hwnd) = w.hwnd() {
                unregister_appbar_native(hwnd);
            }
        }
    }
}

/// Shared by the tray menu, the in-app Quit button, and the window CloseRequested
/// handlers so that any shutdown path (including Task Manager's WM_CLOSE) behaves
/// identically.
pub fn restore_taskbar_and_exit(handle: &AppHandle) {
    SHUTTING_DOWN.store(true, Ordering::Relaxed);
    NATIVE_TASKBAR_HIDDEN.store(false, Ordering::SeqCst);
    for (_, w) in handle.webview_windows() {
        let _ = w.hide();
    }
    unregister_all_appbars(handle);
    set_taskbar_visibility(true, true);
    NATIVE_TASKBAR_HIDDEN.store(false, Ordering::Relaxed);
    // Destroy all webview windows before exiting so Chromium's UnregisterClass for
    // Chrome_WidgetWin_0 finds no open windows (avoids the harmless Error=1412 log).
    for (_, w) in handle.webview_windows() {
        let _ = w.destroy();
    }
    handle.exit(0);
}

#[tauri::command]
pub async fn quit_fern(handle: AppHandle) {
    restore_taskbar_and_exit(&handle);
}

fn registered_uninstaller() -> Option<std::path::PathBuf> {
    use std::os::windows::process::CommandExt;
    for root in ["HKCU", "HKLM"] {
        let key = format!(r"{}\Software\Microsoft\Windows\CurrentVersion\Uninstall\Fern", root);
        let Ok(out) = std::process::Command::new("reg")
            .args(["query", &key, "/v", "UninstallString"])
            .creation_flags(0x0800_0000)
            .output() else { continue };
        let text = String::from_utf8_lossy(&out.stdout);
        let Some(line) = text.lines().find(|l| l.contains("REG_SZ")) else { continue };
        let value = line.split("REG_SZ").nth(1).unwrap_or("").trim();
        let exe = if let Some(rest) = value.strip_prefix('"') {
            rest.split('"').next().unwrap_or("")
        } else {
            value.split(" --").next().unwrap_or("")
        };
        let path = std::path::PathBuf::from(exe);
        if !exe.is_empty() && path.exists() { return Some(path); }
    }
    None
}

#[tauri::command]
pub async fn uninstall_fern(handle: AppHandle) -> Result<(), String> {
    crate::uninstall_registry::heal_uninstall_registration();

    let mut uninstaller = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or_else(|| "Could not resolve install directory".to_string())?
        .join("uninstall.exe");

    if !uninstaller.exists() {
        uninstaller = registered_uninstaller().ok_or_else(|| {
            "This copy of Fern isn't an installed one (it's a portable or development build), so there's nothing to uninstall. Run it from its install folder, or use Windows Settings > Apps.".to_string()
        })?;
    }

    std::process::Command::new(&uninstaller)
        .arg("--uninstall")
        .current_dir(std::env::temp_dir())
        .spawn()
        .map_err(|e| format!("Couldn't launch {}: {}", uninstaller.display(), e))?;

    restore_taskbar_and_exit(&handle);
    Ok(())
}

#[tauri::command]
pub async fn get_diagnostics(app: AppHandle) -> String {
    tauri::async_runtime::spawn_blocking(move || crate::diagnostics::build_report(&app))
        .await
        .unwrap_or_else(|e| format!("Couldn't build the report: {e}"))
}

#[tauri::command]
pub fn open_log_folder() -> Result<(), String> {
    let dir = crate::diagnostics::log_dir().ok_or_else(|| "No log folder yet".to_string())?;
    std::process::Command::new("explorer")
        .arg(&dir)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_hitbox_logging(enabled: bool) {
    crate::diagnostics::set_hitbox_logging(enabled);
}

#[tauri::command]
pub fn get_hitbox_logging() -> bool {
    crate::diagnostics::hitbox_logging()
}

#[tauri::command]
pub async fn restart_fern(handle: AppHandle) {
    #[cfg(debug_assertions)]
    {
        // Dev just reloads the pages. Leave SHUTTING_DOWN alone here or the hooks stay off.
        crate::state::SPLASH_FINISHED.store(false, Ordering::Relaxed);
        for (label, w) in handle.webview_windows() {
            let extra = label != "main" && label != "dock";
            if crate::state::is_dock_label(&label) || crate::state::is_notch_label(&label) {
                if extra {
                    crate::monitors::destroy_monitor_window(&handle, &label);
                } else {
                    let _ = w.hide();
                }
            }
        }
        for (_, w) in handle.webview_windows() {
            let _ = w.eval("window.location.reload()");
        }
        return;
    }
    #[cfg(not(debug_assertions))]
    {
        relaunch_fern(&handle);
    }
}

/// Relaunches the process. Also used for webview crashes, reloading pages can't fix those.
pub fn relaunch_fern(handle: &AppHandle) {
    SHUTTING_DOWN.store(true, Ordering::Relaxed);
    unregister_all_appbars(handle);
    set_taskbar_visibility(true, true);
    NATIVE_TASKBAR_HIDDEN.store(false, Ordering::Relaxed);
    crate::on_demand_windows::close_all(handle);
    std::env::set_var("FERN_RESTARTING", "1");
    close_single_instance_handles();
    crate::webview_args::clear_if_applied_by_fern();
    handle.restart();
}

// Windows Accent
#[tauri::command]
pub async fn sync_windows_accent(app: tauri::AppHandle, hex: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || crate::windows_accent::sync(&app, &hex))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn restore_windows_accent(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || crate::windows_accent::restore(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn has_windows_accent_backup(app: tauri::AppHandle) -> bool {
    crate::windows_accent::has_backup(&app)
}

#[tauri::command]
pub fn windows_accent_backup_exists(app: tauri::AppHandle) -> bool {
    crate::windows_accent::has_backup(&app)
}

// Hotkeys
#[tauri::command]
pub fn reload_hotkeys() {
    crate::hotkeys::reload();
}
