use std::collections::HashMap;
use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::task::Poll;
use std::thread;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use tauri::Emitter;

use crate::ai::providers::{parse_images, trim_context, AiMessage, AiStreamEvent, Provider};

const CLI_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_ERROR_CHARS: usize = 300;
const STDOUT_CHUNK_BYTES: usize = 1024;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static CLI_CACHE: OnceLock<Mutex<HashMap<Provider, CliInfo>>> = OnceLock::new();

#[derive(Clone, Debug, serde::Serialize)]
pub struct CliInfo {
    pub installed: bool,
    pub path: Option<String>,
}

fn cli_cache() -> &'static Mutex<HashMap<Provider, CliInfo>> {
    CLI_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cli_binary_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "claude",
        Provider::Gemini => "gemini",
        Provider::OpenAI => "codex",
    }
}

fn candidate_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if !dir.as_os_str().is_empty() {
                dirs.push(dir);
            }
        }
    }

    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join("npm"));
    }

    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local_appdata);
        dirs.push(local.join("Programs"));
        dirs.push(local.join("pnpm"));
    }

    if let Some(userprofile) = std::env::var_os("USERPROFILE") {
        let home = PathBuf::from(userprofile);
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".claude").join("local"));
        dirs.push(home.join(".bun").join("bin"));
        dirs.push(home.join("scoop").join("shims"));
    }

    dirs
}

fn resolve_cli_binary(name: &str) -> Option<PathBuf> {
    let exe_name = format!("{name}.exe");
    let cmd_name = format!("{name}.cmd");

    for dir in candidate_search_dirs() {
        let exe_path = dir.join(&exe_name);
        if exe_path.is_file() {
            return Some(exe_path);
        }
        let cmd_path = dir.join(&cmd_name);
        if cmd_path.is_file() {
            return Some(cmd_path);
        }
        #[cfg(not(windows))]
        {
            let plain_path = dir.join(name);
            if plain_path.is_file() {
                return Some(plain_path);
            }
        }
    }

    None
}

pub fn detect_cli(provider: Provider) -> CliInfo {
    let cache = cli_cache();
    {
        let guard = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(info) = guard.get(&provider) {
            return info.clone();
        }
    }

    let found = resolve_cli_binary(cli_binary_name(provider));
    let info = CliInfo {
        installed: found.is_some(),
        path: found.map(|p| p.to_string_lossy().into_owned()),
    };

    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    guard.insert(provider, info.clone());
    info
}

pub fn refresh_cli_cache() {
    let cache = cli_cache();
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    guard.clear();
}

struct TempImageGuard(Vec<PathBuf>);

impl TempImageGuard {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn paths(&self) -> &[PathBuf] {
        &self.0
    }

    fn push(&mut self, path: PathBuf) {
        self.0.push(path);
    }
}

impl Drop for TempImageGuard {
    fn drop(&mut self) {
        for p in &self.0 {
            let _ = std::fs::remove_file(p);
        }
    }
}

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        match self.0.as_mut() {
            Some(child) => {
                let status = child.try_wait()?;
                if status.is_some() {
                    self.0 = None;
                }
                Ok(status)
            }
            None => Ok(None),
        }
    }

    fn kill_and_reap(&mut self) {
        if let Some(mut child) = self.0.take() {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let pid = child.id().to_string();
                let _ = Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &pid])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill_and_reap();
    }
}

fn sanitize_request_id(request_id: &str) -> String {
    let cleaned: String = request_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(48)
        .collect();
    if cleaned.is_empty() {
        "req".to_string()
    } else {
        cleaned
    }
}

fn write_temp_images(messages: &[AiMessage], request_id: &str) -> Result<TempImageGuard, String> {
    let mut guard = TempImageGuard::new();

    let Some(last_user) = messages
        .iter()
        .rev()
        .find(|m| m.role.eq_ignore_ascii_case("user"))
    else {
        return Ok(guard);
    };

    let images = parse_images(&last_user.images);
    if images.is_empty() {
        return Ok(guard);
    }

    let safe_id = sanitize_request_id(request_id);
    let temp_dir = std::env::temp_dir();

    for (idx, img) in images.into_iter().enumerate() {
        let bytes = STANDARD
            .decode(&img.data_base64)
            .map_err(|e| format!("Failed to decode attached image: {}", e))?;
        let path = temp_dir.join(format!("fern-ai-{safe_id}-{idx}.jpg"));
        guard.push(path.clone());
        std::fs::write(&path, bytes)
            .map_err(|e| format!("Failed to write temporary image file: {}", e))?;
    }

    Ok(guard)
}

pub fn build_transcript(messages: &[AiMessage], temp_image_paths: &[PathBuf]) -> String {
    let mut out = if messages.len() == 1
        && messages[0].role.eq_ignore_ascii_case("user")
        && temp_image_paths.is_empty()
    {
        return messages[0].text.trim().to_string();
    } else if messages.len() == 1 && messages[0].role.eq_ignore_ascii_case("user") {
        messages[0].text.trim().to_string()
    } else {
        let mut turns = Vec::new();
        for msg in messages {
            let trimmed = msg.text.trim();
            if trimmed.is_empty() {
                continue;
            }
            let speaker = if msg.role.eq_ignore_ascii_case("assistant")
                || msg.role.eq_ignore_ascii_case("model")
            {
                "Assistant"
            } else {
                "User"
            };
            turns.push(format!("{speaker}: {trimmed}"));
        }
        turns.join("\n\n")
    };

    if !temp_image_paths.is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str("Attached image files (inspect if relevant):");
        for path in temp_image_paths {
            out.push_str("\n- ");
            out.push_str(&path.to_string_lossy());
        }
    }

    out
}

fn provider_cli_args(provider: Provider) -> &'static [&'static str] {
    match provider {
        Provider::Claude => &["-p"],
        Provider::Gemini => &["-p", ""],
        Provider::OpenAI => &["exec", "-"],
    }
}

fn is_cmd_script(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
        .unwrap_or(false)
}

fn format_cmd_arg(arg: &str) -> String {
    if arg.is_empty() {
        "\"\"".to_string()
    } else if arg.chars().any(|c| c.is_whitespace() || c == '"') {
        format!("\"{}\"", arg.replace('"', "\\\""))
    } else {
        arg.to_string()
    }
}

fn format_cmd_raw_arg(bin_path: &Path, args: &[&str]) -> String {
    let quoted_bin = format!("\"{}\"", bin_path.to_string_lossy());
    if args.is_empty() {
        format!("\"{quoted_bin}\"")
    } else {
        let args_str = args
            .iter()
            .map(|arg| format_cmd_arg(arg))
            .collect::<Vec<_>>()
            .join(" ");
        format!("\"{quoted_bin} {args_str}\"")
    }
}

fn build_cli_command(provider: Provider, bin_path: &Path) -> Command {
    let args = provider_cli_args(provider);

    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let mut c = if is_cmd_script(bin_path) {
            let mut cmd_exe = Command::new("cmd.exe");
            cmd_exe.args(["/D", "/S", "/C"]);
            cmd_exe.raw_arg(format_cmd_raw_arg(bin_path, args));
            cmd_exe
        } else {
            let mut direct = Command::new(bin_path);
            direct.args(args);
            direct
        };
        c.creation_flags(CREATE_NO_WINDOW);
        c
    };

    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new(bin_path);
        c.args(args);
        c
    };

    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    cmd
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => s[..idx].to_string(),
        None => s.to_string(),
    }
}

fn classify_cli_stderr(provider: Provider, stderr: &str, status_code: Option<i32>) -> String {
    let trimmed = stderr.trim();
    let lower = trimmed.to_ascii_lowercase();
    const AUTH_KEYWORDS: &[&str] = &[
        "login",
        "auth",
        "unauthorized",
        "not logged in",
        "sign in",
    ];
    if AUTH_KEYWORDS.iter().any(|kw| lower.contains(kw)) {
        return "cli-auth-required".to_string();
    }
    if !trimmed.is_empty() {
        return truncate_chars(trimmed, MAX_ERROR_CHARS);
    }
    match status_code {
        Some(code) => format!("{} CLI exited with code {}", provider.label(), code),
        None => format!("{} CLI process failed", provider.label()),
    }
}

fn drain_valid_utf8(pending: &mut Vec<u8>, out: &mut Vec<String>) {
    loop {
        if pending.is_empty() {
            break;
        }
        match std::str::from_utf8(pending) {
            Ok(valid) => {
                if !valid.is_empty() {
                    out.push(valid.to_string());
                }
                pending.clear();
                break;
            }
            Err(err) => {
                let valid_up_to = err.valid_up_to();
                if valid_up_to > 0 {
                    if let Ok(valid) = std::str::from_utf8(&pending[..valid_up_to]) {
                        out.push(valid.to_string());
                    }
                }
                match err.error_len() {
                    Some(bad_len) => {
                        out.push("\u{FFFD}".to_string());
                        pending.drain(..valid_up_to + bad_len);
                    }
                    None => {
                        pending.drain(..valid_up_to);
                        break;
                    }
                }
            }
        }
    }
}

enum RaceOutcome<T> {
    Ready(T),
    Cancelled,
    TimedOut,
}

async fn wait_for_cancel(cancel: &mut tokio::sync::watch::Receiver<bool>) {
    if *cancel.borrow() {
        return;
    }
    loop {
        match cancel.changed().await {
            Ok(()) => {
                if *cancel.borrow() {
                    return;
                }
            }
            Err(_) => std::future::pending::<()>().await,
        }
    }
}

async fn race_with_cancel_and_deadline<T, F>(
    cancel: &mut tokio::sync::watch::Receiver<bool>,
    deadline: tokio::time::Instant,
    fut: F,
) -> RaceOutcome<T>
where
    F: Future<Output = T>,
{
    if *cancel.borrow() {
        return RaceOutcome::Cancelled;
    }
    if tokio::time::Instant::now() >= deadline {
        return RaceOutcome::TimedOut;
    }

    let mut fut = std::pin::pin!(fut);
    let mut cancel_fut = std::pin::pin!(wait_for_cancel(cancel));
    let mut sleep_fut = std::pin::pin!(tokio::time::sleep_until(deadline));

    std::future::poll_fn(|cx| {
        if cancel_fut.as_mut().poll(cx).is_ready() {
            return Poll::Ready(RaceOutcome::Cancelled);
        }
        if sleep_fut.as_mut().poll(cx).is_ready() {
            return Poll::Ready(RaceOutcome::TimedOut);
        }
        match fut.as_mut().poll(cx) {
            Poll::Ready(val) => Poll::Ready(RaceOutcome::Ready(val)),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}

pub async fn ask_cli(
    app: &tauri::AppHandle,
    provider: Provider,
    messages: &[AiMessage],
    request_id: &str,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<String, String> {
    if *cancel.borrow() {
        return Err("cancelled".to_string());
    }

    let info = detect_cli(provider);
    let Some(bin_path_str) = info.path.filter(|_| info.installed) else {
        return Err("cli-not-found".to_string());
    };

    let working_messages: Vec<AiMessage> = trim_context(messages)
        .into_iter()
        .filter(|m| !m.text.trim().is_empty() || !parse_images(&m.images).is_empty())
        .collect();

    if working_messages.is_empty() {
        return Err("Cannot send an empty message".to_string());
    }

    let temp_guard = write_temp_images(&working_messages, request_id)?;
    let transcript = build_transcript(&working_messages, temp_guard.paths());

    let mut cmd = build_cli_command(provider, Path::new(&bin_path_str));
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start {} CLI: {}", provider.label(), e))?;

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let mut child_guard = ChildGuard::new(child);

    if let Some(mut stdin) = stdin {
        let prompt_bytes = transcript.into_bytes();
        thread::spawn(move || {
            let _ = stdin.write_all(&prompt_bytes);
            let _ = stdin.flush();
        });
    }

    let (stdout_tx, mut stdout_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    if let Some(mut stdout) = stdout {
        thread::spawn(move || {
            let mut buf = [0u8; STDOUT_CHUNK_BYTES];
            let mut pending = Vec::new();
            let mut chunks = Vec::new();
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        drain_valid_utf8(&mut pending, &mut chunks);
                        for chunk in chunks.drain(..) {
                            if !chunk.is_empty() && stdout_tx.send(chunk).is_err() {
                                return;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            if !pending.is_empty() {
                let tail = String::from_utf8_lossy(&pending).into_owned();
                if !tail.is_empty() {
                    let _ = stdout_tx.send(tail);
                }
            }
        });
    } else {
        drop(stdout_tx);
    }

    let (stderr_tx, mut stderr_rx) = tokio::sync::oneshot::channel::<String>();
    if let Some(mut stderr) = stderr {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                match stderr.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let remaining = MAX_STDERR_BYTES.saturating_sub(buf.len());
                        if remaining > 0 {
                            buf.extend_from_slice(&chunk[..n.min(remaining)]);
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = stderr_tx.send(String::from_utf8_lossy(&buf).into_owned());
        });
    } else {
        let _ = stderr_tx.send(String::new());
    }

    let deadline = tokio::time::Instant::now() + CLI_TIMEOUT;
    let mut full_reply = String::new();

    loop {
        match race_with_cancel_and_deadline(&mut cancel, deadline, stdout_rx.recv()).await {
            RaceOutcome::Ready(Some(delta)) => {
                if !delta.is_empty() {
                    full_reply.push_str(&delta);
                    let _ = app.emit(
                        "ai-stream",
                        AiStreamEvent {
                            id: request_id.to_string(),
                            delta,
                        },
                    );
                }
            }
            RaceOutcome::Ready(None) => break,
            RaceOutcome::Cancelled => {
                child_guard.kill_and_reap();
                return Err("cancelled".to_string());
            }
            RaceOutcome::TimedOut => {
                child_guard.kill_and_reap();
                return Err("cli-timeout".to_string());
            }
        }
    }

    let exit_status = loop {
        match child_guard.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                match race_with_cancel_and_deadline(
                    &mut cancel,
                    deadline,
                    tokio::time::sleep(Duration::from_millis(25)),
                )
                .await
                {
                    RaceOutcome::Ready(()) => continue,
                    RaceOutcome::Cancelled => {
                        child_guard.kill_and_reap();
                        return Err("cancelled".to_string());
                    }
                    RaceOutcome::TimedOut => {
                        child_guard.kill_and_reap();
                        return Err("cli-timeout".to_string());
                    }
                }
            }
            Err(e) => {
                child_guard.kill_and_reap();
                return Err(format!("Failed to wait for {} CLI: {}", provider.label(), e));
            }
        }
    };

    let stderr_text = match tokio::time::timeout(Duration::from_millis(250), &mut stderr_rx).await {
        Ok(Ok(s)) => s,
        _ => String::new(),
    };

    if !exit_status.success() {
        let err_source = if stderr_text.trim().is_empty() {
            &full_reply
        } else {
            &stderr_text
        };
        return Err(classify_cli_stderr(provider, err_source, exit_status.code()));
    }

    drop(temp_guard);
    Ok(full_reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_image_guard_removes_files_on_drop() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let p1 = std::env::temp_dir().join(format!("fern-cli-test-{unique}-0.jpg"));
        let p2 = std::env::temp_dir().join(format!("fern-cli-test-{unique}-1.jpg"));

        std::fs::write(&p1, b"fake-jpeg-1").expect("write p1");
        std::fs::write(&p2, b"fake-jpeg-2").expect("write p2");
        assert!(p1.exists());
        assert!(p2.exists());

        {
            let mut guard = TempImageGuard::new();
            guard.push(p1.clone());
            guard.push(p2.clone());
            assert_eq!(guard.paths().len(), 2);
        }

        assert!(!p1.exists(), "first temp image should be deleted on drop");
        assert!(!p2.exists(), "second temp image should be deleted on drop");
    }

    #[test]
    fn build_transcript_formats_multiturn_and_images() {
        let single = vec![AiMessage {
            role: "user".to_string(),
            text: "  Hello Fern!  ".to_string(),
            images: Vec::new(),
        }];
        assert_eq!(build_transcript(&single, &[]), "Hello Fern!");

        let multi = vec![
            AiMessage {
                role: "user".to_string(),
                text: "What is Rust?".to_string(),
                images: Vec::new(),
            },
            AiMessage {
                role: "assistant".to_string(),
                text: "A systems programming language.".to_string(),
                images: Vec::new(),
            },
            AiMessage {
                role: "user".to_string(),
                text: "Explain this screenshot.".to_string(),
                images: Vec::new(),
            },
        ];
        let paths = vec![
            PathBuf::from(r"C:\Temp\fern-ai-req1-0.jpg"),
            PathBuf::from(r"C:\Temp\fern-ai-req1-1.jpg"),
        ];

        let transcript = build_transcript(&multi, &paths);
        assert!(transcript.contains("User: What is Rust?"));
        assert!(transcript.contains("Assistant: A systems programming language."));
        assert!(transcript.contains("User: Explain this screenshot."));
        assert!(transcript.contains(r"C:\Temp\fern-ai-req1-0.jpg"));
        assert!(transcript.contains(r"C:\Temp\fern-ai-req1-1.jpg"));
    }

    #[test]
    fn classify_cli_stderr_detects_auth_and_clamps_errors() {
        assert_eq!(
            classify_cli_stderr(Provider::Claude, "Error: Please sign in first", Some(1)),
            "cli-auth-required"
        );
        assert_eq!(
            classify_cli_stderr(Provider::Gemini, "Unauthorized: not logged in", Some(1)),
            "cli-auth-required"
        );
        let long_err = "x".repeat(450);
        let clamped = classify_cli_stderr(Provider::OpenAI, &long_err, Some(2));
        assert_eq!(clamped.chars().count(), MAX_ERROR_CHARS);
    }

    #[test]
    fn drain_valid_utf8_handles_split_multibyte_chars() {
        let mut pending = Vec::new();
        let mut chunks = Vec::new();
        // 'ü' is 0xC3 0xBC
        pending.extend_from_slice(b"gr\xC3");
        drain_valid_utf8(&mut pending, &mut chunks);
        assert_eq!(chunks, vec!["gr"]);
        assert_eq!(pending, vec![0xC3]);

        pending.extend_from_slice(b"\xBCn");
        drain_valid_utf8(&mut pending, &mut chunks);
        assert_eq!(chunks, vec!["gr", "ün"]);
        assert!(pending.is_empty());
    }

    #[test]
    fn cmd_wrapper_raw_arg_preserves_spaces_and_empty_args() {
        let gemini_path = PathBuf::from(r"C:\Users\John Doe\AppData\Roaming\npm\gemini.cmd");
        let raw_gemini = format_cmd_raw_arg(&gemini_path, provider_cli_args(Provider::Gemini));
        assert_eq!(
            raw_gemini,
            r#"""C:\Users\John Doe\AppData\Roaming\npm\gemini.cmd" -p """"#
        );

        let claude_path = PathBuf::from(r"C:\Users\John Doe\AppData\Roaming\npm\claude.cmd");
        let raw_claude = format_cmd_raw_arg(&claude_path, provider_cli_args(Provider::Claude));
        assert_eq!(
            raw_claude,
            r#"""C:\Users\John Doe\AppData\Roaming\npm\claude.cmd" -p""#
        );

        let codex_path = PathBuf::from(r"C:\Users\John Doe\AppData\Roaming\npm\codex.cmd");
        let raw_codex = format_cmd_raw_arg(&codex_path, provider_cli_args(Provider::OpenAI));
        assert_eq!(
            raw_codex,
            r#"""C:\Users\John Doe\AppData\Roaming\npm\codex.cmd" exec -""#
        );

        let cmd = build_cli_command(Provider::Gemini, &gemini_path);
        assert_eq!(cmd.get_program(), "cmd.exe");
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "/D",
                "/S",
                "/C",
                r#"""C:\Users\John Doe\AppData\Roaming\npm\gemini.cmd" -p """"#
            ]
        );
    }
}
