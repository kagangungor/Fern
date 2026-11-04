//! Low-memory browser arguments for every WebView2 in the app. They go through the environment
//! because all webviews of a profile must share the same arguments (on-demand windows included),
//! and because the variable replaces wry's default arguments, whose flags are repeated here.

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

/// Must match `identifier` in tauri.conf.json. These arguments are set before Tauri starts, so the
/// app's path resolver isn't available yet.
const APP_IDENTIFIER: &str = "com.kagangungor.fern";

fn gpu_disabled_in(settings_json: &str) -> bool {
    let Ok(settings) = serde_json::from_str::<serde_json::Value>(settings_json) else {
        return false;
    };
    settings.get(DISABLE_GPU_SETTING).and_then(|v| v.as_str()) == Some("true")
}

fn gpu_disabled() -> bool {
    let Some(appdata) = std::env::var_os("APPDATA") else { return false };
    let path = std::path::PathBuf::from(appdata).join(APP_IDENTIFIER).join("settings.json");
    std::fs::read_to_string(path).is_ok_and(|json| gpu_disabled_in(&json))
}

/// Call before any webview exists. A variable the user set themselves wins.
pub fn apply() {
    if std::env::var_os("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").is_some() {
        return;
    }
    let mut args = String::from(BASE_ARGS);
    if gpu_disabled() {
        args.push_str(DISABLE_GPU_ARGS);
    }
    std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", args);
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
        assert!(gpu_disabled_in(r#"{"fern-webview-disable-gpu":"true"}"#));
    }
}
