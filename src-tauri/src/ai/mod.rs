pub mod capture;
pub mod cli;
pub mod providers;
pub mod secrets;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Emitter};

use crate::ai::providers::{AiMessage, Provider};
use crate::memory_saver::trim_soon;
use crate::utils::get_setting_str;

static ACTIVE_REQUESTS: OnceLock<Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>> =
    OnceLock::new();

fn active_requests() -> &'static Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>> {
    ACTIVE_REQUESTS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct RequestGuard {
    request_id: String,
}

impl RequestGuard {
    fn register(request_id: String, tx: tokio::sync::watch::Sender<bool>) -> Self {
        let mut guard = active_requests().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(prev) = guard.insert(request_id.clone(), tx) {
            let _ = prev.send(true);
        }
        Self { request_id }
    }
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        let mut guard = active_requests().lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(&self.request_id);
    }
}

struct TrimOnDrop;

impl Drop for TrimOnDrop {
    fn drop(&mut self) {
        trim_soon();
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AiProviderStatus {
    pub id: String,
    pub label: String,
    pub has_api_key: bool,
    pub cli_installed: bool,
    pub active_backend: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AiStatusSnapshot {
    pub default_provider: String,
    pub backend_mode: String,
    pub providers: Vec<AiProviderStatus>,
}

fn normalize_backend_mode(raw: Option<String>) -> String {
    match raw
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("api") => "api".to_string(),
        Some("cli") => "cli".to_string(),
        _ => "auto".to_string(),
    }
}

pub fn provider_candidates(
    requested: Provider,
    backend: &str,
    has_key: impl Fn(Provider) -> bool,
    has_cli: impl Fn(Provider) -> bool,
) -> Vec<(Provider, &'static str)> {
    let mut ordered = Vec::with_capacity(Provider::ALL.len());
    ordered.push(requested);
    for p in Provider::ALL {
        if p != requested {
            ordered.push(p);
        }
    }

    let mode = backend.trim().to_ascii_lowercase();
    let mut out = Vec::new();

    match mode.as_str() {
        "api" => {
            for p in ordered {
                if has_key(p) {
                    out.push((p, "api"));
                }
            }
            if out.is_empty() {
                out.push((requested, "api"));
            }
        }
        "cli" => {
            for p in ordered {
                if has_cli(p) {
                    out.push((p, "cli"));
                }
            }
        }
        _ => {
            for p in ordered {
                if has_key(p) {
                    out.push((p, "api"));
                } else if has_cli(p) {
                    out.push((p, "cli"));
                }
            }
        }
    }

    out
}

pub fn console_url_for_provider(provider: &str) -> &'static str {
    match provider.trim().to_ascii_lowercase().as_str() {
        "openai" | "chatgpt" | "gpt" => "https://platform.openai.com/api-keys",
        "gemini" | "google" => "https://aistudio.google.com/app/apikey",
        _ => "https://console.anthropic.com/settings/keys",
    }
}

fn should_try_fallback(err: &str) -> bool {
    if err.eq_ignore_ascii_case("cancelled") {
        return false;
    }
    providers::is_provider_switchable(err)
        || matches!(
            err,
            "api-key-missing" | "cli-auth-required" | "cli-not-found" | "cli-timeout"
        )
}

pub fn warm_up(_app: AppHandle) {
    std::thread::spawn(|| {
        let _ = providers::http_client();
        for p in Provider::ALL {
            let _ = cli::detect_cli(p);
        }
    });
}

#[tauri::command]
pub async fn ai_ask(
    app: AppHandle,
    request_id: String,
    messages: Vec<AiMessage>,
    provider: Option<String>,
    model: Option<String>,
) -> Result<String, String> {
    let _trim_guard = TrimOnDrop;

    let backend_mode = normalize_backend_mode(get_setting_str(&app, "fern-ai-backend"));
    let requested = provider
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(Provider::from_setting)
        .unwrap_or_else(|| {
            Provider::from_setting(&get_setting_str(&app, "fern-ai-provider").unwrap_or_default())
        });

    let candidates = provider_candidates(
        requested,
        &backend_mode,
        |p| secrets::has_key(p.id()),
        |p| cli::detect_cli(p).installed,
    );

    if candidates.is_empty() {
        return if backend_mode == "cli" {
            Err("cli-not-found".to_string())
        } else {
            Err("ai-not-configured".to_string())
        };
    }

    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let _req_guard = RequestGuard::register(request_id.clone(), cancel_tx);

    let mut last_err: Option<String> = None;

    for (idx, &(candidate, mode)) in candidates.iter().enumerate() {
        if *cancel_rx.borrow() {
            return Err("cancelled".to_string());
        }

        if candidate != requested {
            let _ = app.emit("ai-provider-used", candidate.id());
        }

        let attempt = match mode {
            "api" => {
                if let Some(key) = secrets::get_key(candidate.id()) {
                    let effective_model = if candidate == requested {
                        model
                            .as_deref()
                            .map(str::trim)
                            .filter(|m| !m.is_empty())
                            .map(str::to_string)
                    } else {
                        None
                    }
                    .or_else(|| {
                        get_setting_str(&app, candidate.model_setting())
                            .map(|m| m.trim().to_string())
                            .filter(|m| !m.is_empty())
                    })
                    .unwrap_or_else(|| candidate.default_model().to_string());

                    providers::ask_api(
                        &app,
                        candidate,
                        key,
                        &effective_model,
                        &messages,
                        &request_id,
                        cancel_rx.clone(),
                    )
                    .await
                } else {
                    Err("api-key-missing".to_string())
                }
            }
            "cli" => {
                cli::ask_cli(&app, candidate, &messages, &request_id, cancel_rx.clone()).await
            }
            _ => Err("ai-not-configured".to_string()),
        };

        match attempt {
            Ok(reply) => return Ok(reply),
            Err(err) => {
                if err == "cancelled" || *cancel_rx.borrow() {
                    return Err("cancelled".to_string());
                }
                let has_more = idx + 1 < candidates.len();
                if has_more && should_try_fallback(&err) {
                    last_err = Some(err);
                    continue;
                }
                return Err(err);
            }
        }
    }

    Err(last_err.unwrap_or_else(|| "ai-not-configured".to_string()))
}

#[tauri::command]
pub fn ai_cancel(request_id: String) -> Result<(), String> {
    let guard = active_requests().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(tx) = guard.get(&request_id) {
        let _ = tx.send(true);
    }
    Ok(())
}

#[tauri::command]
pub fn ai_get_status(app: AppHandle, refresh_cli: Option<bool>) -> AiStatusSnapshot {
    if refresh_cli.unwrap_or(false) {
        cli::refresh_cli_cache();
    }

    let backend_mode = normalize_backend_mode(get_setting_str(&app, "fern-ai-backend"));
    let default_provider =
        Provider::from_setting(&get_setting_str(&app, "fern-ai-provider").unwrap_or_default())
            .id()
            .to_string();

    let providers = Provider::ALL
        .iter()
        .map(|&p| {
            let has_api_key = secrets::has_key(p.id());
            let cli_installed = cli::detect_cli(p).installed;
            let active_backend = match backend_mode.as_str() {
                "api" => {
                    if has_api_key {
                        "api"
                    } else {
                        "none"
                    }
                }
                "cli" => {
                    if cli_installed {
                        "cli"
                    } else {
                        "none"
                    }
                }
                _ => {
                    if has_api_key {
                        "api"
                    } else if cli_installed {
                        "cli"
                    } else {
                        "none"
                    }
                }
            };

            AiProviderStatus {
                id: p.id().to_string(),
                label: p.label().to_string(),
                has_api_key,
                cli_installed,
                active_backend: active_backend.to_string(),
            }
        })
        .collect();

    AiStatusSnapshot {
        default_provider,
        backend_mode,
        providers,
    }
}

#[tauri::command]
pub fn ai_set_key(
    provider: String,
    key: String,
    session_only: Option<bool>,
) -> Result<(), String> {
    let p = Provider::from_setting(&provider);
    secrets::set_key(p.id(), &key, session_only.unwrap_or(false))
}

#[tauri::command]
pub fn ai_delete_key(provider: String) -> Result<(), String> {
    let p = Provider::from_setting(&provider);
    secrets::delete_key(p.id())
}

#[tauri::command]
pub async fn ai_capture_screen(app: AppHandle, mode: Option<String>) -> Result<String, String> {
    let effective_mode = mode
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "screen".to_string());

    tokio::task::spawn_blocking(move || capture::capture_for_ai(&app, &effective_mode))
        .await
        .map_err(|e| format!("Screen capture task failed: {}", e))?
}

#[tauri::command]
pub fn ai_warm(app: AppHandle) {
    warm_up(app);
}

#[tauri::command]
pub fn ai_open_console(provider: String) -> Result<(), String> {
    let url = console_url_for_provider(&provider);
    let wide_open: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    let wide_url: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let res = unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::PCWSTR(wide_open.as_ptr()),
            windows::core::PCWSTR(wide_url.as_ptr()),
            None,
            None,
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    if res.0 as usize <= 32 {
        Err(format!("Failed to open browser (error {})", res.0 as usize))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_candidates_auto_prefers_api_then_cli_then_fallback() {
        // Requested has both key and CLI -> prefers API; other providers use API if key else CLI.
        let c1 = provider_candidates(
            Provider::Claude,
            "auto",
            |p| matches!(p, Provider::Claude | Provider::Gemini),
            |p| matches!(p, Provider::Claude | Provider::OpenAI),
        );
        assert_eq!(
            c1,
            vec![
                (Provider::Claude, "api"),
                (Provider::OpenAI, "cli"),
                (Provider::Gemini, "api"),
            ]
        );

        // Requested has only CLI -> uses CLI first, then falls back to other providers' API.
        let c2 = provider_candidates(
            Provider::OpenAI,
            "auto",
            |p| p == Provider::Claude,
            |p| p == Provider::OpenAI,
        );
        assert_eq!(
            c2,
            vec![(Provider::OpenAI, "cli"), (Provider::Claude, "api")]
        );

        // Requested has neither -> falls back to other providers that have key or CLI.
        let c3 = provider_candidates(
            Provider::Gemini,
            "auto",
            |p| p == Provider::OpenAI,
            |p| p == Provider::Claude,
        );
        assert_eq!(
            c3,
            vec![(Provider::Claude, "cli"), (Provider::OpenAI, "api")]
        );

        // Nothing configured -> empty list (triggers ai-not-configured).
        let c4 = provider_candidates(Provider::Claude, "auto", |_| false, |_| false);
        assert!(c4.is_empty());
    }

    #[test]
    fn provider_candidates_api_and_cli_strict_modes() {
        // Strict API mode ignores CLI-only providers and keeps requested first when it has a key.
        let api_with_keys = provider_candidates(
            Provider::OpenAI,
            "api",
            |p| matches!(p, Provider::OpenAI | Provider::Gemini),
            |_| true,
        );
        assert_eq!(
            api_with_keys,
            vec![(Provider::OpenAI, "api"), (Provider::Gemini, "api")]
        );

        // Strict API mode with zero keys still returns requested so caller gets api-key-missing.
        let api_no_keys = provider_candidates(Provider::Gemini, "api", |_| false, |_| true);
        assert_eq!(api_no_keys, vec![(Provider::Gemini, "api")]);

        // Strict CLI mode ignores API keys and orders requested first when installed.
        let cli_installed = provider_candidates(
            Provider::Gemini,
            "cli",
            |_| true,
            |p| matches!(p, Provider::Gemini | Provider::Claude),
        );
        assert_eq!(
            cli_installed,
            vec![(Provider::Gemini, "cli"), (Provider::Claude, "cli")]
        );

        // Strict CLI mode with no CLIs installed returns empty (triggers cli-not-found).
        let cli_none = provider_candidates(Provider::Claude, "cli", |_| true, |_| false);
        assert!(cli_none.is_empty());
    }

    #[test]
    fn console_url_allowlist_only_returns_known_urls() {
        const ALLOWED: [&str; 3] = [
            "https://console.anthropic.com/settings/keys",
            "https://platform.openai.com/api-keys",
            "https://aistudio.google.com/app/apikey",
        ];

        assert_eq!(
            console_url_for_provider("openai"),
            "https://platform.openai.com/api-keys"
        );
        assert_eq!(
            console_url_for_provider("ChatGPT"),
            "https://platform.openai.com/api-keys"
        );
        assert_eq!(
            console_url_for_provider("gpt"),
            "https://platform.openai.com/api-keys"
        );
        assert_eq!(
            console_url_for_provider("gemini"),
            "https://aistudio.google.com/app/apikey"
        );
        assert_eq!(
            console_url_for_provider("Google"),
            "https://aistudio.google.com/app/apikey"
        );
        assert_eq!(
            console_url_for_provider("claude"),
            "https://console.anthropic.com/settings/keys"
        );

        // Arbitrary or malicious inputs must only resolve to the hardcoded allowlist.
        for hostile in [
            "",
            "https://evil.example.com",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "cmd.exe /c calc",
        ] {
            let url = console_url_for_provider(hostile);
            assert!(
                ALLOWED.contains(&url),
                "unexpected URL returned for {:?}: {}",
                hostile,
                url
            );
            assert_eq!(url, "https://console.anthropic.com/settings/keys");
        }
    }
}
