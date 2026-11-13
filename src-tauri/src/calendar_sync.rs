use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

const CACHE_FILE: &str = "calendar_cache.json";
const DEFAULT_INTERVAL_MIN: i64 = 15;
const MIN_INTERVAL_MIN: i64 = 5;
pub const ACCOUNT_KEY: &str = "account";
pub const LOOKBACK_DAYS: i64 = 35;
pub const LOOKAHEAD_DAYS: i64 = 95;

static SYNCING: AtomicBool = AtomicBool::new(false);
static LAST_ATTEMPT_MS: AtomicI64 = AtomicI64::new(0);

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalEvent {
    pub uid: String,
    pub calendar: String,
    pub color: Option<String>,
    pub title: String,
    pub location: Option<String>,
    pub all_day: bool,
    pub start: i64,
    pub end: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalInfo {
    pub name: String,
    pub color: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceData {
    pub synced_at: Option<i64>,
    pub error: Option<String>,
    #[serde(default)]
    pub calendars: Vec<CalInfo>,
    #[serde(default)]
    pub events: Vec<CalEvent>,
}

#[derive(Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    sources: HashMap<String, SourceData>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountView {
    pub calendars: usize,
    pub events: usize,
    pub synced_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub events: usize,
    pub synced_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarState {
    pub connected: bool,
    pub synced_at: Option<i64>,
    pub account: Option<AccountView>,
    pub links: Vec<LinkView>,
    pub calendars: Vec<CalInfo>,
    pub events: Vec<CalEvent>,
}

pub fn now_ms() -> i64 {
    crate::utils::get_now_ms()
}

pub fn link_key(id: &str) -> String {
    format!("ics:{}", id)
}

fn cache_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|p| p.join(CACHE_FILE))
}

fn read_store(app: &AppHandle) -> Store {
    cache_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_store(app: &AppHandle, store: &Store) {
    let Some(path) = cache_path(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(store) {
        let _ = std::fs::write(path, json);
    }
}

fn interval_minutes(app: &AppHandle) -> i64 {
    crate::utils::get_setting_str(app, "fern-caldav-interval")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(DEFAULT_INTERVAL_MIN)
        .max(MIN_INTERVAL_MIN)
}

fn state_from(app: &AppHandle, store: &Store) -> CalendarState {
    let mut events: Vec<CalEvent> = Vec::new();
    let mut calendars: Vec<CalInfo> = Vec::new();
    let mut latest: Option<i64> = None;
    let mut note = |synced: Option<i64>| latest = latest.max(synced);

    let account = crate::caldav::is_connected(app).then(|| {
        let data = store.sources.get(ACCOUNT_KEY).cloned().unwrap_or_default();
        events.extend(data.events.iter().cloned());
        calendars.extend(data.calendars.iter().cloned());
        note(data.synced_at);
        AccountView {
            calendars: data.calendars.len(),
            events: data.events.len(),
            synced_at: data.synced_at,
            error: data.error,
        }
    });

    let links = crate::ics::list_links(app)
        .into_iter()
        .map(|meta| {
            let data = store.sources.get(&link_key(&meta.id)).cloned().unwrap_or_default();
            events.extend(data.events.iter().cloned());
            calendars.push(CalInfo { name: meta.name.clone(), color: Some(crate::ics::color_for(&meta.id)) });
            note(data.synced_at);
            LinkView {
                id: meta.id,
                name: meta.name,
                provider: meta.provider,
                events: data.events.len(),
                synced_at: data.synced_at,
                error: data.error,
            }
        })
        .collect::<Vec<_>>();

    events.sort_by_key(|e| e.start);
    CalendarState {
        connected: account.is_some() || !links.is_empty(),
        synced_at: latest,
        account,
        links,
        calendars,
        events,
    }
}

pub fn current_state(app: &AppHandle) -> CalendarState {
    state_from(app, &read_store(app))
}

pub fn emit_state(app: &AppHandle) -> CalendarState {
    let state = current_state(app);
    let _ = app.emit("caldav-updated", &state);
    state
}

fn store_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

struct SyncGuard;

impl Drop for SyncGuard {
    fn drop(&mut self) {
        SYNCING.store(false, Ordering::SeqCst);
    }
}

pub fn put_source(app: &AppHandle, key: &str, data: SourceData) {
    let _guard = store_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut store = read_store(app);
    store.sources.insert(key.to_string(), data);
    write_store(app, &store);
}

pub fn drop_source(app: &AppHandle, key: &str) {
    let _guard = store_lock().lock().unwrap_or_else(|e| e.into_inner());
    let mut store = read_store(app);
    if store.sources.remove(key).is_some() {
        write_store(app, &store);
    }
}

fn record(store: &mut Store, key: String, result: Result<(Vec<CalInfo>, Vec<CalEvent>), String>) {
    let entry = store.sources.entry(key).or_default();
    match result {
        Ok((calendars, events)) => {
            entry.calendars = calendars;
            entry.events = events;
            entry.synced_at = Some(now_ms());
            entry.error = None;
        }
        Err(e) => entry.error = Some(e),
    }
}

fn merge_sync_results(
    store: &mut Store,
    account_connected: bool,
    current_link_keys: &[String],
    fetched: Vec<(String, Result<(Vec<CalInfo>, Vec<CalEvent>), String>)>,
) {
    for (key, result) in fetched {
        let still_wanted = if key == ACCOUNT_KEY {
            account_connected
        } else {
            current_link_keys.contains(&key)
        };
        if still_wanted {
            record(store, key, result);
        }
    }
    store
        .sources
        .retain(|key, _| (account_connected && key == ACCOUNT_KEY) || current_link_keys.contains(key));
}

async fn sync(app: &AppHandle, background: bool) -> Result<CalendarState, String> {
    let creds = crate::caldav::stored_credentials(app);
    let links = crate::ics::list_links(app);
    if creds.is_none() && links.is_empty() {
        return Err("Connect a calendar account or add a calendar link first.".to_string());
    }
    if SYNCING.swap(true, Ordering::SeqCst) {
        return Err("A sync is already running.".to_string());
    }
    let _sync_guard = SyncGuard;
    LAST_ATTEMPT_MS.store(now_ms(), Ordering::Relaxed);

    let auth_failed = {
        let _guard = store_lock().lock().unwrap_or_else(|e| e.into_inner());
        read_store(app)
            .sources
            .get(ACCOUNT_KEY)
            .and_then(|s| s.error.as_deref())
            == Some(crate::caldav::AUTH_ERROR)
    };

    let mut fetched: Vec<(String, Result<(Vec<CalInfo>, Vec<CalEvent>), String>)> = Vec::new();
    if let Some(creds) = creds {
        if !(background && auth_failed) {
            let result = crate::caldav::fetch_all(&creds).await;
            fetched.push((ACCOUNT_KEY.to_string(), result));
        }
    }

    for link in &links {
        let result = crate::ics::fetch_link(app, link).await.map(|events| (Vec::new(), events));
        fetched.push((link_key(&link.id), result));
    }

    let store = {
        let _guard = store_lock().lock().unwrap_or_else(|e| e.into_inner());
        let mut store = read_store(app);
        let account_connected = crate::caldav::is_connected(app);
        let wanted: Vec<String> = crate::ics::list_links(app).iter().map(|l| link_key(&l.id)).collect();
        merge_sync_results(&mut store, account_connected, &wanted, fetched);
        write_store(app, &store);
        store
    };

    let state = state_from(app, &store);
    let _ = app.emit("caldav-updated", &state);
    Ok(state)
}

/// True when no source reported an error on the last sync.
fn last_sync_clean(app: &AppHandle) -> bool {
    read_store(app).sources.values().all(|s| s.error.is_none())
}

pub fn start_background_sync(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        // After a restart or crash, sync right away instead of waiting out the interval.
        // The network may not be back yet, so retry a few times until a sync comes through clean.
        if crate::state::RELAUNCHED.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_secs(5)).await;
            for _ in 0..6 {
                let has_source = crate::caldav::is_connected(&app) || !crate::ics::list_links(&app).is_empty();
                if !has_source {
                    break;
                }
                let _ = sync(&app, true).await;
                if last_sync_clean(&app) {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        }
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let has_source = crate::caldav::is_connected(&app) || !crate::ics::list_links(&app).is_empty();
            if has_source {
                let store = read_store(&app);
                let last = store
                    .sources
                    .values()
                    .filter_map(|s| s.synced_at)
                    .max()
                    .unwrap_or(0)
                    .max(LAST_ATTEMPT_MS.load(Ordering::Relaxed));
                if now_ms() - last >= interval_minutes(&app) * 60_000 {
                    let _ = sync(&app, true).await;
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}

#[tauri::command]
pub async fn caldav_sync_now(app: AppHandle) -> Result<CalendarState, String> {
    sync(&app, false).await
}

#[tauri::command]
pub fn caldav_get_cache(app: AppHandle) -> CalendarState {
    current_state(&app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_guard_clears_flag_on_drop() {
        SYNCING.store(false, Ordering::SeqCst);
        {
            assert!(!SYNCING.swap(true, Ordering::SeqCst));
            let _guard = SyncGuard;
            assert!(SYNCING.load(Ordering::SeqCst));
        }
        assert!(!SYNCING.load(Ordering::SeqCst));
    }

    #[test]
    fn merge_sync_results_preserves_concurrently_added_and_drops_removed_sources() {
        let mut store = Store::default();
        store.sources.insert(
            "link:added_mid_sync".to_string(),
            SourceData {
                events: vec![CalEvent {
                    uid: "u1".to_string(),
                    calendar: "New".to_string(),
                    color: None,
                    title: "Mid-sync link".to_string(),
                    location: None,
                    all_day: false,
                    start: 100,
                    end: 200,
                }],
                synced_at: Some(1234),
                ..Default::default()
            },
        );
        store.sources.insert("link:stale".to_string(), SourceData::default());

        let current_links = vec!["link:added_mid_sync".to_string(), "link:kept".to_string()];
        let fetched = vec![
            (
                ACCOUNT_KEY.to_string(),
                Ok((vec![CalInfo { name: "Work".to_string(), color: None }], vec![])),
            ),
            ("link:kept".to_string(), Ok((vec![], vec![]))),
            ("link:removed_mid_sync".to_string(), Ok((vec![], vec![]))),
        ];

        merge_sync_results(&mut store, false, &current_links, fetched);

        // Account was disconnected mid-sync -> must not be resurrected
        assert!(!store.sources.contains_key(ACCOUNT_KEY));
        // Removed link must not be resurrected; stale link must be pruned
        assert!(!store.sources.contains_key("link:removed_mid_sync"));
        assert!(!store.sources.contains_key("link:stale"));
        // Link added mid-sync must stay intact with its cached events
        assert_eq!(store.sources["link:added_mid_sync"].events.len(), 1);
        // Existing synced link gets updated
        assert!(store.sources["link:kept"].synced_at.is_some());
    }
}
