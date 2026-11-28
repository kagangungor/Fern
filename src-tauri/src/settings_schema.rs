use std::collections::HashMap;
use serde_json::Value;

pub const CURRENT_SCHEMA_VERSION: u32 = 2;
pub const SCHEMA_VERSION_KEY: &str = "schemaVersion";

/// Legacy key aliases mapped safely during v1 -> v2 migration without overwriting
/// an already-populated canonical key.
const LEGACY_KEY_ALIASES: &[(&str, &str)] = &[
    ("fern-monitor-mode", "fern-dock-monitor-mode"),
    ("fern-monitor-id", "fern-dock-monitor-id"),
    ("fern-language-code", "fern-language"),
];

const MONITOR_MODE_KEYS: &[&str] = &[
    "fern-dock-monitor-mode",
    "fern-notch-monitor-mode",
];

const WINDOW_MODE_KEYS: &[&str] = &[
    "fern-dock-mode",
    "fern-notch-mode",
];

const WINDOW_MODE_MAP_KEYS: &[&str] = &[
    "fern-dock-mode-by-monitor",
    "fern-notch-mode-by-monitor",
];

const NON_EMPTY_KNOWN_KEYS: &[&str] = &[
    "fern-dock-mode",
    "fern-notch-mode",
    "fern-scale",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub from_version: u32,
    pub to_version: u32,
    pub migrated: bool,
    pub normalized_keys: Vec<String>,
    pub deprecated_keys_mapped: Vec<String>,
}

/// Detect the settings schema version from a raw settings map.
/// Returns `1` if `schemaVersion` is missing or invalid, or its integer value when `>= 2`.
pub fn detect_schema_version(map: &HashMap<String, Value>) -> u32 {
    match map.get(SCHEMA_VERSION_KEY) {
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .filter(|&v| v >= 2)
            .unwrap_or(1),
        _ => 1,
    }
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

fn normalize_monitor_mode_value(raw: &str) -> &'static str {
    match raw.trim().to_ascii_lowercase().as_str() {
        "all" => "all",
        "specific" => "specific",
        _ => "primary",
    }
}

fn normalize_window_mode_map(val: &Value) -> Option<String> {
    let parsed_map: Option<HashMap<String, Value>> = match val {
        Value::String(s) => serde_json::from_str::<HashMap<String, Value>>(s).ok(),
        Value::Object(obj) => Some(obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
        _ => None,
    };

    let mut entries = parsed_map?;
    let mut changed = !matches!(val, Value::String(_));

    for entry_val in entries.values_mut() {
        let replacement = entry_val.as_str().and_then(|s| {
            let trimmed = s.trim();
            if trimmed == "auto-hide" {
                Some("smart".to_string())
            } else if trimmed != s {
                Some(trimmed.to_string())
            } else {
                None
            }
        });
        if let Some(new_val) = replacement {
            *entry_val = Value::String(new_val);
            changed = true;
        }
    }

    if changed {
        serde_json::to_string(&entries).ok()
    } else {
        None
    }
}

/// Idempotently migrate a settings map to `CURRENT_SCHEMA_VERSION`.
///
/// - Already-v2 (or newer) maps return `migrated: false` without mutating user values.
/// - v1 maps are silently upgraded in place while preserving existing `fern-*` string
///   formats expected by `useSettings.ts` and `localStorage`, as well as unknown/custom keys.
pub fn migrate_settings_map(map: &mut HashMap<String, Value>) -> MigrationReport {
    let from_version = detect_schema_version(map);
    if from_version >= CURRENT_SCHEMA_VERSION {
        return MigrationReport {
            from_version,
            to_version: from_version,
            migrated: false,
            normalized_keys: Vec::new(),
            deprecated_keys_mapped: Vec::new(),
        };
    }

    let mut normalized_keys = Vec::new();
    let mut deprecated_keys_mapped = Vec::new();

    // 1. Map deprecated/legacy key aliases without overwriting already-set canonical keys.
    for &(old_key, new_key) in LEGACY_KEY_ALIASES {
        if let Some(old_val) = map.remove(old_key) {
            let canonical_has_value = map.get(new_key).is_some_and(|v| !v.is_null());
            if !canonical_has_value && !old_val.is_null() {
                map.insert(new_key.to_string(), old_val);
                push_unique(&mut deprecated_keys_mapped, format!("{old_key}->{new_key}"));
            }
        }
    }

    // 2. Strip null tombstones on fern-* keys (never dropping valid user preferences).
    let null_fern_keys: Vec<String> = map
        .iter()
        .filter(|(k, v)| k.starts_with("fern-") && v.is_null())
        .map(|(k, _)| k.clone())
        .collect();
    for key in null_fern_keys {
        map.remove(&key);
        push_unique(&mut normalized_keys, key);
    }

    // 3. Normalize boolean/numeric primitives on fern-* keys to strings so frontend
    //    localStorage readers and Rust get_setting_str stay consistent.
    let primitive_keys: Vec<(String, String)> = map
        .iter()
        .filter_map(|(k, v)| {
            if !k.starts_with("fern-") {
                return None;
            }
            match v {
                Value::Bool(b) => Some((k.clone(), if *b { "true" } else { "false" }.to_string())),
                Value::Number(n) => Some((k.clone(), n.to_string())),
                _ => None,
            }
        })
        .collect();
    for (key, str_val) in primitive_keys {
        map.insert(key.clone(), Value::String(str_val));
        push_unique(&mut normalized_keys, key);
    }

    // 4. Normalize legacy "auto-hide" window mode values -> "smart".
    for &key in WINDOW_MODE_KEYS {
        let replacement = map.get(key).and_then(|v| v.as_str()).and_then(|s| {
            let trimmed = s.trim();
            if trimmed == "auto-hide" {
                Some("smart".to_string())
            } else if trimmed != s && !trimmed.is_empty() {
                Some(trimmed.to_string())
            } else {
                None
            }
        });
        if let Some(new_val) = replacement {
            map.insert(key.to_string(), Value::String(new_val));
            push_unique(&mut normalized_keys, key.to_string());
        }
    }

    // 5. Normalize per-monitor window mode JSON maps (e.g. "auto-hide" -> "smart",
    //    or raw JSON object -> JSON string).
    for &key in WINDOW_MODE_MAP_KEYS {
        let normalized_json = map.get(key).and_then(normalize_window_mode_map);
        if let Some(json) = normalized_json {
            map.insert(key.to_string(), Value::String(json));
            push_unique(&mut normalized_keys, key.to_string());
        }
    }

    // 6. Normalize multi-monitor mode values to valid MonitorMode strings.
    for &key in MONITOR_MODE_KEYS {
        let replacement = map.get(key).and_then(|val| {
            let normalized = match val {
                Value::String(s) => normalize_monitor_mode_value(s),
                _ => "primary",
            };
            if val.as_str() != Some(normalized) {
                Some(normalized)
            } else {
                None
            }
        });
        if let Some(normalized) = replacement {
            map.insert(key.to_string(), Value::String(normalized.to_string()));
            push_unique(&mut normalized_keys, key.to_string());
        }
    }

    // 7. Strip empty/whitespace-only corrupt strings for known non-empty keys.
    for &key in NON_EMPTY_KNOWN_KEYS {
        let is_empty_str = map
            .get(key)
            .and_then(|v| v.as_str())
            .is_some_and(|s| s.trim().is_empty());
        if is_empty_str {
            map.remove(key);
            push_unique(&mut normalized_keys, key.to_string());
        }
    }

    // 8. Stamp schemaVersion = 2.
    map.insert(
        SCHEMA_VERSION_KEY.to_string(),
        serde_json::json!(CURRENT_SCHEMA_VERSION),
    );

    normalized_keys.sort();
    deprecated_keys_mapped.sort();

    MigrationReport {
        from_version,
        to_version: CURRENT_SCHEMA_VERSION,
        migrated: true,
        normalized_keys,
        deprecated_keys_mapped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fresh_empty_map_gets_schema_version_2() {
        let mut map = HashMap::new();
        assert_eq!(detect_schema_version(&map), 1);

        let report = migrate_settings_map(&mut map);
        assert!(report.migrated);
        assert_eq!(report.from_version, 1);
        assert_eq!(report.to_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(map.get(SCHEMA_VERSION_KEY), Some(&json!(2)));
        assert!(report.normalized_keys.is_empty());
        assert!(report.deprecated_keys_mapped.is_empty());
    }

    #[test]
    fn v1_map_migrates_silently_and_preserves_user_keys() {
        let mut map = HashMap::new();
        map.insert("fern-theme-mode".to_string(), json!("dark"));
        map.insert("fern-dock-enabled".to_string(), json!("true"));
        map.insert("fern-scale".to_string(), json!("1.25"));
        map.insert("fern-dock-mode".to_string(), json!("auto-hide"));
        map.insert("fern-notch-enabled".to_string(), json!(false));
        map.insert("custom-user-key".to_string(), json!({ "nested": 42 }));

        let report = migrate_settings_map(&mut map);
        assert!(report.migrated);
        assert_eq!(report.from_version, 1);
        assert_eq!(report.to_version, 2);
        assert_eq!(map.get(SCHEMA_VERSION_KEY), Some(&json!(2)));
        assert_eq!(map.get("fern-theme-mode"), Some(&json!("dark")));
        assert_eq!(map.get("fern-dock-enabled"), Some(&json!("true")));
        assert_eq!(map.get("fern-scale"), Some(&json!("1.25")));
        assert_eq!(map.get("fern-dock-mode"), Some(&json!("smart")));
        assert_eq!(map.get("fern-notch-enabled"), Some(&json!("false")));
        assert_eq!(map.get("custom-user-key"), Some(&json!({ "nested": 42 })));
        assert!(report.normalized_keys.contains(&"fern-dock-mode".to_string()));
        assert!(report.normalized_keys.contains(&"fern-notch-enabled".to_string()));
    }

    #[test]
    fn already_v2_map_is_idempotent() {
        let mut map = HashMap::new();
        map.insert(SCHEMA_VERSION_KEY.to_string(), json!(2));
        map.insert("fern-dock-mode".to_string(), json!("smart"));
        map.insert("fern-theme-mode".to_string(), json!("light"));

        let before = map.clone();
        let report = migrate_settings_map(&mut map);
        assert!(!report.migrated);
        assert_eq!(report.from_version, 2);
        assert_eq!(report.to_version, 2);
        assert!(report.normalized_keys.is_empty());
        assert!(report.deprecated_keys_mapped.is_empty());
        assert_eq!(map, before);
    }

    #[test]
    fn corrupt_or_non_numeric_schema_version_falls_back_to_v1_and_migrates() {
        for corrupt in [json!("invalid"), json!("2"), json!(null), json!(true), json!(0), json!(-5), json!(1.5)] {
            let mut map = HashMap::new();
            map.insert(SCHEMA_VERSION_KEY.to_string(), corrupt);
            map.insert("fern-theme-style".to_string(), json!("glass"));

            assert_eq!(detect_schema_version(&map), 1);
            let report = migrate_settings_map(&mut map);
            assert!(report.migrated);
            assert_eq!(report.from_version, 1);
            assert_eq!(report.to_version, 2);
            assert_eq!(map.get(SCHEMA_VERSION_KEY), Some(&json!(2)));
            assert_eq!(map.get("fern-theme-style"), Some(&json!("glass")));
        }
    }

    #[test]
    fn legacy_aliases_map_without_overwriting_existing_canonical_key() {
        let mut map = HashMap::new();
        map.insert("fern-monitor-mode".to_string(), json!("all"));
        map.insert("fern-monitor-id".to_string(), json!("DISPLAY2"));
        map.insert("fern-language-code".to_string(), json!("tr"));
        // Canonical key already set: must NOT be overwritten
        map.insert("fern-dock-monitor-mode".to_string(), json!("specific"));

        let report = migrate_settings_map(&mut map);
        assert!(report.migrated);
        assert_eq!(map.get("fern-dock-monitor-mode"), Some(&json!("specific")));
        assert_eq!(map.get("fern-dock-monitor-id"), Some(&json!("DISPLAY2")));
        assert_eq!(map.get("fern-language"), Some(&json!("tr")));
        assert!(!map.contains_key("fern-monitor-mode"));
        assert!(!map.contains_key("fern-monitor-id"));
        assert!(!map.contains_key("fern-language-code"));
        assert!(!report.deprecated_keys_mapped.contains(&"fern-monitor-mode->fern-dock-monitor-mode".to_string()));
        assert!(report.deprecated_keys_mapped.contains(&"fern-monitor-id->fern-dock-monitor-id".to_string()));
        assert!(report.deprecated_keys_mapped.contains(&"fern-language-code->fern-language".to_string()));
    }

    #[test]
    fn normalizes_per_monitor_maps_and_invalid_monitor_modes() {
        let mut map = HashMap::new();
        map.insert("fern-dock-monitor-mode".to_string(), json!("ALL"));
        map.insert("fern-notch-monitor-mode".to_string(), json!("unknown-mode"));
        map.insert(
            "fern-dock-mode-by-monitor".to_string(),
            json!(r#"{"DISPLAY1":"auto-hide","DISPLAY2":"fixed"}"#),
        );
        map.insert("fern-weather-city".to_string(), json!(null));
        map.insert("fern-scale".to_string(), json!("   "));

        let report = migrate_settings_map(&mut map);
        assert!(report.migrated);
        assert_eq!(map.get("fern-dock-monitor-mode"), Some(&json!("all")));
        assert_eq!(map.get("fern-notch-monitor-mode"), Some(&json!("primary")));
        assert!(!map.contains_key("fern-weather-city"));
        assert!(!map.contains_key("fern-scale"));

        let by_mon_str = map
            .get("fern-dock-mode-by-monitor")
            .and_then(|v| v.as_str())
            .expect("per-monitor map should remain a JSON string");
        let parsed: HashMap<String, String> = serde_json::from_str(by_mon_str).unwrap();
        assert_eq!(parsed.get("DISPLAY1").map(String::as_str), Some("smart"));
        assert_eq!(parsed.get("DISPLAY2").map(String::as_str), Some("fixed"));
    }
}
