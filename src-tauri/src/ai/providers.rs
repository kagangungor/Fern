use std::future::Future;
use std::sync::OnceLock;
use std::task::Poll;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde_json::{json, Value};
use tauri::Emitter;

use crate::ai::secrets::SecretString;

pub const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const HTTP_START_TIMEOUT: Duration = Duration::from_secs(25);
pub const HTTP_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

const MAX_OUTPUT_TOKENS: u32 = 4096;
const MAX_CONTINUATIONS: usize = 2;
const MAX_CONTEXT_MESSAGES: usize = 20;
const MAX_MESSAGE_CHARS: usize = 8000;
const MAX_IMAGES_PER_MESSAGE: usize = 4;
const MAX_IMAGE_BASE64_BYTES: usize = 10 * 1024 * 1024;
const MAX_ERROR_BODY_CHARS: usize = 400;

const CONTINUE_PROMPT: &str = "Your previous answer was cut off. Continue exactly where it stopped. Do not repeat anything and do not start over.";

static HTTP_CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Provider {
    Claude,
    OpenAI,
    Gemini,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Claude, Provider::OpenAI, Provider::Gemini];

    pub fn from_setting(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "openai" | "chatgpt" | "gpt" => Self::OpenAI,
            "gemini" | "google" => Self::Gemini,
            _ => Self::Claude,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::OpenAI => "openai",
            Self::Gemini => "gemini",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::OpenAI => "ChatGPT",
            Self::Gemini => "Gemini",
        }
    }

    pub fn default_model(self) -> &'static str {
        match self {
            Self::Claude => "claude-sonnet-4-5",
            Self::OpenAI => "gpt-4o-mini",
            Self::Gemini => "gemini-flash-latest",
        }
    }

    pub fn model_setting(self) -> &'static str {
        match self {
            Self::Claude => "fern-ai-model",
            Self::OpenAI => "fern-ai-model-openai",
            Self::Gemini => "fern-ai-model-gemini",
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct AiMessage {
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub images: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct InlineImage {
    pub mime: String,
    pub data_base64: String,
}

#[derive(Clone, serde::Serialize)]
pub struct AiStreamEvent {
    pub id: String,
    pub delta: String,
}

pub fn http_client() -> Result<&'static reqwest::Client, String> {
    let entry = HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .read_timeout(HTTP_IDLE_TIMEOUT)
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(300))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {}", sanitize_reqwest_error(&e)))
    });
    match entry {
        Ok(client) => Ok(client),
        Err(msg) => Err(msg.clone()),
    }
}

pub fn parse_images(raw: &[String]) -> Vec<InlineImage> {
    let mut out = Vec::new();
    for entry in raw {
        if out.len() >= MAX_IMAGES_PER_MESSAGE {
            break;
        }
        let trimmed = entry.trim();
        let Some(rest) = trimmed.strip_prefix("data:") else {
            continue;
        };
        let Some((mime_raw, data_raw)) = rest.split_once(";base64,") else {
            continue;
        };
        let mime_lower = mime_raw.trim().to_ascii_lowercase();
        let mime = match mime_lower.as_str() {
            "image/png" => "image/png",
            "image/jpeg" | "image/jpg" => "image/jpeg",
            "image/webp" => "image/webp",
            "image/gif" => "image/gif",
            _ => continue,
        };
        let data: String = data_raw.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        if data.is_empty() || data.len() > MAX_IMAGE_BASE64_BYTES {
            continue;
        }
        if STANDARD.decode(&data).is_err() {
            continue;
        }
        out.push(InlineImage {
            mime: mime.to_string(),
            data_base64: data,
        });
    }
    out
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => s[..idx].to_string(),
        None => s.to_string(),
    }
}

pub fn trim_context(messages: &[AiMessage]) -> Vec<AiMessage> {
    let start = messages.len().saturating_sub(MAX_CONTEXT_MESSAGES);
    let slice = &messages[start..];

    let last_user_idx = slice
        .iter()
        .rposition(|m| m.role.eq_ignore_ascii_case("user"));

    slice
        .iter()
        .enumerate()
        .map(|(idx, m)| AiMessage {
            role: m.role.clone(),
            text: truncate_chars(&m.text, MAX_MESSAGE_CHARS),
            images: if Some(idx) == last_user_idx {
                m.images.clone()
            } else {
                Vec::new()
            },
        })
        .collect()
}

pub fn is_provider_switchable(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    if lower.contains("cancelled") || lower.contains("canceled") || lower.contains("empty message") {
        return false;
    }
    const SWITCHABLE_MARKERS: &[&str] = &[
        "quota",
        "rate limit",
        "rate_limit",
        "too many requests",
        "429",
        "503",
        "502",
        "504",
        "529",
        "overloaded",
        "capacity",
        "timed out",
        "timeout",
        "401",
        "403",
        "unauthorized",
        "forbidden",
        "invalid api key",
        "invalid_api_key",
        "authentication",
        "credit",
        "billing",
        "insufficient",
        "network",
        "connection",
        "connect error",
        "dns",
        "reset by peer",
        "broken pipe",
        "tls",
        "unreachable",
    ];
    SWITCHABLE_MARKERS.iter().any(|m| lower.contains(m))
}

fn sanitize_reqwest_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        return "Request timed out".to_string();
    }
    if err.is_connect() {
        return "Network connection error".to_string();
    }
    if let Some(status) = err.status() {
        return format!("HTTP status {}", status.as_u16());
    }
    if err.is_body() || err.is_decode() {
        return "Network stream interrupted".to_string();
    }
    "Network request failed".to_string()
}

fn scrub_secret(text: &str, secret: &SecretString) -> String {
    let raw = secret.as_str();
    if raw.is_empty() {
        return text.to_string();
    }
    text.replace(raw, "[REDACTED]")
}

fn extract_http_error_detail(body: &str, secret: &SecretString) -> String {
    let parsed_msg = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        v.pointer("/error/message")
            .and_then(|m| m.as_str())
            .or_else(|| v.pointer("/message").and_then(|m| m.as_str()))
            .map(|s| s.to_string())
    });

    let raw = parsed_msg
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| body.trim().to_string());

    let scrubbed = scrub_secret(&raw, secret);
    truncate_chars(&scrubbed, MAX_ERROR_BODY_CHARS)
}

async fn wait_for_cancel(cancel: &mut tokio::sync::watch::Receiver<bool>) -> String {
    if *cancel.borrow() {
        return "cancelled".to_string();
    }
    loop {
        match cancel.changed().await {
            Ok(()) => {
                if *cancel.borrow() {
                    return "cancelled".to_string();
                }
            }
            Err(_) => std::future::pending::<()>().await,
        }
    }
}

async fn race_cancel<T, F>(
    cancel: &mut tokio::sync::watch::Receiver<bool>,
    fut: F,
) -> Result<T, String>
where
    F: Future<Output = Result<T, String>>,
{
    if *cancel.borrow() {
        return Err("cancelled".to_string());
    }
    let mut fut = std::pin::pin!(fut);
    let mut cancel_fut = std::pin::pin!(wait_for_cancel(cancel));

    std::future::poll_fn(|cx| {
        if let Poll::Ready(err) = cancel_fut.as_mut().poll(cx) {
            return Poll::Ready(Err(err));
        }
        fut.as_mut().poll(cx)
    })
    .await
}

async fn send_cancellable(
    request: reqwest::RequestBuilder,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<reqwest::Response, String> {
    race_cancel(cancel, async move {
        match tokio::time::timeout(HTTP_START_TIMEOUT, request.send()).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(e)) => Err(sanitize_reqwest_error(&e)),
            Err(_) => Err(format!(
                "Request timed out after {}s waiting for response",
                HTTP_START_TIMEOUT.as_secs()
            )),
        }
    })
    .await
}

async fn read_sse_cancellable<F>(
    mut response: reqwest::Response,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
    mut on_data: F,
) -> Result<(), String>
where
    F: FnMut(&str),
{
    let mut buf: Vec<u8> = Vec::with_capacity(4096);

    loop {
        if *cancel.borrow() {
            return Err("cancelled".to_string());
        }

        let chunk_opt = race_cancel(cancel, async {
            response
                .chunk()
                .await
                .map_err(|e| format!("Stream read error: {}", sanitize_reqwest_error(&e)))
        })
        .await?;

        let Some(chunk) = chunk_opt else {
            break;
        };

        buf.extend_from_slice(&chunk);

        while let Some(pos) = buf.iter().position(|&b| b == b'\n') {
            let mut line_bytes = buf.drain(..=pos).collect::<Vec<u8>>();
            if line_bytes.last() == Some(&b'\n') {
                line_bytes.pop();
            }
            if line_bytes.last() == Some(&b'\r') {
                line_bytes.pop();
            }

            if process_sse_line(&line_bytes, &mut on_data) {
                return Ok(());
            }
        }
    }

    if !buf.is_empty() {
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        let _ = process_sse_line(&buf, &mut on_data);
    }

    Ok(())
}

/// Returns `true` if the SSE stream signaled `[DONE]`.
fn process_sse_line<F: FnMut(&str)>(line_bytes: &[u8], on_data: &mut F) -> bool {
    let line = String::from_utf8_lossy(line_bytes);
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with(':') {
        return false;
    }
    if let Some(rest) = trimmed.strip_prefix("data:") {
        let payload = rest.trim_start();
        if payload == "[DONE]" {
            return true;
        }
        if !payload.is_empty() {
            on_data(payload);
        }
    }
    false
}

struct TurnResult {
    text: String,
    truncated: bool,
}

fn build_claude_body(model: &str, messages: &[AiMessage]) -> Result<String, String> {
    let formatted: Vec<Value> = messages
        .iter()
        .map(|m| {
            let role = if m.role.eq_ignore_ascii_case("assistant") {
                "assistant"
            } else {
                "user"
            };
            let images = parse_images(&m.images);
            let content = if images.is_empty() {
                Value::String(m.text.clone())
            } else {
                let mut blocks: Vec<Value> = images
                    .into_iter()
                    .map(|img| {
                        json!({
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": img.mime,
                                "data": img.data_base64,
                            }
                        })
                    })
                    .collect();
                if !m.text.is_empty() {
                    blocks.push(json!({
                        "type": "text",
                        "text": m.text,
                    }));
                }
                Value::Array(blocks)
            };
            json!({
                "role": role,
                "content": content,
            })
        })
        .collect();

    serde_json::to_string(&json!({
        "model": model,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "stream": true,
        "messages": formatted,
    }))
    .map_err(|e| format!("Failed to serialize Claude request: {}", e))
}

fn build_openai_body(model: &str, messages: &[AiMessage]) -> Result<String, String> {
    let formatted: Vec<Value> = messages
        .iter()
        .map(|m| {
            let role = if m.role.eq_ignore_ascii_case("assistant") {
                "assistant"
            } else if m.role.eq_ignore_ascii_case("system") {
                "system"
            } else {
                "user"
            };
            let images = parse_images(&m.images);
            let content = if images.is_empty() {
                Value::String(m.text.clone())
            } else {
                let mut blocks = Vec::new();
                if !m.text.is_empty() {
                    blocks.push(json!({
                        "type": "text",
                        "text": m.text,
                    }));
                }
                for img in images {
                    blocks.push(json!({
                        "type": "image_url",
                        "image_url": {
                            "url": format!("data:{};base64,{}", img.mime, img.data_base64),
                        }
                    }));
                }
                Value::Array(blocks)
            };
            json!({
                "role": role,
                "content": content,
            })
        })
        .collect();

    serde_json::to_string(&json!({
        "model": model,
        "max_tokens": MAX_OUTPUT_TOKENS,
        "stream": true,
        "messages": formatted,
    }))
    .map_err(|e| format!("Failed to serialize OpenAI request: {}", e))
}

fn build_gemini_body(messages: &[AiMessage]) -> Result<String, String> {
    let contents: Vec<Value> = messages
        .iter()
        .map(|m| {
            let role = if m.role.eq_ignore_ascii_case("assistant")
                || m.role.eq_ignore_ascii_case("model")
            {
                "model"
            } else {
                "user"
            };
            let images = parse_images(&m.images);
            let mut parts = Vec::new();
            if !m.text.is_empty() {
                parts.push(json!({ "text": m.text }));
            }
            for img in images {
                parts.push(json!({
                    "inline_data": {
                        "mime_type": img.mime,
                        "data": img.data_base64,
                    }
                }));
            }
            if parts.is_empty() {
                parts.push(json!({ "text": "" }));
            }
            json!({
                "role": role,
                "parts": parts,
            })
        })
        .collect();

    serde_json::to_string(&json!({
        "contents": contents,
        "generationConfig": {
            "maxOutputTokens": MAX_OUTPUT_TOKENS,
        }
    }))
    .map_err(|e| format!("Failed to serialize Gemini request: {}", e))
}

fn sanitize_gemini_model(model: &str) -> String {
    let cleaned: String = model
        .trim()
        .chars()
        .filter(|&c| !matches!(c, '/' | '?' | '#' | '\\') && !c.is_whitespace())
        .collect();
    if cleaned.is_empty() {
        Provider::Gemini.default_model().to_string()
    } else {
        cleaned
    }
}

fn emit_delta(app: &tauri::AppHandle, request_id: &str, delta: &str) {
    if delta.is_empty() {
        return;
    }
    let _ = app.emit(
        "ai-stream",
        AiStreamEvent {
            id: request_id.to_string(),
            delta: delta.to_string(),
        },
    );
}

async fn stream_turn(
    app: &tauri::AppHandle,
    provider: Provider,
    key: &SecretString,
    model: &str,
    messages: &[AiMessage],
    request_id: &str,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<TurnResult, String> {
    let client = http_client()?;

    let request = match provider {
        Provider::Claude => {
            let body = build_claude_body(model, messages)?;
            client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", key.as_str())
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .body(body)
        }
        Provider::OpenAI => {
            let body = build_openai_body(model, messages)?;
            client
                .post("https://api.openai.com/v1/chat/completions")
                .header("authorization", format!("Bearer {}", key.as_str()))
                .header("content-type", "application/json")
                .body(body)
        }
        Provider::Gemini => {
            let body = build_gemini_body(messages)?;
            let safe_model = sanitize_gemini_model(model);
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
                safe_model
            );
            client
                .post(&url)
                .header("x-goog-api-key", key.as_str())
                .header("content-type", "application/json")
                .body(body)
        }
    };

    let response = send_cancellable(request, cancel).await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let detail = extract_http_error_detail(&body, key);
        return if detail.is_empty() {
            Err(format!("{} API error (HTTP {})", provider.label(), status.as_u16()))
        } else {
            Err(format!(
                "{} API error (HTTP {}): {}",
                provider.label(),
                status.as_u16(),
                detail
            ))
        };
    }

    let mut turn_text = String::new();
    let mut truncated = false;
    let mut stream_err: Option<String> = None;

    read_sse_cancellable(response, cancel, |payload| {
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return;
        };

        match provider {
            Provider::Claude => {
                match v.get("type").and_then(|t| t.as_str()) {
                    Some("content_block_delta") => {
                        if let Some(delta) = v.pointer("/delta/text").and_then(|t| t.as_str()) {
                            if !delta.is_empty() {
                                turn_text.push_str(delta);
                                emit_delta(app, request_id, delta);
                            }
                        }
                    }
                    Some("message_delta") => {
                        if v.pointer("/delta/stop_reason").and_then(|s| s.as_str())
                            == Some("max_tokens")
                        {
                            truncated = true;
                        }
                    }
                    Some("error") => {
                        let msg = v
                            .pointer("/error/message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("Claude stream error");
                        stream_err = Some(scrub_secret(msg, key));
                    }
                    _ => {}
                }
            }
            Provider::OpenAI => {
                if let Some(err_msg) = v.pointer("/error/message").and_then(|m| m.as_str()) {
                    stream_err = Some(scrub_secret(err_msg, key));
                    return;
                }
                if let Some(choice) = v.get("choices").and_then(|c| c.get(0)) {
                    if let Some(delta) = choice.pointer("/delta/content").and_then(|t| t.as_str()) {
                        if !delta.is_empty() {
                            turn_text.push_str(delta);
                            emit_delta(app, request_id, delta);
                        }
                    }
                    if choice.get("finish_reason").and_then(|f| f.as_str()) == Some("length") {
                        truncated = true;
                    }
                }
            }
            Provider::Gemini => {
                if let Some(err_msg) = v.pointer("/error/message").and_then(|m| m.as_str()) {
                    stream_err = Some(scrub_secret(err_msg, key));
                    return;
                }
                if let Some(cand) = v.get("candidates").and_then(|c| c.get(0)) {
                    if let Some(parts) = cand.pointer("/content/parts").and_then(|p| p.as_array()) {
                        for part in parts {
                            if let Some(delta) = part.get("text").and_then(|t| t.as_str()) {
                                if !delta.is_empty() {
                                    turn_text.push_str(delta);
                                    emit_delta(app, request_id, delta);
                                }
                            }
                        }
                    }
                    if cand.get("finishReason").and_then(|f| f.as_str()) == Some("MAX_TOKENS") {
                        truncated = true;
                    }
                }
            }
        }
    })
    .await?;

    if let Some(err) = stream_err {
        return Err(format!("{} API error: {}", provider.label(), err));
    }

    Ok(TurnResult {
        text: turn_text,
        truncated,
    })
}

pub async fn ask_api(
    app: &tauri::AppHandle,
    provider: Provider,
    key: SecretString,
    model: &str,
    messages: &[AiMessage],
    request_id: &str,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<String, String> {
    if key.as_str().trim().is_empty() {
        return Err(format!("{} API key is missing (401)", provider.label()));
    }

    let mut working_messages: Vec<AiMessage> = trim_context(messages)
        .into_iter()
        .filter(|m| !m.text.trim().is_empty() || !parse_images(&m.images).is_empty())
        .collect();

    if working_messages.is_empty() {
        return Err("Cannot send an empty message".to_string());
    }

    let effective_model = if model.trim().is_empty() {
        provider.default_model()
    } else {
        model.trim()
    };

    let mut full_reply = String::new();

    for continuation in 0..=MAX_CONTINUATIONS {
        let turn = stream_turn(
            app,
            provider,
            &key,
            effective_model,
            &working_messages,
            request_id,
            &mut cancel,
        )
        .await?;

        full_reply.push_str(&turn.text);

        if !turn.truncated || continuation == MAX_CONTINUATIONS || turn.text.is_empty() {
            break;
        }

        working_messages.push(AiMessage {
            role: "assistant".to_string(),
            text: turn.text,
            images: Vec::new(),
        });
        working_messages.push(AiMessage {
            role: "user".to_string(),
            text: CONTINUE_PROMPT.to_string(),
            images: Vec::new(),
        });
    }

    Ok(full_reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn sse_reader_reassembles_split_utf8_and_lines() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test listener");
        let addr = listener.local_addr().expect("listener addr");

        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept connection");
            let mut req_buf = [0u8; 1024];
            let _ = stream.read(&mut req_buf);

            let header = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/event-stream\r\n",
                "Cache-Control: no-cache\r\n",
                "Connection: close\r\n",
                "\r\n"
            );
            stream.write_all(header.as_bytes()).expect("write header");
            stream.flush().expect("flush header");

            // Split the 2-byte UTF-8 sequence for 'ü' (0xC3 0xBC) across two TCP writes.
            let chunk1: &[u8] = b"data: {\"text\":\"gr\xC3";
            let chunk2: &[u8] = b"\xBC\xC3\x9Fen\"}\n\ndata: {\"text\":\"ok\"}\r\ndata: [DONE]\n\n";

            stream.write_all(chunk1).expect("write chunk1");
            stream.flush().expect("flush chunk1");
            thread::sleep(Duration::from_millis(25));

            stream.write_all(chunk2).expect("write chunk2");
            stream.flush().expect("flush chunk2");
        });

        let url = format!("http://{}/sse", addr);
        let payloads = tauri::async_runtime::block_on(async move {
            let client = http_client().expect("http_client");
            let (_tx, mut rx) = tokio::sync::watch::channel(false);
            let resp = send_cancellable(client.get(&url), &mut rx)
                .await
                .expect("send_cancellable");

            let mut received = Vec::new();
            read_sse_cancellable(resp, &mut rx, |data| {
                received.push(data.to_string());
            })
            .await
            .expect("read_sse_cancellable");
            received
        });

        server.join().expect("server thread");

        assert_eq!(payloads.len(), 2);
        assert_eq!(payloads[0], "{\"text\":\"grüßen\"}");
        assert_eq!(payloads[1], "{\"text\":\"ok\"}");
    }

    #[test]
    fn timeout_bounds_are_sane() {
        assert!(HTTP_CONNECT_TIMEOUT >= Duration::from_secs(5));
        assert!(HTTP_CONNECT_TIMEOUT < HTTP_START_TIMEOUT);
        assert!(HTTP_START_TIMEOUT < HTTP_IDLE_TIMEOUT);
        assert!(HTTP_IDLE_TIMEOUT <= Duration::from_secs(300));
    }

    #[test]
    fn trim_context_keeps_only_latest_user_images() {
        let mut msgs = Vec::new();
        for i in 0..25 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            msgs.push(AiMessage {
                role: role.to_string(),
                text: format!("msg-{}", i),
                images: if role == "user" {
                    vec![format!("data:image/png;base64,IMG{}", i)]
                } else {
                    Vec::new()
                },
            });
        }
        // Make the last user message carry > MAX_MESSAGE_CHARS multi-byte text
        let long_text = "ü".repeat(MAX_MESSAGE_CHARS + 50);
        // Index 24 is the last user message (even)
        msgs[24].text = long_text;
        // Add a trailing assistant message so the last user message is not the very last element
        msgs.push(AiMessage {
            role: "assistant".to_string(),
            text: "final reply".to_string(),
            images: Vec::new(),
        });

        let trimmed = trim_context(&msgs);
        assert_eq!(trimmed.len(), MAX_CONTEXT_MESSAGES);

        // The latest user message in `trimmed` is at index len - 2
        let last_user_pos = trimmed.len() - 2;
        assert_eq!(trimmed[last_user_pos].role, "user");
        assert_eq!(trimmed[last_user_pos].images, vec!["data:image/png;base64,IMG24"]);
        assert_eq!(trimmed[last_user_pos].text.chars().count(), MAX_MESSAGE_CHARS);

        for (idx, m) in trimmed.iter().enumerate() {
            if idx != last_user_pos {
                assert!(
                    m.images.is_empty(),
                    "message at index {} should have images stripped",
                    idx
                );
            }
        }
    }

    #[test]
    fn parse_images_validates_and_caps() {
        let valid_png = format!("data:image/png;base64,{}", STANDARD.encode(b"fake-png"));
        let valid_jpg = format!("data:image/jpg;base64,{}", STANDARD.encode(b"fake-jpg"));
        let raw = vec![
            "not-a-data-url".to_string(),
            "data:text/plain;base64,SGVsbG8=".to_string(),
            "data:image/png;base64,%%%invalid%%%".to_string(),
            valid_png.clone(),
            valid_jpg,
            valid_png.clone(),
            valid_png.clone(),
            valid_png,
        ];
        let parsed = parse_images(&raw);
        assert_eq!(parsed.len(), MAX_IMAGES_PER_MESSAGE);
        assert_eq!(parsed[0].mime, "image/png");
        assert_eq!(parsed[1].mime, "image/jpeg");
    }

    #[test]
    fn provider_switchable_classification() {
        assert!(is_provider_switchable("Claude API error (HTTP 429): rate limit"));
        assert!(is_provider_switchable("OpenAI API error (HTTP 503): overloaded"));
        assert!(is_provider_switchable("Request timed out after 25s"));
        assert!(is_provider_switchable("Gemini API error (HTTP 401): invalid api key"));
        assert!(is_provider_switchable("Network connection error"));
        assert!(!is_provider_switchable("cancelled"));
        assert!(!is_provider_switchable("Cannot send an empty message"));
    }

    #[test]
    fn provider_metadata_and_secret_scrubbing() {
        assert_eq!(Provider::ALL.len(), 3);
        assert_eq!(Provider::from_setting("openai"), Provider::OpenAI);
        assert_eq!(Provider::from_setting("gemini"), Provider::Gemini);
        assert_eq!(Provider::from_setting("claude"), Provider::Claude);
        assert_eq!(Provider::Claude.id(), "claude");
        assert_eq!(Provider::OpenAI.label(), "ChatGPT");
        assert_eq!(Provider::Gemini.default_model(), "gemini-flash-latest");
        assert_eq!(Provider::Claude.model_setting(), "fern-ai-model");
        assert_eq!(Provider::OpenAI.model_setting(), "fern-ai-model-openai");
        assert_eq!(Provider::Gemini.model_setting(), "fern-ai-model-gemini");

        let secret = SecretString::new("sk-ant-super-secret-123".to_string());
        let body = r#"{"error":{"message":"Invalid key sk-ant-super-secret-123 provided"}}"#;
        let detail = extract_http_error_detail(body, &secret);
        assert!(!detail.contains("sk-ant-super-secret-123"));
        assert!(detail.contains("[REDACTED]"));
    }

    #[test]
    fn gemini_model_sanitizes_path_query_and_fragment_chars() {
        assert_eq!(
            sanitize_gemini_model("gemini-2.5-flash"),
            "gemini-2.5-flash"
        );
        let sanitized = sanitize_gemini_model("gemini-flash/../evil?key=leak#frag");
        assert!(!sanitized.contains('/'));
        assert!(!sanitized.contains('?'));
        assert!(!sanitized.contains('#'));
        assert_eq!(sanitize_gemini_model(" /?# "), Provider::Gemini.default_model());
    }
}
