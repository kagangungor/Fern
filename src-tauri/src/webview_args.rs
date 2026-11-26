//! Low-memory browser arguments for every WebView2 in the app. They go through the environment
//! because all webviews of a profile must share the same arguments (on-demand windows included),
//! and because the variable replaces wry's default arguments, whose flags are repeated here.

use std::sync::atomic::{AtomicBool, Ordering};

const BASE_ARGS: &str = concat!(
    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,",
    "SpareRendererForSitePerProcess,",
    "InterestFeedContentSuggestions,MediaRouter,OptimizationHints,Translate,",
    "AutofillServerCommunication,CertificateTransparencyComponentUpdater,",
    "SegmentationPlatform,BackgroundFetch ",
    "--autoplay-policy=no-user-gesture-required ",
    "--no-first-run --no-default-browser-check --disable-sync ",
    "--disable-background-networking --disable-component-update ",
    "--disable-domain-reliability --disable-breakpad ",
    "--disk-cache-size=16777216 --media-cache-size=8388608 ",
    "--enable-low-end-device-mode ",
    "--js-flags=--max-semi-space-size=1 ",
    "--renderer-process-limit=1 ",
    "--num-raster-threads=2"
);

/// Software rendering frees the GPU process, but animations then cost CPU. So it stays opt-in.
const DISABLE_GPU_ARGS: &str = " --disable-gpu";
const DISABLE_GPU_SETTING: &str = "fern-webview-disable-gpu";
const DOCK_MONITOR_MODE_SETTING: &str = "fern-dock-monitor-mode";
const NOTCH_MONITOR_MODE_SETTING: &str = "fern-notch-monitor-mode";

/// Must match `identifier` in tauri.conf.json. These arguments are set before Tauri starts, so the
/// app's path resolver isn't available yet.
const APP_IDENTIFIER: &str = "com.kagangungor.fern";

static APPLIED_BY_FERN: AtomicBool = AtomicBool::new(false);

fn gpu_disabled_in(settings_json: &str) -> bool {
    let Ok(settings) = serde_json::from_str::<serde_json::Value>(settings_json) else {
        return false;
    };
    match settings.get(DISABLE_GPU_SETTING) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    }
}

fn multi_monitor_all_in(settings_json: &str) -> bool {
    let Ok(settings) = serde_json::from_str::<serde_json::Value>(settings_json) else {
        return false;
    };
    let is_all = |key: &str| {
        matches!(settings.get(key), Some(serde_json::Value::String(s)) if s == "all")
    };
    is_all(DOCK_MONITOR_MODE_SETTING) || is_all(NOTCH_MONITOR_MODE_SETTING)
}

fn read_settings_json_from_path(path: &std::path::Path) -> Option<String> {
    if let Ok(json) = crate::utils::retry_io(|| std::fs::read_to_string(path)) {
        if serde_json::from_str::<serde_json::Value>(&json).is_ok() {
            return Some(json);
        }
    }
    let bak = crate::utils::settings_backup_path(path);
    crate::utils::retry_io(|| std::fs::read_to_string(&bak))
        .ok()
        .filter(|json| serde_json::from_str::<serde_json::Value>(json).is_ok())
}

fn read_settings_json() -> Option<String> {
    let appdata = std::env::var_os("APPDATA")?;
    let path = std::path::PathBuf::from(appdata).join(APP_IDENTIFIER).join("settings.json");
    read_settings_json_from_path(&path)
}

fn build_args(gpu_disabled: bool, multi_monitor_all: bool) -> String {
    let mut args = if multi_monitor_all {
        BASE_ARGS.replacen("--renderer-process-limit=1", "--renderer-process-limit=3", 1)
    } else {
        String::from(BASE_ARGS)
    };
    if gpu_disabled {
        args.push_str(DISABLE_GPU_ARGS);
    }
    args
}

/// Call before any webview exists. A variable the user set themselves wins.
pub fn apply() {
    if std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_some() {
        return;
    }
    let settings_json = read_settings_json();
    let gpu_disabled = settings_json.as_deref().is_some_and(gpu_disabled_in);
    let multi_monitor_all = settings_json.as_deref().is_some_and(multi_monitor_all_in);
    let args = build_args(gpu_disabled, multi_monitor_all);
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", args);
    APPLIED_BY_FERN.store(true, Ordering::Relaxed);
}

/// Clear `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` before a restart if Fern itself set it,
/// so the relaunched process re-reads `fern-webview-disable-gpu` from `settings.json`.
/// A user-supplied environment variable is never modified.
pub fn clear_if_applied_by_fern() {
    if APPLIED_BY_FERN.swap(false, Ordering::Relaxed) {
        std::env::remove_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_args_keep_the_wry_defaults() {
        assert!(BASE_ARGS.contains("msWebOOUI"));
        assert!(BASE_ARGS.contains("msPdfOOUI"));
        assert!(BASE_ARGS.contains("msSmartScreenProtection"));
    }

    #[test]
    fn feature_list_is_a_single_flag() {
        assert_eq!(BASE_ARGS.matches("--disable-features=").count(), 1);
    }

    #[test]
    fn identifier_matches_tauri_config() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], APP_IDENTIFIER);
    }

    #[test]
    fn gpu_stays_on_unless_asked() {
        assert!(!gpu_disabled_in("{}"));
        assert!(!gpu_disabled_in("not json"));
        assert!(!gpu_disabled_in(r#"{"fern-webview-disable-gpu":"false"}"#));
        assert!(!gpu_disabled_in(r#"{"fern-webview-disable-gpu":false}"#));
        assert!(gpu_disabled_in(r#"{"fern-webview-disable-gpu":"true"}"#));
        assert!(gpu_disabled_in(r#"{"fern-webview-disable-gpu":true}"#));
    }

    #[test]
    fn multi_monitor_all_detects_dock_or_notch_all_mode() {
        assert!(!multi_monitor_all_in("{}"));
        assert!(!multi_monitor_all_in("not json"));
        assert!(!multi_monitor_all_in(r#"{"fern-dock-monitor-mode":"primary","fern-notch-monitor-mode":"specific"}"#));
        assert!(multi_monitor_all_in(r#"{"fern-dock-monitor-mode":"all"}"#));
        assert!(multi_monitor_all_in(r#"{"fern-notch-monitor-mode":"all"}"#));
        assert!(multi_monitor_all_in(r#"{"fern-dock-monitor-mode":"primary","fern-notch-monitor-mode":"all"}"#));
    }

    #[test]
    fn build_args_relaxes_renderer_limit_only_for_multi_monitor_all() {
        let single = build_args(false, false);
        assert!(single.contains("--renderer-process-limit=1"));
        assert!(!single.contains("--renderer-process-limit=3"));
        assert!(!single.contains("--disable-gpu"));

        let multi = build_args(false, true);
        assert!(multi.contains("--renderer-process-limit=3"));
        assert!(!multi.contains("--renderer-process-limit=1"));
        assert!(!multi.contains("--disable-gpu"));

        let multi_no_gpu = build_args(true, true);
        assert!(multi_no_gpu.contains("--renderer-process-limit=3"));
        assert!(multi_no_gpu.contains("--disable-gpu"));
    }

    #[test]
    fn clear_if_applied_by_fern_only_clears_fern_owned_var() {
        const VAR: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
        let prev = std::env::var_os(VAR);

        // User-supplied value must not be overwritten or cleared.
        APPLIED_BY_FERN.store(false, Ordering::Relaxed);
        std::env::set_var(VAR, "--user-custom-flag");
        apply();
        assert_eq!(std::env::var(VAR).as_deref(), Ok("--user-custom-flag"));
        clear_if_applied_by_fern();
        assert_eq!(std::env::var(VAR).as_deref(), Ok("--user-custom-flag"));

        // When Fern applies the variable itself, clear_if_applied_by_fern removes it.
        std::env::remove_var(VAR);
        apply();
        assert!(std::env::var_os(VAR).is_some());
        clear_if_applied_by_fern();
        assert!(std::env::var_os(VAR).is_none());

        match prev {
            Some(val) => std::env::set_var(VAR, val),
            None => std::env::remove_var(VAR),
        }
    }
}
