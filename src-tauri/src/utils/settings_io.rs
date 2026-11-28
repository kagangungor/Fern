use std::path::PathBuf;

use super::window::retry_io;

pub fn settings_backup_path(path: &std::path::Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".bak");
    PathBuf::from(os)
}

fn write_temp_synced(tmp_path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(tmp_path)?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

pub fn write_json_atomic_with_backup(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent)?;
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("settings.json");
    let pid = std::process::id();
    let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!("{file_name}.{pid}.{seq}.tmp"));

    if let Err(err) = write_temp_synced(&tmp_path, content) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    // Best-effort rotation of previous valid settings.json into settings.json.bak
    if path.exists() {
        if let Ok(existing) = retry_io(|| std::fs::read_to_string(path)) {
            if !existing.trim().is_empty()
                && serde_json::from_str::<std::collections::HashMap<String, serde_json::Value>>(
                    &existing,
                )
                .is_ok()
            {
                let bak_path = settings_backup_path(path);
                let bak_seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
                let bak_tmp = parent.join(format!("{file_name}.bak.{pid}.{bak_seq}.tmp"));
                let bak_res = write_temp_synced(&bak_tmp, &existing)
                    .and_then(|()| retry_io(|| std::fs::rename(&bak_tmp, &bak_path)));
                if let Err(err) = bak_res {
                    let _ = std::fs::remove_file(&bak_tmp);
                    crate::diagnostics::log(&format!(
                        "Failed to update settings backup {}: {err}",
                        bak_path.display()
                    ));
                }
            }
        }
    }

    if let Err(err) = retry_io(|| std::fs::rename(&tmp_path, path)) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    Ok(())
}

pub fn read_settings_map_with_fallback(
    path: &std::path::Path,
) -> Result<std::collections::HashMap<String, serde_json::Value>, String> {
    let bak_path = settings_backup_path(path);
    let read_backup = || -> Result<std::collections::HashMap<String, serde_json::Value>, std::io::Error> {
        let bak_content = retry_io(|| std::fs::read_to_string(&bak_path))?;
        serde_json::from_str(&bak_content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    };

    match retry_io(|| std::fs::read_to_string(path)) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(map) => Ok(map),
            Err(parse_err) => match read_backup() {
                Ok(bak_map) => {
                    crate::diagnostics::log(&format!(
                        "settings.json parse failed ({parse_err}); recovered from backup"
                    ));
                    Ok(bak_map)
                }
                Err(bak_err) => Err(format!(
                    "Failed to parse settings.json ({parse_err}) and backup recovery failed ({bak_err})"
                )),
            },
        },
        Err(io_err) if io_err.kind() == std::io::ErrorKind::NotFound => match read_backup() {
            Ok(bak_map) => {
                crate::diagnostics::log("settings.json missing; recovered from backup");
                Ok(bak_map)
            }
            Err(bak_err) if bak_err.kind() == std::io::ErrorKind::NotFound => {
                Ok(std::collections::HashMap::new())
            }
            Err(bak_err) => Err(format!(
                "settings.json missing and backup recovery failed ({bak_err})"
            )),
        },
        Err(io_err) => match read_backup() {
            Ok(bak_map) => {
                crate::diagnostics::log(&format!(
                    "settings.json read failed ({io_err}); recovered from backup"
                ));
                Ok(bak_map)
            }
            Err(bak_err) => Err(format!(
                "Failed to read settings.json ({io_err}) and backup recovery failed ({bak_err})"
            )),
        },
    }
}

fn settings_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, serde_json::Value>> {
    crate::state::SETTINGS_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Load settings.json into the in-memory cache. Call once at startup.
pub fn init_settings_cache(app: &tauri::AppHandle) {
    use tauri::Manager;

    if let Some(path) = app.path().app_config_dir().ok().map(|p| p.join("settings.json")) {
        match read_settings_map_with_fallback(&path) {
            Ok(mut settings) => {
                let had_existing_data = path.exists() || !settings.is_empty();
                let report = crate::settings_schema::migrate_settings_map(&mut settings);
                if report.migrated && had_existing_data {
                    crate::diagnostics::log(&format!(
                        "Migrated settings.json schema v{} -> v{} (normalized: {:?}, mapped: {:?})",
                        report.from_version,
                        report.to_version,
                        report.normalized_keys,
                        report.deprecated_keys_mapped,
                    ));
                    match serde_json::to_string_pretty(&settings) {
                        Ok(content) => {
                            if let Err(err) = write_json_atomic_with_backup(&path, &content) {
                                crate::diagnostics::log(&format!(
                                    "Failed to persist migrated settings.json: {err}"
                                ));
                            }
                        }
                        Err(err) => {
                            crate::diagnostics::log(&format!(
                                "Failed to serialize migrated settings.json: {err}"
                            ));
                        }
                    }
                }
                if let Ok(mut cache) = settings_cache().lock() {
                    *cache = settings;
                }
            }
            Err(err) => {
                crate::diagnostics::log(&format!("init_settings_cache failed: {err}"));
            }
        }
    }
}

/// Clone the current in-memory settings cache (used as a fallback when disk read fails).
pub fn clone_settings_cache() -> std::collections::HashMap<String, serde_json::Value> {
    settings_cache()
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

/// Replace the entire settings cache (used by the file watcher on external changes).
pub fn replace_settings_cache(new_settings: std::collections::HashMap<String, serde_json::Value>) {
    if let Ok(mut cache) = settings_cache().lock() {
        *cache = new_settings;
    }
}

pub fn get_fern_scale(_app: &tauri::AppHandle) -> f64 {
    get_setting_str(_app, "fern-scale")
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.0)
}

/// Read any string value from the settings cache. Returns None if the key is absent.
pub fn get_setting_str(_app: &tauri::AppHandle, key: &str) -> Option<String> {
    let cache = crate::state::SETTINGS_CACHE.get()?;
    let guard = cache.lock().ok()?;
    guard.get(key)?.as_str().map(|s| s.to_string())
}

/// Read a boolean setting directly from cache without an AppHandle.
/// Like `setting_flag`, but defaults to `true` unless the setting is explicitly `false` / `"false"`.
pub fn setting_flag_default_true(key: &str) -> bool {
    let Some(cache) = crate::state::SETTINGS_CACHE.get() else { return true };
    let Ok(guard) = cache.lock() else { return true };
    match guard.get(key) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s != "false",
        _ => true,
    }
}

pub fn setting_flag(key: &str) -> bool {
    let Some(cache) = crate::state::SETTINGS_CACHE.get() else { return false };
    let Ok(guard) = cache.lock() else { return false };
    match guard.get(key) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn atomic_write_creates_settings_and_rotates_backup() {
        use super::{read_settings_map_with_fallback, settings_backup_path, write_json_atomic_with_backup};

        let root = std::env::temp_dir().join(format!("fern-settings-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("settings.json");
        let bak = settings_backup_path(&path);

        write_json_atomic_with_backup(&path, r#"{"fern-theme":"dark"}"#).unwrap();
        assert!(path.exists());
        assert!(!bak.exists());

        write_json_atomic_with_backup(&path, r#"{"fern-theme":"light"}"#).unwrap();
        assert!(bak.exists());
        assert_eq!(std::fs::read_to_string(&bak).unwrap(), r#"{"fern-theme":"dark"}"#);

        let loaded = read_settings_map_with_fallback(&path).unwrap();
        assert_eq!(loaded.get("fern-theme").and_then(|v| v.as_str()), Some("light"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_settings_recovers_from_valid_backup() {
        use super::{read_settings_map_with_fallback, settings_backup_path};

        let root = std::env::temp_dir().join(format!("fern-settings-recover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        let bak = settings_backup_path(&path);

        std::fs::write(&path, "{corrupt json").unwrap();
        std::fs::write(&bak, r#"{"fern-scale":"1.25"}"#).unwrap();

        let recovered = read_settings_map_with_fallback(&path).unwrap();
        assert_eq!(recovered.get("fern-scale").and_then(|v| v.as_str()), Some("1.25"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_settings_without_backup_returns_err() {
        use super::read_settings_map_with_fallback;

        let root = std::env::temp_dir().join(format!("fern-settings-corrupt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");

        std::fs::write(&path, "{not valid json").unwrap();

        assert!(read_settings_map_with_fallback(&path).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }
}
