use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;

use super::dock::change_dock_mode;
use super::notch::change_notch_mode;

fn re_register_appbars(app: &AppHandle, settings: &HashMap<String, serde_json::Value>) {
    let notch_fixed = settings.get("fern-notch-mode").map(|v| v.as_str() == Some("fixed")).unwrap_or(true);
    let dock_fixed = settings.get("fern-dock-mode").map(|v| v.as_str() == Some("fixed")).unwrap_or(false);
    for (label, win) in app.webview_windows() {
        if crate::state::is_notch_label(&label) && notch_fixed {
            crate::services::register_appbar(win);
        } else if crate::state::is_dock_label(&label) && dock_fixed {
            crate::services::register_dock_appbar(win);
        }
    }
}

const MONITOR_TARGETING_KEYS: [&str; 5] = [
    "fern-dock-monitor-mode",
    "fern-dock-monitor-id",
    "fern-notch-monitor-mode",
    "fern-notch-monitor-id",
    "fern-dock-enabled",
];

fn settings_file_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn apply_setting_side_effects(
    app: &AppHandle,
    settings: &HashMap<String, serde_json::Value>,
    changed_keys: &[&str],
) {
    if changed_keys.contains(&"fern-agent-api") {
        let on = settings
            .get("fern-agent-api")
            .map(|v| matches!(v.as_str(), Some("true")) || v.as_bool() == Some(true))
            .unwrap_or(false);
        crate::agent_api::set_enabled(app, on);
    }

    if changed_keys.contains(&"fern-scale") {
        re_register_appbars(app, settings);
    }

    if changed_keys.contains(&"fern-dock-reserve-space") {
        let app_clone = app.clone();
        let dock_mode = settings
            .get("fern-dock-mode")
            .and_then(|v| v.as_str())
            .unwrap_or("smart")
            .to_string();
        tauri::async_runtime::spawn(async move {
            change_dock_mode(app_clone, dock_mode).await;
        });
    }

    if changed_keys.contains(&"fern-notch-respect-workarea") {
        for (label, win) in app.webview_windows() {
            if crate::state::is_notch_label(&label) {
                if crate::state::get_flag(crate::state::main_appbar_registered(), &label) {
                    crate::services::register_appbar(win);
                } else {
                    crate::services::position_notch_window(&win);
                    if let Ok(hwnd) = win.hwnd() {
                        crate::utils::re_assert_topmost(hwnd);
                    }
                }
            }
        }
    }

    if changed_keys.contains(&"fern-language") {
        crate::i18n::refresh_tray_labels(app);
    }

    if changed_keys.iter().any(|k| MONITOR_TARGETING_KEYS.contains(k)) {
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            crate::monitors::sync_monitor_windows(&app_clone);
            let dock_enabled = crate::utils::get_setting_str(&app_clone, "fern-dock-enabled")
                .map(|v| v == "true")
                .unwrap_or(true);
            if dock_enabled {
                let dock_mode = crate::utils::get_setting_str(&app_clone, "fern-dock-mode")
                    .unwrap_or_else(|| "smart".to_string());
                change_dock_mode(app_clone.clone(), dock_mode).await;
            }
            let notch_mode = crate::utils::get_setting_str(&app_clone, "fern-notch-mode")
                .unwrap_or_else(|| "fixed".to_string());
            change_notch_mode(app_clone.clone(), notch_mode).await;
        });
    }
}

#[tauri::command]
pub fn save_setting(app: AppHandle, key: String, value: serde_json::Value) -> Result<(), String> {
    let path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("settings.json");
    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    let (settings, emitted_value) = {
        let _guard = settings_file_lock().lock().unwrap_or_else(|e| e.into_inner());
        let mut settings = match crate::utils::read_settings_map_with_fallback(&path) {
            Ok(existing) => existing,
            Err(err) => {
                let cached = crate::utils::clone_settings_cache();
                if !cached.is_empty() {
                    crate::diagnostics::log(&format!(
                        "save_setting({key}): failed to read settings.json ({err}); falling back to in-memory cache"
                    ));
                    cached
                } else {
                    crate::diagnostics::log(&format!(
                        "save_setting({key}): refusing to overwrite unreadable settings.json ({err})"
                    ));
                    return Err(err);
                }
            }
        };
        crate::settings_schema::migrate_settings_map(&mut settings);
        settings.insert(key.clone(), value);
        settings.insert(
            crate::settings_schema::SCHEMA_VERSION_KEY.to_string(),
            serde_json::json!(crate::settings_schema::CURRENT_SCHEMA_VERSION),
        );
        let content = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
        crate::utils::write_json_atomic_with_backup(&path, &content).map_err(|e| e.to_string())?;
        crate::utils::replace_settings_cache(settings.clone());
        let emitted = settings[&key].clone();
        (settings, emitted)
    };

    let _ = app.emit("settings-changed", serde_json::json!({ "key": &key, "value": &emitted_value }));
    apply_setting_side_effects(&app, &settings, &[key.as_str()]);

    Ok(())
}

#[tauri::command]
pub fn get_monitors(app: AppHandle) -> Vec<crate::types::MonitorInfo> {
    crate::monitors::list_monitors(&app)
}

#[tauri::command]
pub fn get_dock_mode_for_window(window: tauri::WebviewWindow, app: AppHandle) -> String {
    crate::monitors::resolve_mode_for_label(&app, crate::types::WindowKind::Dock, window.label())
}

#[tauri::command]
pub fn get_notch_mode_for_window(window: tauri::WebviewWindow, app: AppHandle) -> String {
    crate::monitors::resolve_mode_for_label(&app, crate::types::WindowKind::Notch, window.label())
}

#[tauri::command]
pub fn set_dock_mode_for_window(window: tauri::WebviewWindow, app: AppHandle, mode: String) -> Result<(), String> {
    let res = crate::monitors::set_mode_for_label(&app, crate::types::WindowKind::Dock, window.label(), &mode);
    let app_clone = app.clone();
    let mode_clone = mode.clone();
    tauri::async_runtime::spawn(async move {
        change_dock_mode(app_clone, mode_clone).await;
    });
    res
}

#[tauri::command]
pub fn set_notch_mode_for_window(window: tauri::WebviewWindow, app: AppHandle, mode: String) -> Result<(), String> {
    let res = crate::monitors::set_mode_for_label(&app, crate::types::WindowKind::Notch, window.label(), &mode);
    let app_clone = app.clone();
    let mode_clone = mode.clone();
    tauri::async_runtime::spawn(async move {
        change_notch_mode(app_clone, mode_clone).await;
    });
    res
}

#[tauri::command]
pub fn load_settings(app: AppHandle) -> Result<HashMap<String, serde_json::Value>, String> {
    let path = match app.path().app_config_dir() {
        Ok(p) => p.join("settings.json"),
        Err(_) => return Ok(HashMap::new()),
    };
    let _guard = settings_file_lock().lock().unwrap_or_else(|e| e.into_inner());
    match crate::utils::read_settings_map_with_fallback(&path) {
        Ok(mut settings) => {
            let had_existing_data = path.exists() || !settings.is_empty();
            let report = crate::settings_schema::migrate_settings_map(&mut settings);
            if report.migrated && had_existing_data {
                crate::diagnostics::log(&format!(
                    "load_settings migrated settings.json schema v{} -> v{} (normalized: {:?}, mapped: {:?})",
                    report.from_version,
                    report.to_version,
                    report.normalized_keys,
                    report.deprecated_keys_mapped,
                ));
                if let Ok(content) = serde_json::to_string_pretty(&settings) {
                    if let Err(err) = crate::utils::write_json_atomic_with_backup(&path, &content) {
                        crate::diagnostics::log(&format!(
                            "load_settings failed to persist migrated settings.json: {err}"
                        ));
                    }
                }
                crate::utils::replace_settings_cache(settings.clone());
            }
            Ok(settings)
        }
        Err(err) => {
            crate::diagnostics::log(&format!("load_settings failed: {err}"));
            Ok(HashMap::new())
        }
    }
}

#[tauri::command]
pub fn reset_settings(app: AppHandle) -> Result<(), String> {
    let path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("settings.json");
    {
        let _guard = settings_file_lock().lock().unwrap_or_else(|e| e.into_inner());
        crate::utils::write_json_atomic_with_backup(&path, "{}").map_err(|e| e.to_string())?;
        let bak_path = crate::utils::settings_backup_path(&path);
        if bak_path.exists() {
            let _ = crate::utils::retry_io(|| std::fs::remove_file(&bak_path));
        }
        crate::utils::replace_settings_cache(HashMap::new());
    }
    std::thread::spawn(|| crate::agent_api::set_hooks(false));
    Ok(())
}

#[tauri::command]
pub fn export_settings(app: AppHandle) -> Result<String, String> {
    let path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("settings.json");
    let _guard = settings_file_lock().lock().unwrap_or_else(|e| e.into_inner());
    if let Ok(content) = std::fs::read_to_string(&path) {
        // Validate it's valid JSON before returning
        let _settings: HashMap<String, serde_json::Value> =
            serde_json::from_str(&content).map_err(|e| format!("Invalid settings file: {}", e))?;
        Ok(content)
    } else {
        Err("No settings file found".into())
    }
}

/// Opens native save dialog in Rust and exports settings directly to the chosen path
#[tauri::command]
pub async fn export_settings_dialog(app: AppHandle) -> Result<bool, String> {
    let settings_json = export_settings(app.clone())?;

    let file_path = app
        .dialog()
        .file()
        .set_title("Export Fern Settings")
        .set_file_name("fern-settings.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file();

    if let Some(path) = file_path {
        let path_buf = path.into_path().map_err(|e| e.to_string())?;
        std::fs::write(&path_buf, settings_json).map_err(|e| format!("Failed to write settings: {}", e))?;
        Ok(true)
    } else {
        Ok(false) // User cancelled dialog
    }
}

/// Opens native file picker in Rust, validates content, and imports settings
#[tauri::command]
pub async fn import_settings_dialog(app: AppHandle) -> Result<bool, String> {
    const MAX_SETTINGS_SIZE_BYTES: u64 = 2 * 1024 * 1024; // 2 MB limit

    let file_path = app
        .dialog()
        .file()
        .set_title("Import Fern Settings")
        .add_filter("JSON", &["json"])
        .blocking_pick_file();

    if let Some(path) = file_path {
        let path_buf = path.into_path().map_err(|e| e.to_string())?;

        let metadata = std::fs::metadata(&path_buf).map_err(|e| e.to_string())?;
        if metadata.len() > MAX_SETTINGS_SIZE_BYTES {
            return Err("Settings file is too large (maximum 2MB allowed)".into());
        }

        let content = std::fs::read_to_string(&path_buf).map_err(|e| format!("Failed to read file: {}", e))?;
        import_settings(app, content)?;
        Ok(true)
    } else {
        Ok(false) // User cancelled dialog
    }
}

#[tauri::command]
pub fn import_settings(app: AppHandle, settings: String) -> Result<(), String> {
    let mut imported: HashMap<String, serde_json::Value> =
        serde_json::from_str(&settings).map_err(|e| format!("Invalid JSON: {}", e))?;
    let report = crate::settings_schema::migrate_settings_map(&mut imported);
    if report.migrated {
        crate::diagnostics::log(&format!(
            "import_settings migrated imported settings schema v{} -> v{} (normalized: {:?}, mapped: {:?})",
            report.from_version,
            report.to_version,
            report.normalized_keys,
            report.deprecated_keys_mapped,
        ));
    }

    let path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("settings.json");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let (removed_keys, all_changed_keys) = {
        let _guard = settings_file_lock().lock().unwrap_or_else(|e| e.into_inner());
        let previous = crate::utils::read_settings_map_with_fallback(&path).unwrap_or_default();

        let content = serde_json::to_string_pretty(&imported).map_err(|e| e.to_string())?;
        crate::utils::write_json_atomic_with_backup(&path, &content).map_err(|e| e.to_string())?;
        crate::utils::replace_settings_cache(imported.clone());

        let removed: Vec<String> = previous
            .keys()
            .filter(|k| !imported.contains_key(*k))
            .cloned()
            .collect();
        let mut changed: Vec<String> = imported.keys().cloned().collect();
        for k in &removed {
            changed.push(k.clone());
        }
        (removed, changed)
    };

    // Broadcast updates (updating localStorage and React state across all windows)
    for key in &removed_keys {
        let _ = app.emit(
            "settings-external-changed",
            serde_json::json!({ "key": key, "value": serde_json::Value::Null }),
        );
    }
    for (key, value) in &imported {
        let _ = app.emit(
            "settings-external-changed",
            serde_json::json!({ "key": key, "value": value }),
        );
    }

    let key_refs: Vec<&str> = all_changed_keys.iter().map(String::as_str).collect();
    apply_setting_side_effects(&app, &imported, &key_refs);

    Ok(())
}

pub fn setup_settings_watcher(app: AppHandle) {
    use tauri::Manager;

    let config_dir = match app.path().app_config_dir() {
        Ok(p) => p,
        Err(_) => return,
    };
    let settings_path = config_dir.join("settings.json");

    std::thread::spawn(move || {
        use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_LIST_DIRECTORY,
            FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
            OPEN_EXISTING, ReadDirectoryChangesW,
        };

        unsafe {
            let dir_path: Vec<u16> = config_dir.to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            let dir_handle = match CreateFileW(
                windows::core::PCWSTR(dir_path.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                windows::Win32::Storage::FileSystem::FILE_SHARE_READ
                    | windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE
                    | windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            ) {
                Ok(h) if h != INVALID_HANDLE_VALUE => h,
                _ => return,
            };

            let mut notify_buffer = [0u8; 4096];
            let mut bytes_returned = 0u32;

            loop {
                let success = ReadDirectoryChangesW(
                    dir_handle,
                    notify_buffer.as_mut_ptr() as *mut _,
                    notify_buffer.len() as u32,
                    false,
                    FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME,
                    Some(&mut bytes_returned),
                    None,
                    None,
                );

                if success.is_err() {
                    break;
                }

                std::thread::sleep(std::time::Duration::from_millis(200));

                let parsed = {
                    let _guard = settings_file_lock().lock().unwrap_or_else(|e| e.into_inner());
                    crate::utils::retry_io(|| std::fs::read_to_string(&settings_path))
                        .ok()
                        .and_then(|c| serde_json::from_str::<HashMap<String, serde_json::Value>>(&c).ok())
                };
                if let Some(new_settings) = parsed {
                    // Collect diffs while holding the lock, then drop before emitting
                    let (changed, removed) = {
                        let mut cache = crate::state::SETTINGS_CACHE
                            .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());

                        let mut changed = Vec::new();
                        for (key, value) in &new_settings {
                            if cache.get(key) != Some(value) {
                                changed.push((key.clone(), value.clone()));
                            }
                        }

                        let removed: Vec<String> = cache.keys()
                            .filter(|k| !new_settings.contains_key(*k))
                            .cloned()
                            .collect();

                        *cache = new_settings.clone();
                        (changed, removed)
                    };
                    // Lock dropped — safe to emit without blocking save_setting
                    for (key, value) in &changed {
                        let _ = app.emit(
                            "settings-external-changed",
                            serde_json::json!({ "key": key, "value": value }),
                        );
                    }
                    for key in removed {
                        let _ = app.emit(
                            "settings-external-changed",
                            serde_json::json!({ "key": key, "value": null }),
                        );
                    }
                }
            }

            let _ = CloseHandle(dir_handle);
        }
    });
}
