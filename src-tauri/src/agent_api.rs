use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const MAX_HEAD: usize = 8192;
const MAX_BODY: usize = 8192;
const MAX_CONNECTIONS: usize = 8;
const MAX_SSE_SUBSCRIBERS: usize = 8;
const SSE_CHANNEL_CAP: usize = 32;
const ASK_WAIT: u64 = 120;
const ASK_TIMEOUT_MIN: u64 = 5;
const ASK_TIMEOUT_MAX: u64 = 300;
const DETAIL_MAX: usize = 600;
const BADGE_MAX: usize = 32;
const MAX_ACTIONS: usize = 4;
const ACTION_ID_MAX: usize = 48;
const ACTION_LABEL_MAX: usize = 48;
const ACTION_STYLE_MAX: usize = 24;
const ACTION_URL_MAX: usize = 512;
const SETTING: &str = "fern-agent-api";

static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static NEXT_ASK: AtomicU64 = AtomicU64::new(1);
static STARTED: AtomicBool = AtomicBool::new(false);
static ENABLED: AtomicBool = AtomicBool::new(false);
static WATCHDOG_STARTED: AtomicBool = AtomicBool::new(false);
type Decision = (bool, Vec<usize>);
static PENDING: Mutex<Option<HashMap<u64, mpsc::Sender<Decision>>>> = Mutex::new(None);
static SSE_SUBSCRIBERS: Mutex<Vec<mpsc::SyncSender<String>>> = Mutex::new(Vec::new());

#[derive(Clone)]
struct OwnerTrack {
    pid: u32,
    start_time: u64,
}

static ACTIVE_OWNER: Mutex<Option<OwnerTrack>> = Mutex::new(None);
static LAST_TS: AtomicU64 = AtomicU64::new(0);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
struct CardAction {
    id: String,
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Deserialize)]
struct Incoming {
    state: String,
    name: Option<String>,
    project: Option<String>,
    window: Option<u64>,
    detail: Option<String>,
    ts: Option<u64>,
    owner_pid: Option<u32>,
    owner_start: Option<u64>,
    progress: Option<f32>,
    badge: Option<String>,
    actions: Option<Vec<CardAction>>,
}

#[derive(Deserialize)]
struct IncomingAsk {
    name: Option<String>,
    project: Option<String>,
    window: Option<u64>,
    tool: Option<String>,
    detail: Option<String>,
    questions: Option<Vec<AskQuestion>>,
    actions: Option<Vec<CardAction>>,
    timeout_secs: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone)]
struct AskQuestion {
    question: String,
    options: Vec<String>,
}

#[derive(Serialize, Clone)]
struct Ask {
    id: u64,
    tool: Option<String>,
    detail: Option<String>,
    questions: Option<Vec<AskQuestion>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actions: Option<Vec<CardAction>>,
}

#[derive(Serialize, Clone)]
struct Activity {
    state: &'static str,
    name: Option<String>,
    project: Option<String>,
    window: Option<u64>,
    detail: Option<String>,
    ask: Option<Ask>,
    cancelled: Option<u64>,
    quiet: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    progress: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    badge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actions: Option<Vec<CardAction>>,
}

#[derive(Serialize, Clone)]
pub struct IntegrationStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub detected: bool,
    pub hooks_installed: bool,
    pub config_path: Option<String>,
}

pub fn data_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|dir| PathBuf::from(dir).join("com.kagangungor.fern"))
}

fn config_path() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("agent.json"))
}

fn state_of(raw: &str) -> Option<Option<&'static str>> {
    match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "working" | "thinking" | "running" => Some(Some("working")),
        "tool" | "tool_running" => Some(Some("tool")),
        "needs_input" | "input" | "waiting" => Some(Some("needs_input")),
        "done" | "completed" | "complete" | "finished" => Some(Some("done")),
        "error" | "failed" => Some(Some("error")),
        "clear" | "idle" | "stop" => Some(None),
        _ => None,
    }
}

fn clean(text: Option<String>, limit: usize) -> Option<String> {
    let cleaned: String = text?.chars().filter(|c| !c.is_control()).take(limit).collect();
    let cleaned = cleaned.trim().to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn clamp_progress(progress: Option<f32>) -> Option<f32> {
    progress.filter(|p| p.is_finite()).map(|p| p.clamp(0.0, 1.0))
}

fn clamp_ask_timeout(timeout_secs: Option<u64>) -> u64 {
    timeout_secs
        .map(|t| t.clamp(ASK_TIMEOUT_MIN, ASK_TIMEOUT_MAX))
        .unwrap_or(ASK_WAIT)
}

fn sanitize_action_url(url: Option<String>) -> Option<String> {
    let cleaned = clean(url, ACTION_URL_MAX)?;
    let lower = cleaned.to_ascii_lowercase();
    if (lower.starts_with("http://") || lower.starts_with("https://"))
        && !cleaned.chars().any(char::is_whitespace)
    {
        Some(cleaned)
    } else {
        None
    }
}

fn sanitize_actions(actions: Option<Vec<CardAction>>) -> Option<Vec<CardAction>> {
    let cleaned: Vec<CardAction> = actions?
        .into_iter()
        .filter_map(|action| {
            let id = clean(Some(action.id), ACTION_ID_MAX)?;
            let label = clean(Some(action.label), ACTION_LABEL_MAX)?;
            Some(CardAction {
                id,
                label,
                style: clean(action.style, ACTION_STYLE_MAX),
                url: sanitize_action_url(action.url),
            })
        })
        .take(MAX_ACTIONS)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn capabilities_json() -> serde_json::Value {
    serde_json::json!({
        "apiVersion": 2,
        "endpoints": ["/v2/capabilities", "/v2/activity", "/v2/ask", "/v2/events"],
        "v1Compat": true,
        "features": ["sse", "rich_actions", "progress"],
    })
}

fn register_sse_subscriber_in(
    list: &Mutex<Vec<mpsc::SyncSender<String>>>,
) -> Option<mpsc::Receiver<String>> {
    let mut subs = list.lock().ok()?;
    if subs.len() >= MAX_SSE_SUBSCRIBERS {
        subs.retain(|tx| {
            !matches!(
                tx.try_send(": keepalive\n\n".to_string()),
                Err(mpsc::TrySendError::Disconnected(_))
            )
        });
    }
    if subs.len() >= MAX_SSE_SUBSCRIBERS {
        return None;
    }
    let (tx, rx) = mpsc::sync_channel(SSE_CHANNEL_CAP);
    subs.push(tx);
    Some(rx)
}

fn broadcast_sse_in(list: &Mutex<Vec<mpsc::SyncSender<String>>>, event: &str, data_json: &str) {
    let payload = format!("event: {event}\ndata: {data_json}\n\n");
    if let Ok(mut subs) = list.lock() {
        subs.retain(|tx| match tx.try_send(payload.clone()) {
            Ok(()) | Err(mpsc::TrySendError::Full(_)) => true,
            Err(mpsc::TrySendError::Disconnected(_)) => false,
        });
    }
}

fn broadcast_sse(event: &str, data_json: &str) {
    broadcast_sse_in(&SSE_SUBSCRIBERS, event, data_json);
}

fn close_all_sse_subscribers() {
    if let Ok(mut subs) = SSE_SUBSCRIBERS.lock() {
        subs.clear();
    }
}

fn emit_activity(app: &AppHandle, activity: Option<Activity>) {
    if let Ok(json) = serde_json::to_string(&activity) {
        broadcast_sse("activity", &json);
    }
    let _ = app.emit("agent-activity", activity);
}

fn random_token() -> Option<String> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut bytes = [0u8; 24];
    unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) }.ok().ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn reply(stream: &mut TcpStream, status: &str) {
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
}

fn reply_json(stream: &mut TcpStream, body: &str) {
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
}

fn is_stale_timestamp(incoming: Option<u64>, last: u64) -> bool {
    matches!(incoming, Some(ts) if ts > 0 && ts < last)
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn process_start_time(pid: u32) -> Option<u64> {
    use windows::Win32::Foundation::{CloseHandle, FILETIME};
    use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let ok = GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        Some(((created.dwHighDateTime as u64) << 32) | (created.dwLowDateTime as u64))
    }
}

fn is_owner_alive(track: &OwnerTrack) -> bool {
    process_start_time(track.pid).is_some_and(|st| st == track.start_time)
}

fn is_user_quiet() -> bool {
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    unsafe {
        match SHQueryUserNotificationState() {
            Ok(state) => state == QUNS_BUSY || state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE,
            Err(_) => false,
        }
    }
}

fn record_activity_owner(state: Option<&'static str>, owner_pid: Option<u32>, owner_start: Option<u64>) {
    if let Ok(mut guard) = ACTIVE_OWNER.lock() {
        if state.is_none() || state == Some("done") {
            *guard = None;
        } else if let (Some(pid), Some(start_time)) = (owner_pid, owner_start) {
            *guard = Some(OwnerTrack { pid, start_time });
        }
    }
}

fn start_owner_watchdog(app: AppHandle) {
    if WATCHDOG_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3));
        let should_clear = ACTIVE_OWNER
            .lock()
            .ok()
            .and_then(|mut guard| {
                if let Some(track) = guard.as_ref() {
                    if !is_owner_alive(track) {
                        *guard = None;
                        return Some(());
                    }
                }
                None
            })
            .is_some();
        if should_clear {
            emit_activity(&app, None);
        }
    });
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub fn answer(id: u64, allow: bool, choices: Vec<usize>) {
    let sender = PENDING.lock().ok().and_then(|mut map| map.as_mut().and_then(|m| m.remove(&id)));
    if let Some(sender) = sender {
        let _ = sender.send((allow, choices));
    }
}

fn wait_for_answer(
    stream: &mut TcpStream,
    id: u64,
    rx: &mpsc::Receiver<Decision>,
    timeout_secs: u64,
) -> Option<Decision> {
    let _ = stream.set_nonblocking(true);
    let mut waited = 0;
    while waited < timeout_secs {
        if let Ok(decision) = rx.recv_timeout(Duration::from_secs(1)) {
            let _ = stream.set_nonblocking(false);
            return Some(decision);
        }
        waited += 1;
        let mut probe = [0u8; 1];
        if matches!(stream.peek(&mut probe), Ok(0)) {
            break;
        }
    }
    if let Ok(mut map) = PENDING.lock() {
        if let Some(map) = map.as_mut() {
            map.remove(&id);
        }
    }
    let _ = stream.set_nonblocking(false);
    None
}

struct ActiveGuard(bool);

impl ActiveGuard {
    fn release(&mut self) {
        if self.0 {
            self.0 = false;
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.release();
    }
}

fn handle_activity(stream: &mut TcpStream, app: &AppHandle, body: &[u8], v2: bool) {
    let Ok(req) = serde_json::from_slice::<Incoming>(body) else {
        return reply(stream, "400 Bad Request");
    };
    let Some(state) = state_of(&req.state) else {
        return reply(stream, "400 Bad Request");
    };
    let last_ts = LAST_TS.load(Ordering::SeqCst);
    if is_stale_timestamp(req.ts, last_ts) {
        return reply(stream, "204 No Content");
    }
    let ts = req.ts.unwrap_or_else(now_epoch_ms);
    LAST_TS.store(ts, Ordering::SeqCst);
    record_activity_owner(state, req.owner_pid, req.owner_start);

    let quiet = is_user_quiet();
    let progress = if v2 { clamp_progress(req.progress) } else { None };
    let badge = if v2 { clean(req.badge, BADGE_MAX) } else { None };
    let actions = if v2 { sanitize_actions(req.actions) } else { None };
    let activity = state.map(|s| Activity {
        state: s,
        name: clean(req.name, 48),
        project: clean(req.project, 48),
        window: req.window,
        detail: clean(req.detail, DETAIL_MAX),
        ask: None,
        cancelled: None,
        quiet: quiet.then_some(true),
        progress,
        badge,
        actions,
    });
    emit_activity(app, activity);
    if v2 {
        let resp = serde_json::json!({
            "ok": true,
            "apiVersion": 2,
            "state": state,
            "ts": ts,
        })
        .to_string();
        reply_json(stream, &resp);
    } else {
        reply(stream, "204 No Content");
    }
}

fn handle_ask(stream: &mut TcpStream, app: &AppHandle, body: &[u8], v2: bool) {
    let Ok(req) = serde_json::from_slice::<IncomingAsk>(body) else {
        return reply(stream, "400 Bad Request");
    };
    let id = NEXT_ASK.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel();
    if let Ok(mut map) = PENDING.lock() {
        map.get_or_insert_with(HashMap::new).insert(id, tx);
    }
    let (name, project, window) = (clean(req.name, 48), clean(req.project, 48), req.window);
    let quiet = is_user_quiet();
    let actions = if v2 { sanitize_actions(req.actions) } else { None };
    let wait_secs = if v2 { clamp_ask_timeout(req.timeout_secs) } else { ASK_WAIT };
    let activity = Activity {
        state: "needs_input",
        name: name.clone(),
        project: project.clone(),
        window,
        detail: None,
        ask: Some(Ask {
            id,
            tool: clean(req.tool, 48),
            detail: clean(req.detail, DETAIL_MAX),
            questions: req.questions.map(|list| {
                list.into_iter()
                    .take(4)
                    .filter_map(|q| {
                        let question = clean(Some(q.question), DETAIL_MAX)?;
                        Some(AskQuestion {
                            question,
                            options: q.options.into_iter().filter_map(|o| clean(Some(o), 80)).take(4).collect(),
                        })
                    })
                    .collect()
            }),
            actions,
        }),
        cancelled: None,
        quiet: quiet.then_some(true),
        progress: None,
        badge: None,
        actions: None,
    };
    emit_activity(app, Some(activity));

    match wait_for_answer(stream, id, &rx, wait_secs) {
        Some((allow, choices)) if v2 => {
            let body = serde_json::json!({
                "allow": allow,
                "choices": choices,
                "apiVersion": 2,
            })
            .to_string();
            reply_json(stream, &body);
        }
        Some((true, choices)) if !choices.is_empty() => {
            let body = serde_json::json!({ "decision": "allow", "choices": choices }).to_string();
            reply_json(stream, &body);
        }
        Some((true, _)) => reply_json(stream, r#"{"decision":"allow"}"#),
        Some((false, _)) => reply_json(stream, r#"{"decision":"deny"}"#),
        None => {
            emit_activity(
                app,
                Some(Activity {
                    state: "working",
                    name,
                    project,
                    window,
                    detail: None,
                    ask: None,
                    cancelled: Some(id),
                    quiet: None,
                    progress: None,
                    badge: None,
                    actions: None,
                }),
            );
            reply(stream, "204 No Content");
        }
    }
}

fn handle_sse(mut stream: TcpStream, rx: mpsc::Receiver<String>) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
    if stream.write_all(headers.as_bytes()).is_err() {
        return;
    }
    if stream.write_all(b"event: ready\ndata: {\"apiVersion\":2}\n\n").is_err() || stream.flush().is_err() {
        return;
    }
    while ENABLED.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(payload) => {
                if stream.write_all(payload.as_bytes()).is_err() || stream.flush().is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !ENABLED.load(Ordering::Relaxed) {
                    break;
                }
                if stream.write_all(b": keepalive\n\n").is_err() || stream.flush().is_err() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn handle(mut stream: TcpStream, app: &AppHandle, token: &str, active_guard: &mut ActiveGuard) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    let head_end = loop {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > MAX_HEAD {
            return reply(&mut stream, "431 Request Header Fields Too Large");
        }
    };

    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(raw_path)) = (parts.next(), parts.next()) else {
        return reply(&mut stream, "400 Bad Request");
    };
    let path = raw_path.split('?').next().unwrap_or(raw_path);

    let mut auth = "";
    let mut length: usize = 0;
    for line in lines {
        if let Some((name, val)) = line.split_once(':') {
            let (name, val) = (name.trim(), val.trim());
            if name.eq_ignore_ascii_case("authorization") {
                auth = val;
            } else if name.eq_ignore_ascii_case("content-length") {
                length = val.parse().unwrap_or(0);
            }
        }
    }

    if !same(auth.strip_prefix("Bearer ").unwrap_or_default(), token) {
        return reply(&mut stream, "401 Unauthorized");
    }
    if length > MAX_BODY {
        return reply(&mut stream, "413 Payload Too Large");
    }

    let mut body = buf[head_end..].to_vec();
    while body.len() < length {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => body.extend_from_slice(&chunk[..n]),
        }
    }
    body.truncate(length);

    match (method, path) {
        ("POST", "/activity") => handle_activity(&mut stream, app, &body, false),
        ("POST", "/v2/activity") => handle_activity(&mut stream, app, &body, true),
        ("POST", "/ask") => handle_ask(&mut stream, app, &body, false),
        ("POST", "/v2/ask") => handle_ask(&mut stream, app, &body, true),
        ("GET", "/v2/capabilities") => {
            let body = capabilities_json().to_string();
            reply_json(&mut stream, &body);
        }
        ("GET", "/v2/events") => {
            let Some(rx) = register_sse_subscriber_in(&SSE_SUBSCRIBERS) else {
                return reply(&mut stream, "503 Service Unavailable");
            };
            active_guard.release();
            handle_sse(stream, rx);
        }
        _ => reply(&mut stream, "404 Not Found"),
    }
}

fn hook_entries(exe: &str) -> Vec<(&'static str, Option<&'static str>, serde_json::Value)> {
    let hook = |args: &[&str], timeout: Option<u64>| {
        let mut all = vec!["agent"];
        all.extend_from_slice(args);
        all.extend_from_slice(&["--name", "Claude Code"]);
        let mut entry = serde_json::json!({ "type": "command", "command": exe, "args": all });
        if let Some(timeout) = timeout {
            entry["timeout"] = timeout.into();
        }
        entry
    };
    vec![
        ("UserPromptSubmit", None, hook(&["working"], None)),
        ("PreToolUse", None, hook(&["tool"], None)),
        ("PostToolUse", None, hook(&["working"], None)),
        ("PostToolUseFailure", None, hook(&["error"], None)),
        ("PermissionRequest", None, hook(&["ask"], Some(ASK_WAIT + 10))),
        ("Notification", Some("idle_prompt"), hook(&["needs-input"], None)),
        ("Stop", None, hook(&["done"], None)),
        ("SessionEnd", None, hook(&["clear"], None)),
    ]
}

fn is_ours(group: &serde_json::Value) -> bool {
    group["hooks"].as_array().is_some_and(|hooks| {
        hooks.iter().any(|hook| {
            hook["args"][0].as_str() == Some("agent")
                && hook["command"].as_str().is_some_and(|c| c.to_ascii_lowercase().contains("fern"))
        })
    })
}

fn sync_claude_hooks(enable: bool) {
    let Some(path) = std::env::var_os("USERPROFILE").map(|d| PathBuf::from(d).join(".claude").join("settings.json")) else { return };
    let text = std::fs::read_to_string(&path).ok();
    let mut root: serde_json::Value = match text.as_deref().map(str::trim) {
        Some(t) if !t.is_empty() => match serde_json::from_str(t) {
            Ok(value) => value,
            Err(_) => return,
        },
        _ if enable => serde_json::json!({}),
        _ => return,
    };
    let original = root.clone();
    let Some(exe) = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()) else { return };
    let Some(object) = root.as_object_mut() else { return };

    let hooks = object.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let Some(hooks) = hooks.as_object_mut() else { return };
    for event in hooks.values_mut() {
        if let Some(groups) = event.as_array_mut() {
            groups.retain(|group| !is_ours(group));
        }
    }
    if enable {
        for (event, matcher, hook) in hook_entries(&exe) {
            let mut group = serde_json::json!({ "hooks": [hook] });
            if let Some(matcher) = matcher {
                group["matcher"] = matcher.into();
            }
            match hooks.entry(event).or_insert_with(|| serde_json::json!([])).as_array_mut() {
                Some(groups) => groups.push(group),
                None => return,
            }
        }
    }
    hooks.retain(|_, event| event.as_array().map(|groups| !groups.is_empty()).unwrap_or(true));
    if hooks.is_empty() {
        object.remove("hooks");
    }
    if root == original {
        return;
    }

    let Ok(mut out) = serde_json::to_string_pretty(&root) else { return };
    out.push('\n');
    if let Some(text) = &text {
        let backup = path.with_file_name("settings.json.fern-backup");
        if !backup.exists() {
            let _ = std::fs::write(&backup, text);
        }
    }
    let temp = path.with_file_name("settings.json.fern-tmp");
    if std::fs::write(&temp, &out).is_ok() {
        if std::fs::rename(&temp, &path).is_err() {
            let _ = std::fs::write(&path, &out);
            let _ = std::fs::remove_file(&temp);
        }
    } else {
        let _ = std::fs::write(&path, &out);
    }
}

fn codex_config_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|d| PathBuf::from(d).join(".codex").join("config.toml"))
}

fn opencode_plugin_paths() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![
        home.join(".config").join("opencode").join("plugin").join("fern.js"),
        home.join(".config").join("opencode").join("plugins").join("fern.js"),
    ]
}

fn opencode_plugin_script(exe: &str) -> String {
    let escaped = exe.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        r#"// Auto-managed by Fern — do not edit manually
import {{ spawn }} from "node:child_process";

const FERN_EXE = "{escaped}";

function notify(state, detail) {{
  try {{
    const args = ["agent", state, "--name", "OpenCode"];
    if (detail) args.push("--detail", String(detail).slice(0, 200));
    const child = spawn(FERN_EXE, args, {{ detached: true, stdio: "ignore", windowsHide: true }});
    child.unref();
  }} catch {{}}
}}

export const FernPlugin = async () => ({{
  event: async ({{ event }}) => {{
    if (!event?.type) return;
    if (event.type === "session.idle") notify("done");
    else if (event.type === "session.error") notify("error", event?.properties?.error?.message);
    else if (event.type === "permission.asked" || event.type === "permission.updated") notify("needs-input");
    else if (event.type === "session.status" && event?.properties?.status?.type === "busy") notify("working");
  }},
  "tool.execute.before": async (input) => {{
    notify("tool", input?.tool);
  }},
}});
"#
    )
}

fn sync_codex_hooks_at(path: &PathBuf, exe: &str, enable: bool) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let escaped = exe.replace('\\', "\\\\").replace('"', "\\\"");
    let notify_line = format!(r#"notify = ["{escaped}", "agent", "codex-notify"]"#);

    let mut lines: Vec<String> = text
        .lines()
        .filter(|l| !(l.trim_start().starts_with("notify") && l.contains("fern") && l.contains("agent")))
        .map(String::from)
        .collect();

    if enable {
        let has_other_notify = lines.iter().any(|l| l.trim_start().starts_with("notify ="));
        if !has_other_notify {
            // Insert at top (before any [table] header) so notify stays top-level in TOML
            let insert_at = lines.iter().position(|l| l.trim_start().starts_with('[')).unwrap_or(lines.len());
            lines.insert(insert_at, notify_line);
        }
    }

    let mut out = lines.join("\n");
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if out == text {
        return;
    }
    if !text.is_empty() {
        let backup = path.with_file_name("config.toml.fern-backup");
        if !backup.exists() {
            let _ = std::fs::write(&backup, &text);
        }
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, out);
}

fn sync_opencode_hooks_at(paths: &[PathBuf], exe: &str, enable: bool) {
    let script = opencode_plugin_script(exe);
    for path in paths {
        if enable {
            if let Some(parent) = path.parent() {
                if parent.is_dir() || path == &paths[0] {
                    let _ = std::fs::create_dir_all(parent);
                    let _ = std::fs::write(path, &script);
                }
            }
        } else if path.exists() {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn integrations_status() -> Vec<IntegrationStatus> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from);
    let claude_dir = home.as_ref().map(|h| h.join(".claude"));
    let claude_settings = claude_dir.as_ref().map(|d| d.join("settings.json"));
    let claude_installed = claude_settings
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("hooks")?.as_object().cloned())
        .is_some_and(|hooks| {
            hooks.values().any(|ev| {
                ev.as_array()
                    .is_some_and(|arr| arr.iter().any(is_ours))
            })
        });

    let codex_cfg = codex_config_path();
    let codex_detected = home.as_ref().is_some_and(|h| h.join(".codex").is_dir());
    let codex_installed = codex_cfg
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|t| t.lines().any(|l| l.trim_start().starts_with("notify") && l.contains("fern")));

    let opencode_dir = home.as_ref().map(|h| h.join(".config").join("opencode"));
    let opencode_detected = opencode_dir.as_ref().is_some_and(|d| d.is_dir());
    let opencode_paths = opencode_plugin_paths();
    let opencode_installed = opencode_paths.iter().any(|p| p.exists());

    vec![
        IntegrationStatus {
            id: "claude",
            name: "Claude Code",
            detected: claude_dir.as_ref().is_some_and(|d| d.is_dir()),
            hooks_installed: claude_installed,
            config_path: claude_settings.map(|p| p.to_string_lossy().into_owned()),
        },
        IntegrationStatus {
            id: "codex",
            name: "Codex CLI",
            detected: codex_detected,
            hooks_installed: codex_installed,
            config_path: codex_cfg.map(|p| p.to_string_lossy().into_owned()),
        },
        IntegrationStatus {
            id: "opencode",
            name: "OpenCode (v1/v2)",
            detected: opencode_detected,
            hooks_installed: opencode_installed,
            config_path: opencode_paths.first().map(|p| p.to_string_lossy().into_owned()),
        },
    ]
}

pub fn sync_single_integration(id: &str, enable: bool) {
    let Some(exe) = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()) else {
        return;
    };
    match id {
        "claude" => sync_claude_hooks(enable),
        "codex" => {
            if let Some(cfg) = codex_config_path() {
                sync_codex_hooks_at(&cfg, &exe, enable);
            }
        }
        "opencode" => {
            sync_opencode_hooks_at(&opencode_plugin_paths(), &exe, enable);
        }
        _ => {}
    }
}

pub fn set_hooks(enable: bool) {
    let is_build_output = std::env::current_exe().is_ok_and(|p| p.components().any(|c| c.as_os_str() == "target"));
    if cfg!(debug_assertions) || is_build_output {
        return;
    }
    let Some(exe) = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()) else {
        return;
    };
    if !enable || std::env::var_os("USERPROFILE").is_some_and(|d| PathBuf::from(d).join(".claude").is_dir()) {
        sync_claude_hooks(enable);
    }
    if !enable || std::env::var_os("USERPROFILE").is_some_and(|d| PathBuf::from(d).join(".codex").is_dir()) {
        if let Some(cfg) = codex_config_path() {
            sync_codex_hooks_at(&cfg, &exe, enable);
        }
    }
    if !enable || std::env::var_os("USERPROFILE").is_some_and(|d| PathBuf::from(d).join(".config").join("opencode").is_dir()) {
        sync_opencode_hooks_at(&opencode_plugin_paths(), &exe, enable);
    }
}

pub fn set_enabled(app: &AppHandle, enable: bool) {
    ENABLED.store(enable, Ordering::SeqCst);
    if !enable {
        close_all_sse_subscribers();
    }
    std::thread::spawn(move || set_hooks(enable));
    if enable {
        start(app.clone());
    }
}

pub fn start(app: AppHandle) {
    let enabled = crate::utils::get_setting_str(&app, SETTING).as_deref() == Some("true");
    ENABLED.store(enabled, Ordering::SeqCst);
    if !enabled {
        return;
    }
    start_owner_watchdog(app.clone());
    std::thread::spawn(|| set_hooks(true));
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(token) = random_token() else {
        STARTED.store(false, Ordering::SeqCst);
        return;
    };
    let Ok(listener) = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))) else {
        STARTED.store(false, Ordering::SeqCst);
        return;
    };
    let Ok(addr) = listener.local_addr() else {
        STARTED.store(false, Ordering::SeqCst);
        return;
    };
    let Some(path) = config_path() else {
        STARTED.store(false, Ordering::SeqCst);
        return;
    };
    let config = serde_json::json!({ "port": addr.port(), "token": token }).to_string();
    if std::fs::write(&path, config).is_err() {
        STARTED.store(false, Ordering::SeqCst);
        return;
    }

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            if !ENABLED.load(Ordering::Relaxed) {
                continue;
            }
            if !stream.peer_addr().map(|a| a.ip().is_loopback()).unwrap_or(false) {
                continue;
            }
            if ACTIVE.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                ACTIVE.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let app = app.clone();
            let token = token.clone();
            std::thread::spawn(move || {
                let mut guard = ActiveGuard(true);
                handle(stream, &app, &token, &mut guard);
            });
        }
    });
}

fn project_name() -> Option<String> {
    let dir = std::env::var_os("CLAUDE_PROJECT_DIR").map(PathBuf::from).or_else(|| std::env::current_dir().ok())?;
    dir.file_name().map(|n| n.to_string_lossy().into_owned())
}

fn ancestor_owner() -> (Option<u64>, Option<u32>, Option<u64>) {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindow, GetWindowThreadProcessId, IsWindowVisible, GW_OWNER};

    let mut parents: HashMap<u32, (u32, String)> = HashMap::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return (None, None, None);
        };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            parents.insert(entry.th32ProcessID, (entry.th32ParentProcessID, String::from_utf16_lossy(&entry.szExeFile[..len]).to_ascii_lowercase()));
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
    }

    let mut chain = Vec::new();
    let mut pid = std::process::id();
    while let Some((parent, _)) = parents.get(&pid) {
        match parents.get(parent) {
            Some((_, name)) if name != "explorer.exe" && !chain.contains(parent) => chain.push(*parent),
            _ => break,
        }
        pid = *parent;
    }

    struct Search {
        chain: Vec<u32>,
        found: Vec<(usize, u64)>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let search = &mut *(lparam.0 as *mut Search);
        if IsWindowVisible(hwnd).as_bool() && GetWindow(hwnd, GW_OWNER).map(|o| o.0.is_null()).unwrap_or(true) {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if let Some(rank) = search.chain.iter().position(|&p| p == pid) {
                search.found.push((rank, hwnd.0 as usize as u64));
            }
        }
        BOOL(1)
    }
    let mut search = Search { chain, found: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
    }
    let hwnd = search.found.into_iter().min_by_key(|(rank, _)| *rank).map(|(_, hwnd)| hwnd);
    let owner_pid = search.chain.first().copied();
    let owner_start = owner_pid.and_then(process_start_time);
    (hwnd, owner_pid, owner_start)
}

fn ancestor_window() -> Option<u64> {
    ancestor_owner().0
}

fn flags(args: &[String]) -> (Option<String>, Option<String>, Option<String>) {
    let (mut name, mut project, mut detail) = (None, None, None);
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        match flag.as_str() {
            "--name" | "-n" => name = rest.next().cloned(),
            "--project" | "-p" => project = rest.next().cloned(),
            "--detail" | "-d" => detail = rest.next().cloned(),
            _ => {}
        }
    }
    (name, project.or_else(project_name), detail)
}

fn post(path: &str, body: String, wait: u64) -> Option<String> {
    let config: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(config_path()?).ok()?).ok()?;
    let port = config.get("port")?.as_u64()? as u16;
    let token = config.get("token")?.as_str()?;

    let mut stream = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_secs(1)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(wait))).ok()?;
    write!(
        stream,
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}

fn send(args: &[String]) -> Option<()> {
    send_with(args, None)
}

fn send_with(args: &[String], detail: Option<String>) -> Option<()> {
    let state = args.first()?.clone();
    let (name, project, flag_detail) = flags(&args[1..]);
    let (window, owner_pid, owner_start) = ancestor_owner();
    let body = serde_json::json!({
        "state": state,
        "name": name,
        "project": project,
        "window": window,
        "owner_pid": owner_pid,
        "owner_start": owner_start,
        "ts": now_epoch_ms(),
        "detail": detail.or(flag_detail),
    })
    .to_string();
    post("/activity", body, 2)?.starts_with("HTTP/1.1 204").then_some(())
}

fn parse_ask_user_questions(raw: &serde_json::Value) -> Option<Vec<(String, Vec<String>)>> {
    let list = raw.as_array()?;
    if list.is_empty() || list.len() > 4 {
        return None;
    }
    let parsed: Vec<(String, Vec<String>)> = list
        .iter()
        .filter(|q| q["multiSelect"].as_bool() != Some(true))
        .filter_map(|q| {
            let labels: Vec<String> = q["options"]
                .as_array()?
                .iter()
                .filter_map(|o| o["label"].as_str().map(str::to_string))
                .collect();
            let question = q["question"].as_str()?.to_string();
            (2..=4).contains(&labels.len()).then_some((question, labels))
        })
        .collect();
    (parsed.len() == list.len()).then_some(parsed)
}

fn build_ask_answer_output(
    asked: &[(String, Vec<String>)],
    raw_questions: &serde_json::Value,
    decision: &serde_json::Value,
) -> Option<serde_json::Value> {
    let choices = decision["choices"].as_array()?;
    if choices.len() != asked.len() {
        return None;
    }
    let mut answers = serde_json::Map::new();
    for ((question, labels), choice) in asked.iter().zip(choices) {
        let label = choice.as_u64().and_then(|i| labels.get(i as usize))?;
        answers.insert(question.clone(), serde_json::Value::String(label.clone()));
    }
    let updated = serde_json::json!({ "questions": raw_questions, "answers": answers });
    Some(serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PermissionRequest",
            "decision": { "behavior": "allow", "updatedInput": updated }
        }
    }))
}

fn ask(args: &[String]) {
    let (name, project, _) = flags(args);
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let event: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
    let tool_input = &event["tool_input"];
    let detail: Option<String>;
    let mut asked: Option<Vec<(String, Vec<String>)>> = None;
    if event["tool_name"].as_str() == Some("AskUserQuestion") {
        let Some(parsed) = parse_ask_user_questions(&tool_input["questions"]) else {
            let question = tool_input["questions"][0]["question"].as_str().map(str::to_string);
            let mut forwarded = vec!["needs-input".to_string()];
            forwarded.extend(args.iter().cloned());
            let _ = send_with(&forwarded, question);
            return;
        };
        detail = None;
        asked = Some(parsed);
    } else {
        detail = ["command", "file_path", "url", "pattern", "description"]
            .iter()
            .find_map(|key| tool_input.get(*key).and_then(|v| v.as_str()))
            .map(str::to_string)
            .or_else(|| (!tool_input.is_null()).then(|| tool_input.to_string()));
    }
    let detail = detail.map(|d| d.chars().take(DETAIL_MAX).collect::<String>());
    let questions = asked
        .as_ref()
        .map(|list| list.iter().map(|(q, o)| serde_json::json!({ "question": q, "options": o })).collect::<Vec<_>>());
    let body = serde_json::json!({
        "name": name,
        "project": project,
        "window": ancestor_window(),
        "tool": event["tool_name"].as_str(),
        "detail": detail,
        "questions": questions,
    })
    .to_string();

    let Some(response) = post("/ask", body, ASK_WAIT + 5) else { return };
    let decision = response.split("\r\n\r\n").nth(1).and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok());
    if let Some(asked) = asked {
        if let Some(decision) = decision.as_ref() {
            if let Some(out) = build_ask_answer_output(&asked, &tool_input["questions"], decision) {
                println!("{}", out);
            }
        }
        return;
    }
    let behavior = match decision.as_ref().and_then(|d| d["decision"].as_str()) {
        Some("allow") => serde_json::json!({ "behavior": "allow" }),
        Some("deny") => serde_json::json!({ "behavior": "deny", "message": "Denied from Fern" }),
        _ => return,
    };
    println!("{}", serde_json::json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": behavior } }));
}

fn parse_codex_notify(raw: &str) -> Option<(String, Option<String>, Option<String>)> {
    let val: serde_json::Value = serde_json::from_str(raw).ok()?;
    let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let state = match event_type {
        "agent-turn-complete" | "turn.completed" => "done",
        "turn.failed" | "error" => "error",
        "approval-requested" => "needs-input",
        _ => "done",
    }
    .to_string();
    let project = val
        .get("cwd")
        .and_then(|v| v.as_str())
        .and_then(|p| std::path::Path::new(p).file_name())
        .map(|n| n.to_string_lossy().into_owned());
    let detail = val
        .get("last-assistant-message")
        .or_else(|| val.get("message"))
        .and_then(|v| v.as_str())
        .map(|s| s.chars().take(DETAIL_MAX).collect());
    Some((state, project, detail))
}

pub fn run_cli(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some("agent") {
        return None;
    }
    match args.get(2).map(String::as_str) {
        Some("install-hooks") => {
            set_hooks(true);
            return Some(0);
        }
        Some("remove-hooks") => {
            set_hooks(false);
            return Some(0);
        }
        Some("codex-notify") => {
            let raw = args.get(3).map(String::as_str).unwrap_or("");
            if let Some((state, project, detail)) = parse_codex_notify(raw) {
                let mut fwd = vec![state, "--name".into(), "Codex".into()];
                if let Some(p) = project {
                    fwd.push("--project".into());
                    fwd.push(p);
                }
                let _ = send_with(&fwd, detail);
            }
            return Some(0);
        }
        _ => {}
    }
    if args.get(2).map(String::as_str) == Some("ask") {
        ask(&args[3..]);
        return Some(0);
    }
    Some(if send(&args[2..]).is_some() { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::{build_ask_answer_output, parse_ask_user_questions};

    #[test]
    fn parse_ask_user_questions_validates_bounds_and_multiselect() {
        let valid = serde_json::json!([
            { "question": "Color?", "options": [{ "label": "Red" }, { "label": "Blue" }] },
            { "question": "Size?", "options": [{ "label": "S" }, { "label": "M" }, { "label": "L" }] }
        ]);
        assert_eq!(
            parse_ask_user_questions(&valid),
            Some(vec![
                ("Color?".into(), vec!["Red".into(), "Blue".into()]),
                ("Size?".into(), vec!["S".into(), "M".into(), "L".into()]),
            ])
        );

        let multi = serde_json::json!([
            { "question": "Pick", "multiSelect": true, "options": [{ "label": "A" }, { "label": "B" }] }
        ]);
        assert_eq!(parse_ask_user_questions(&multi), None);

        let too_few_options = serde_json::json!([
            { "question": "Only one", "options": [{ "label": "A" }] }
        ]);
        assert_eq!(parse_ask_user_questions(&too_few_options), None);

        let too_many_questions = serde_json::json!([
            { "question": "Q1", "options": [{ "label": "A" }, { "label": "B" }] },
            { "question": "Q2", "options": [{ "label": "A" }, { "label": "B" }] },
            { "question": "Q3", "options": [{ "label": "A" }, { "label": "B" }] },
            { "question": "Q4", "options": [{ "label": "A" }, { "label": "B" }] },
            { "question": "Q5", "options": [{ "label": "A" }, { "label": "B" }] }
        ]);
        assert_eq!(parse_ask_user_questions(&too_many_questions), None);
    }

    #[test]
    fn build_ask_answer_output_maps_choices_to_answers() {
        let asked = vec![
            ("Color?".into(), vec!["Red".into(), "Blue".into()]),
            ("Size?".into(), vec!["S".into(), "M".into()]),
        ];
        let raw_questions = serde_json::json!([
            { "question": "Color?", "options": [{ "label": "Red" }, { "label": "Blue" }] },
            { "question": "Size?", "options": [{ "label": "S" }, { "label": "M" }] }
        ]);
        let decision = serde_json::json!({ "decision": "allow", "choices": [1, 0] });
        let out = build_ask_answer_output(&asked, &raw_questions, &decision).expect("valid answer map");
        assert_eq!(out["hookSpecificOutput"]["decision"]["behavior"], "allow");
        assert_eq!(out["hookSpecificOutput"]["decision"]["updatedInput"]["answers"]["Color?"], "Blue");
        assert_eq!(out["hookSpecificOutput"]["decision"]["updatedInput"]["answers"]["Size?"], "S");

        let out_of_range = serde_json::json!({ "decision": "allow", "choices": [1, 9] });
        assert_eq!(build_ask_answer_output(&asked, &raw_questions, &out_of_range), None);
    }

    #[test]
    fn codex_notify_parses_turn_complete_and_cwd() {
        let raw = r#"{"type":"agent-turn-complete","cwd":"D:\\CODING\\Nectar-main","last-assistant-message":"All tests passed."}"#;
        let (state, project, detail) = super::parse_codex_notify(raw).expect("valid codex notify");
        assert_eq!(state, "done");
        assert_eq!(project.as_deref(), Some("Nectar-main"));
        assert_eq!(detail.as_deref(), Some("All tests passed."));
    }

    #[test]
    fn stale_timestamp_is_rejected_and_newer_accepted() {
        assert!(super::is_stale_timestamp(Some(100), 200));
        assert!(!super::is_stale_timestamp(Some(200), 200));
        assert!(!super::is_stale_timestamp(Some(250), 200));
        assert!(!super::is_stale_timestamp(None, 200));
    }

    #[test]
    fn codex_and_opencode_hooks_roundtrip_in_temp_dir() {
        let dir = std::env::temp_dir().join(format!("fern-agent-hooks-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let codex_cfg = dir.join("config.toml");
        std::fs::write(&codex_cfg, "model = \"o3\"\n[tui]\ntheme = \"dark\"\n").unwrap();

        super::sync_codex_hooks_at(&codex_cfg, "C:\\Program Files\\Fern\\fern.exe", true);
        let enabled = std::fs::read_to_string(&codex_cfg).unwrap();
        assert!(enabled.contains("notify = ["));
        assert!(enabled.contains("codex-notify"));

        super::sync_codex_hooks_at(&codex_cfg, "C:\\Program Files\\Fern\\fern.exe", false);
        let disabled = std::fs::read_to_string(&codex_cfg).unwrap();
        assert!(!disabled.contains("codex-notify"));
        assert!(disabled.contains("model = \"o3\""));

        let opencode_p = dir.join("plugin").join("fern.js");
        super::sync_opencode_hooks_at(&[opencode_p.clone()], "C:\\Fern\\fern.exe", true);
        assert!(opencode_p.exists());
        super::sync_opencode_hooks_at(&[opencode_p.clone()], "C:\\Fern\\fern.exe", false);
        assert!(!opencode_p.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn card_action_sanitization_drops_unsafe_urls_clamps_count_and_trims_labels() {
        let long_label = format!("   {}\x07   ", "A".repeat(80));
        let raw = vec![
            super::CardAction {
                id: "  open-js  ".into(),
                label: long_label,
                style: Some("  primary\n".into()),
                url: Some("javascript:alert(1)".into()),
            },
            super::CardAction {
                id: "open-file".into(),
                label: "Local File".into(),
                style: None,
                url: Some("file:///C:/Windows/System32/cmd.exe".into()),
            },
            super::CardAction {
                id: "open-https".into(),
                label: "Docs".into(),
                style: Some("secondary".into()),
                url: Some("https://example.com/docs".into()),
            },
            super::CardAction {
                id: "   ".into(),
                label: "Dropped empty id".into(),
                style: None,
                url: Some("https://example.com".into()),
            },
            super::CardAction {
                id: "open-http".into(),
                label: "Localhost".into(),
                style: None,
                url: Some("HTTP://127.0.0.1:8080/status".into()),
            },
            super::CardAction {
                id: "fifth-action".into(),
                label: "Should be clamped".into(),
                style: None,
                url: Some("https://example.com/5".into()),
            },
        ];

        let sanitized = super::sanitize_actions(Some(raw)).expect("valid sanitized actions");
        assert_eq!(sanitized.len(), 4);

        assert_eq!(sanitized[0].id, "open-js");
        assert_eq!(sanitized[0].label.len(), super::ACTION_LABEL_MAX - 3); // 3 leading spaces trimmed after take(48)
        assert!(!sanitized[0].label.chars().any(|c| c.is_control()));
        assert_eq!(sanitized[0].style.as_deref(), Some("primary"));
        assert_eq!(sanitized[0].url, None);

        assert_eq!(sanitized[1].id, "open-file");
        assert_eq!(sanitized[1].url, None);

        assert_eq!(sanitized[2].id, "open-https");
        assert_eq!(sanitized[2].url.as_deref(), Some("https://example.com/docs"));

        assert_eq!(sanitized[3].id, "open-http");
        assert_eq!(sanitized[3].url.as_deref(), Some("HTTP://127.0.0.1:8080/status"));
    }

    #[test]
    fn progress_clamping_handles_out_of_range_and_non_finite() {
        assert_eq!(super::clamp_progress(Some(-0.5)), Some(0.0));
        assert_eq!(super::clamp_progress(Some(1.5)), Some(1.0));
        assert_eq!(super::clamp_progress(Some(0.42)), Some(0.42));
        assert_eq!(super::clamp_progress(Some(f32::NAN)), None);
        assert_eq!(super::clamp_progress(Some(f32::INFINITY)), None);
        assert_eq!(super::clamp_progress(None), None);
    }

    #[test]
    fn v2_capabilities_payload_has_expected_structure() {
        let caps = super::capabilities_json();
        assert_eq!(caps["apiVersion"], 2);
        assert_eq!(caps["v1Compat"], true);
        assert_eq!(
            caps["endpoints"],
            serde_json::json!(["/v2/capabilities", "/v2/activity", "/v2/ask", "/v2/events"])
        );
        assert_eq!(
            caps["features"],
            serde_json::json!(["sse", "rich_actions", "progress"])
        );
    }

    #[test]
    fn sse_broadcast_fans_out_and_prunes_disconnected_subscribers() {
        let list = std::sync::Mutex::new(Vec::new());
        let rx1 = super::register_sse_subscriber_in(&list).expect("sub 1");
        let rx2 = super::register_sse_subscriber_in(&list).expect("sub 2");

        super::broadcast_sse_in(&list, "activity", r#"{"state":"working"}"#);
        let expected = "event: activity\ndata: {\"state\":\"working\"}\n\n";
        assert_eq!(rx1.try_recv().ok().as_deref(), Some(expected));
        assert_eq!(rx2.try_recv().ok().as_deref(), Some(expected));

        drop(rx2);
        super::broadcast_sse_in(&list, "activity", r#"{"state":"done"}"#);
        let expected2 = "event: activity\ndata: {\"state\":\"done\"}\n\n";
        assert_eq!(rx1.try_recv().ok().as_deref(), Some(expected2));
        assert_eq!(list.lock().unwrap().len(), 1);

        // Fill up to MAX_SSE_SUBSCRIBERS and verify cap + pruning on register
        let mut receivers = vec![rx1];
        for _ in 1..super::MAX_SSE_SUBSCRIBERS {
            receivers.push(super::register_sse_subscriber_in(&list).expect("within cap"));
        }
        assert!(super::register_sse_subscriber_in(&list).is_none());

        receivers.pop();
        assert!(super::register_sse_subscriber_in(&list).is_some());
    }

    #[test]
    fn v1_activity_and_ask_payloads_parse_backwards_compatibly() {
        let v1_activity = r#"{
            "state": "working",
            "name": "Claude Code",
            "project": "Nectar-main",
            "window": 12345,
            "detail": "Running tests",
            "ts": 1700000000000,
            "owner_pid": 4242,
            "owner_start": 99999
        }"#;
        let parsed: super::Incoming = serde_json::from_str(v1_activity).expect("v1 activity parses");
        assert_eq!(parsed.state, "working");
        assert_eq!(parsed.name.as_deref(), Some("Claude Code"));
        assert_eq!(parsed.progress, None);
        assert_eq!(parsed.badge, None);
        assert_eq!(parsed.actions, None);

        let v1_ask = r#"{
            "name": "Claude Code",
            "project": "Nectar-main",
            "window": 12345,
            "tool": "Bash",
            "detail": "cargo test",
            "questions": [{ "question": "Proceed?", "options": ["Yes", "No"] }]
        }"#;
        let parsed_ask: super::IncomingAsk = serde_json::from_str(v1_ask).expect("v1 ask parses");
        assert_eq!(parsed_ask.tool.as_deref(), Some("Bash"));
        assert_eq!(parsed_ask.actions, None);
        assert_eq!(parsed_ask.timeout_secs, None);
        assert_eq!(super::clamp_ask_timeout(parsed_ask.timeout_secs), super::ASK_WAIT);
        assert_eq!(super::clamp_ask_timeout(Some(1)), super::ASK_TIMEOUT_MIN);
        assert_eq!(super::clamp_ask_timeout(Some(999)), super::ASK_TIMEOUT_MAX);
        assert_eq!(super::clamp_ask_timeout(Some(45)), 45);
    }
}
