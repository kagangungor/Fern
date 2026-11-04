//! Opt-in Memory Saver (Settings > General). While the user is away or the session is locked, Fern
//! empties the working sets of its own process and its WebView2 processes, and stretches its polling
//! loops. Windows pages the memory back in as it is touched, so nothing is lost but a brief cost on
//! return.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
use windows::Win32::System::RemoteDesktop::{
    WTSFreeMemory, WTSQuerySessionInformationW, WTSSessionInfoEx, WTSINFOEXW,
    WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION, WTS_SESSIONSTATE_LOCK,
};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

const SETTING: &str = "fern-memory-saver";
const WEBVIEW_EXE: &str = "msedgewebview2.exe";

const TICK: Duration = Duration::from_secs(2);
const IDLE_AFTER_MS: u64 = 60_000;
/// The webviews keep running while the user is away, so their pages slowly come back.
const RETRIM_EVERY_MS: u64 = 5 * 60_000;
/// The webviews are still loading right after launch, trimming them would only slow that down.
const STARTUP_GRACE_MS: u64 = 30_000;
const RELAXED_FACTOR: u32 = 4;

static RELAXED: AtomicBool = AtomicBool::new(false);
static TRIM_SOON: AtomicBool = AtomicBool::new(false);

/// How long a polling loop with the normal interval `base` should sleep right now.
pub fn poll_interval(base: Duration) -> Duration {
    if RELAXED.load(Ordering::Relaxed) { base * RELAXED_FACTOR } else { base }
}

/// Called from the input hooks, so the loops are back at full speed on the first input rather than
/// on the worker's next tick.
pub fn note_input() {
    RELAXED.store(false, Ordering::Relaxed);
}

/// Asks for a trim on the worker's next tick, if Memory Saver is on. Used after a window is closed.
pub fn trim_soon() {
    TRIM_SOON.store(true, Ordering::Relaxed);
}

/// What the webviews are told, so their own timers can slow down too.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
struct RestState {
    resting: bool,
    locked: bool,
}

/// The webviews are still setting up their listeners right after launch, and would miss the change.
fn announced_state(resting: bool, locked: bool, uptime_ms: u64) -> RestState {
    if uptime_ms < STARTUP_GRACE_MS {
        return RestState::default();
    }
    RestState { resting, locked }
}

/// Same reasoning as the trims in `should_trim`: don't risk stutter for someone watching a video.
fn webviews_low_memory(state: RestState, media_playing: bool) -> bool {
    state.resting && (state.locked || !media_playing)
}

/// Asks WebView2 itself to give back what it can, which goes further than trimming from outside.
fn set_webviews_low_memory(app: &AppHandle, low: bool) {
    use tauri::Manager;
    for (_, window) in app.webview_windows() {
        let _ = window.with_webview(move |webview| unsafe {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
            };
            use windows_core::Interface;

            let Ok(core) = webview.controller().CoreWebView2() else { return };
            let Ok(core) = core.cast::<ICoreWebView2_19>() else { return };
            let level = if low {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
            } else {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
            };
            let _ = core.SetMemoryUsageTargetLevel(level);
        });
    }
}

fn is_resting(enabled: bool, locked: bool, idle_ms: u64) -> bool {
    enabled && (locked || idle_ms >= IDLE_AFTER_MS)
}

struct TrimInputs {
    enabled: bool,
    locked: bool,
    resting: bool,
    was_resting: bool,
    forced: bool,
    busy: bool,
    media_playing: bool,
    uptime_ms: u64,
    since_last_trim_ms: u64,
}

fn should_trim(i: &TrimInputs) -> bool {
    if !i.enabled || i.busy || i.uptime_ms < STARTUP_GRACE_MS {
        return false;
    }
    if i.forced {
        return true;
    }
    // Someone watching a video doesn't touch the input devices either, and the visualizer shouldn't
    // stutter for them. A locked session has no such audience to protect.
    if !i.resting || (i.media_playing && !i.locked) {
        return false;
    }
    !i.was_resting || i.since_last_trim_ms >= RETRIM_EVERY_MS
}

/// Fern plus its WebView2 processes. Apps launched from the dock are children of Fern too, but their
/// memory isn't ours to trim, so the walk only descends through WebView2 processes.
fn trim_targets(me: u32, processes: &[(u32, u32, String)]) -> Vec<u32> {
    let mut targets = vec![me];
    let mut next = 0;
    while next < targets.len() {
        let parent = targets[next];
        next += 1;
        for (pid, parent_pid, exe) in processes {
            if *parent_pid == parent && !targets.contains(pid) && exe.eq_ignore_ascii_case(WEBVIEW_EXE) {
                targets.push(*pid);
            }
        }
    }
    targets
}

fn list_processes() -> Vec<(u32, u32, String)> {
    let mut processes = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return processes;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let exe = String::from_utf16_lossy(&entry.szExeFile[..len]);
                processes.push((entry.th32ProcessID, entry.th32ParentProcessID, exe));
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    processes
}

fn trim_process(pid: u32) -> bool {
    unsafe {
        if pid == std::process::id() {
            return EmptyWorkingSet(GetCurrentProcess()).is_ok();
        }
        let Ok(handle): windows::core::Result<HANDLE> =
            OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, false, pid)
        else {
            return false;
        };
        let trimmed = EmptyWorkingSet(handle).is_ok();
        let _ = CloseHandle(handle);
        trimmed
    }
}

fn trim_all() -> usize {
    trim_targets(std::process::id(), &list_processes())
        .into_iter()
        .filter(|&pid| trim_process(pid))
        .count()
}

fn idle_ms() -> u64 {
    unsafe {
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        if !GetLastInputInfo(&mut info).as_bool() {
            return 0;
        }
        // Both tick counts wrap after 49.7 days, wrapping_sub stays correct across that.
        GetTickCount().wrapping_sub(info.dwTime) as u64
    }
}

fn session_locked() -> bool {
    unsafe {
        let mut buffer = PWSTR::null();
        let mut bytes = 0u32;
        if WTSQuerySessionInformationW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &mut buffer,
            &mut bytes,
        )
        .is_err()
        {
            return false;
        }
        let info = &*(buffer.0 as *const WTSINFOEXW);
        let locked = info.Level == 1
            && info.Data.WTSInfoExLevel1.SessionFlags as u32 == WTS_SESSIONSTATE_LOCK;
        WTSFreeMemory(buffer.0 as *mut _);
        locked
    }
}

pub fn start(app: AppHandle) {
    let started = std::time::Instant::now();
    let _ = std::thread::Builder::new().name("memory-saver".into()).spawn(move || {
        use crate::state::{ANY_MEDIA_PLAYING, DOCK_IS_DRAGGING, MENU_IS_OPEN, OVERLAY_IN_SPLASH, SHUTTING_DOWN};
        let mut was_resting = false;
        let mut last_trim = started;
        let mut announced = RestState::default();
        let mut webviews_low = false;
        loop {
            std::thread::sleep(TICK);
            if SHUTTING_DOWN.load(Ordering::Relaxed) {
                continue;
            }
            let enabled = crate::utils::get_setting_str(&app, SETTING).as_deref() == Some("true");
            let forced = TRIM_SOON.swap(false, Ordering::Relaxed);
            let locked = enabled && session_locked();
            let resting = is_resting(enabled, locked, idle_ms());
            RELAXED.store(resting, Ordering::Relaxed);

            let uptime_ms = started.elapsed().as_millis() as u64;
            let state = announced_state(resting, locked, uptime_ms);
            if state != announced {
                announced = state;
                let _ = app.emit("memory-saver-state", state);
            }
            let low = webviews_low_memory(state, ANY_MEDIA_PLAYING.load(Ordering::Relaxed));
            if low != webviews_low {
                webviews_low = low;
                set_webviews_low_memory(&app, low);
            }
            if !enabled {
                was_resting = false;
                continue;
            }

            let trim = should_trim(&TrimInputs {
                enabled,
                locked,
                resting,
                was_resting,
                forced,
                busy: MENU_IS_OPEN.load(Ordering::Relaxed)
                    || DOCK_IS_DRAGGING.load(Ordering::Relaxed)
                    || OVERLAY_IN_SPLASH.load(Ordering::Relaxed),
                media_playing: ANY_MEDIA_PLAYING.load(Ordering::Relaxed),
                uptime_ms,
                since_last_trim_ms: last_trim.elapsed().as_millis() as u64,
            });
            was_resting = resting;
            if trim {
                last_trim = std::time::Instant::now();
                let trimmed = trim_all();
                crate::diagnostics::log(&format!("memory saver: trimmed {trimmed} processes (locked={locked})"));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resting() -> TrimInputs {
        TrimInputs {
            enabled: true,
            locked: false,
            resting: true,
            was_resting: false,
            forced: false,
            busy: false,
            media_playing: false,
            uptime_ms: STARTUP_GRACE_MS,
            since_last_trim_ms: 0,
        }
    }

    #[test]
    fn rests_only_when_enabled_and_away_or_locked() {
        assert!(!is_resting(false, true, IDLE_AFTER_MS));
        assert!(!is_resting(true, false, IDLE_AFTER_MS - 1));
        assert!(is_resting(true, false, IDLE_AFTER_MS));
        assert!(is_resting(true, true, 0));
    }

    #[test]
    fn trims_once_on_entering_rest_then_only_periodically() {
        assert!(should_trim(&resting()));
        assert!(!should_trim(&TrimInputs { was_resting: true, ..resting() }));
        assert!(should_trim(&TrimInputs { was_resting: true, since_last_trim_ms: RETRIM_EVERY_MS, ..resting() }));
    }

    #[test]
    fn never_trims_when_off_busy_or_just_started() {
        assert!(!should_trim(&TrimInputs { enabled: false, forced: true, ..resting() }));
        assert!(!should_trim(&TrimInputs { busy: true, forced: true, ..resting() }));
        assert!(!should_trim(&TrimInputs { uptime_ms: STARTUP_GRACE_MS - 1, ..resting() }));
    }

    #[test]
    fn a_closed_window_trims_without_waiting_for_idle() {
        assert!(should_trim(&TrimInputs { resting: false, forced: true, ..resting() }));
    }

    #[test]
    fn media_blocks_idle_trims_but_not_lock_trims() {
        assert!(!should_trim(&TrimInputs { media_playing: true, ..resting() }));
        assert!(should_trim(&TrimInputs { media_playing: true, locked: true, ..resting() }));
    }

    #[test]
    fn announces_nothing_until_the_webviews_can_listen() {
        let quiet = RestState { resting: false, locked: false };
        assert_eq!(announced_state(true, true, STARTUP_GRACE_MS - 1), quiet);
        assert_eq!(announced_state(true, true, STARTUP_GRACE_MS), RestState { resting: true, locked: true });
        assert_eq!(announced_state(true, false, STARTUP_GRACE_MS), RestState { resting: true, locked: false });
    }

    #[test]
    fn webviews_go_low_unless_someone_may_be_watching() {
        assert!(!webviews_low_memory(RestState { resting: false, locked: false }, false));
        assert!(webviews_low_memory(RestState { resting: true, locked: false }, false));
        assert!(!webviews_low_memory(RestState { resting: true, locked: false }, true));
        assert!(webviews_low_memory(RestState { resting: true, locked: true }, true));
    }

    #[test]
    fn targets_only_fern_and_its_webview_tree() {
        let processes = vec![
            (2, 1, "msedgewebview2.exe".to_string()),
            (3, 2, "MsEdgeWebView2.exe".to_string()),
            (4, 1, "notepad.exe".to_string()),
            (5, 4, "msedgewebview2.exe".to_string()),
            (6, 99, "msedgewebview2.exe".to_string()),
        ];
        assert_eq!(trim_targets(1, &processes), vec![1, 2, 3]);
    }
}
