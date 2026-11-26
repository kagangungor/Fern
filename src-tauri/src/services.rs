use std::sync::{atomic::{AtomicBool, AtomicU8, AtomicI64, AtomicI32, Ordering}, Mutex, OnceLock};
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::core::BOOL;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW, WH_MOUSE_LL, MSLLHOOKSTRUCT, WM_MOUSEMOVE, WM_LBUTTONDOWN, SetWindowsHookExW, CallNextHookEx};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_NAME_WIN32};
use windows::Win32::Foundation::CloseHandle;
use std::path::Path;
use wmi::{COMLibrary, WMIConnection};

static KEYBOARD_HOOK_APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// Whether a HUD overlay is enabled. When it's off, its keys and the native flyout stay with Windows.
fn overlay_enabled(app: &AppHandle, key: &str) -> bool {
    crate::utils::get_setting_str(app, key).map(|v| v != "false").unwrap_or(true)
}

fn hook_overlay_enabled(key: &str) -> bool {
    KEYBOARD_HOOK_APP_HANDLE.get().is_none_or(|app| overlay_enabled(app, key))
}

/// Win key state, tracked so Win+1-9 can be claimed while Win itself still reaches the shell.
static WIN_KEY_DOWN: AtomicBool = AtomicBool::new(false);
/// Digit of the held Win+Number combo (0 = none). Auto-repeat re-fires keydown, only the first press counts.
static WIN_NUMBER_HELD: AtomicU8 = AtomicU8::new(0);
static WIN_B_HELD: AtomicBool = AtomicBool::new(false);
/// Another key went down while Win was held, so the Win release won't open Start.
static WIN_COMBO_USED: AtomicBool = AtomicBool::new(false);
/// When a lone Win tap was released (ms). Start opens cloaked and takes a moment to be detected as
/// foreground, so the dock treats this as "Start opening" right away instead of waiting for it.
static START_TAP_MS: AtomicI64 = AtomicI64::new(0);
const START_TAP_GRACE_MS: i64 = 700;
/// Whether Start/Search is currently detected on the primary dock monitor.
static CURRENT_START_ON_DOCK: AtomicBool = AtomicBool::new(false);
/// Visibility/overlap loop thread, unparked on foreground or Win tap changes.
static VISIBILITY_THREAD: OnceLock<std::thread::Thread> = OnceLock::new();
/// Unassigned virtual key used as the mask key (see send_start_menu_mask).
const MASK_VK: u16 = 0xE8;

pub fn setup_keyboard_hook(app: AppHandle) -> Option<windows::Win32::UI::WindowsAndMessaging::HHOOK> {
    let _ = KEYBOARD_HOOK_APP_HANDLE.set(app);
    unsafe {
        match windows::Win32::UI::WindowsAndMessaging::SetWindowsHookExA(windows::Win32::UI::WindowsAndMessaging::WH_KEYBOARD_LL, Some(keyboard_hook_proc), None, 0) {
            Ok(h) => Some(h),
            Err(e) => {
                crate::diagnostics::log(&format!("keyboard hook install failed: {e}"));
                None
            }
        }
    }
}

/// Top-row digits 1-9 to a zero-based dock slot.
fn win_number_index(vk: u16) -> Option<u8> {
    match vk {
        0x31..=0x39 => Some((vk - 0x31) as u8),
        _ => None,
    }
}

/// Win+Number can be turned off in Settings > Dock.
fn dock_win_number_enabled() -> bool {
    let Some(app) = KEYBOARD_HOOK_APP_HANDLE.get() else { return true };
    crate::utils::get_setting_str(app, "fern-dock-win-number-enabled")
        .map(|v| v != "false")
        .unwrap_or(true)
}

/// Win state straight from the OS. The tracked flag goes stale if a keyup never arrives.
fn win_key_physically_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LWIN, VK_RWIN};
    unsafe {
        (GetAsyncKeyState(VK_LWIN.0 as i32) as u16 & 0x8000) != 0
            || (GetAsyncKeyState(VK_RWIN.0 as i32) as u16 & 0x8000) != 0
    }
}

/// Explorer opens Start on a lone Win down/up, and a swallowed Win+Number looks exactly like that.
/// Tapping an unassigned key first (AutoHotkey's #MenuMaskKey trick) makes it treat Win as a modifier.
fn send_start_menu_mask() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY};
    let mask = VIRTUAL_KEY(MASK_VK);
    let inputs = [
        INPUT {
            r#type: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: mask, wScan: 0, dwFlags: Default::default(), time: 0, dwExtraInfo: 0 } },
        },
        INPUT {
            r#type: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: mask, wScan: 0, dwFlags: KEYEVENTF_KEYUP, time: 0, dwExtraInfo: 0 } },
        },
    ];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32); }
}

/// Hands the slot to the dock, its click handler already focuses or launches.
fn emit_dock_win_number(index: u8) {
    let Some(app) = KEYBOARD_HOOK_APP_HANDLE.get().cloned() else { return };
    // Don't block input on webview IPC.
    tauri::async_runtime::spawn(async move {
        let _ = app.emit_to("dock", "dock-win-number", index);
    });
}

unsafe extern "system" fn keyboard_hook_proc(code: i32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{KBDLLHOOKSTRUCT, WM_KEYDOWN, WM_SYSKEYDOWN, WM_KEYUP, WM_SYSKEYUP, LLKHF_INJECTED};
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_VOLUME_MUTE, VK_VOLUME_UP, VK_VOLUME_DOWN, VK_LWIN, VK_RWIN, VIRTUAL_KEY};
    if code >= 0 {
        crate::memory_saver::note_input();
        let kb = *(lparam.0 as *const KBDLLHOOKSTRUCT);
        let vk_code = VIRTUAL_KEY(kb.vkCode as u16);
        let is_down = wparam.0 == WM_KEYDOWN as usize || wparam.0 == WM_SYSKEYDOWN as usize;
        let is_up = wparam.0 == WM_KEYUP as usize || wparam.0 == WM_SYSKEYUP as usize;

        // Only physical keys count, our own injected taps must not loop back in.
        if (kb.flags.0 & LLKHF_INJECTED.0) == 0 {
            if vk_code == VK_LWIN || vk_code == VK_RWIN {
                if is_down {
                    // Auto-repeat re-sends keydown while held, only the first press starts a combo.
                    if !WIN_KEY_DOWN.swap(true, Ordering::Relaxed) {
                        WIN_COMBO_USED.store(false, Ordering::Relaxed);
                    }
                    // A new Win press starts a fresh combo, even if a digit keyup got missed.
                    WIN_NUMBER_HELD.store(0, Ordering::Relaxed);
                    WIN_B_HELD.store(false, Ordering::Relaxed);
                } else if is_up {
                    let was_down = WIN_KEY_DOWN.swap(false, Ordering::Relaxed);
                    // A lone tap opens Start, unless Start is already up (then it closes it).
                    if was_down && !WIN_COMBO_USED.load(Ordering::Relaxed)
                        && !CURRENT_START_ON_DOCK.load(Ordering::Relaxed)
                    {
                        START_TAP_MS.store(now_ms(), Ordering::Relaxed);
                        if let Some(t) = VISIBILITY_THREAD.get() { t.unpark(); }
                    }
                    WIN_NUMBER_HELD.store(0, Ordering::Relaxed);
                    WIN_B_HELD.store(false, Ordering::Relaxed);
                }
            } else if WIN_KEY_DOWN.load(Ordering::Relaxed) {
                if is_down { WIN_COMBO_USED.store(true, Ordering::Relaxed); }
                if vk_code.0 == 0x42 {
                    if is_up {
                        WIN_B_HELD.store(false, Ordering::Relaxed);
                    } else if is_down && win_key_physically_down()
                        && NATIVE_TASKBAR_HIDDEN.load(Ordering::Relaxed)
                    {
                        if !WIN_B_HELD.swap(true, Ordering::Relaxed) {
                            send_start_menu_mask();
                            if let Some(app) = KEYBOARD_HOOK_APP_HANDLE.get().cloned() {
                                crate::commands::open_tray_window(app);
                            }
                        }
                        return windows::Win32::Foundation::LRESULT(1);
                    }
                }
                if let Some(index) = win_number_index(vk_code.0) {
                    let slot = index + 1;
                    if is_up {
                        let _ = WIN_NUMBER_HELD.compare_exchange(slot, 0, Ordering::Relaxed, Ordering::Relaxed);
                    } else if is_down {
                        if !win_key_physically_down() {
                            WIN_KEY_DOWN.store(false, Ordering::Relaxed);
                            WIN_NUMBER_HELD.store(0, Ordering::Relaxed);
                        } else if NATIVE_TASKBAR_HIDDEN.load(Ordering::Relaxed)
                            && dock_win_number_enabled()
                        {
                            if WIN_NUMBER_HELD.swap(slot, Ordering::Relaxed) != slot {
                                send_start_menu_mask();
                                emit_dock_win_number(index);
                            }
                            return windows::Win32::Foundation::LRESULT(1);
                        }
                    }
                }
            }
        }

        if (vk_code == VK_VOLUME_MUTE || vk_code == VK_VOLUME_UP || vk_code == VK_VOLUME_DOWN)
            && hook_overlay_enabled("fern-volume-overlay-enabled")
        {
            if is_down { handle_volume_key_event(vk_code); }
            return windows::Win32::Foundation::LRESULT(1);
        }
        if (vk_code.0 == 0x216 || vk_code.0 == 0x217)
            && hook_overlay_enabled("fern-brightness-overlay-enabled")
        {
            if is_down { handle_brightness_key_event(vk_code); }
            return windows::Win32::Foundation::LRESULT(1);
        }
    }
    windows::Win32::UI::WindowsAndMessaging::CallNextHookEx(None, code, wparam, lparam)
}

fn brightness_key_command(vk_code: u16) -> Option<SystemCommand> {
    match vk_code {
        0x216 => Some(SystemCommand::BrightnessDown),
        0x217 => Some(SystemCommand::BrightnessUp),
        _ => None,
    }
}

fn handle_brightness_key_event(vk_code: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) {
    if let Some(sender) = crate::state::COMMAND_SENDER.get() {
        if let Some(cmd) = brightness_key_command(vk_code.0) {
            let _ = sender.send(cmd);
        }
    }
}

fn handle_volume_key_event(vk_code: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) {
    use std::sync::atomic::AtomicU64;
    static LAST_TIME: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last = LAST_TIME.load(std::sync::atomic::Ordering::Relaxed);
    if now.saturating_sub(last) < 50 { return; }
    LAST_TIME.store(now, std::sync::atomic::Ordering::Relaxed);
    if let Some(sender) = crate::state::COMMAND_SENDER.get() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_VOLUME_MUTE, VK_VOLUME_UP, VK_VOLUME_DOWN};
        let cmd = match vk_code {
            VK_VOLUME_MUTE => Some(SystemCommand::VolumeMute),
            VK_VOLUME_UP => Some(SystemCommand::VolumeUp),
            VK_VOLUME_DOWN => Some(SystemCommand::VolumeDown),
            _ => None,
        };
        if let Some(cmd) = cmd { let _ = sender.send(cmd); }
    }
}
use crate::types::*;
use crate::state::*;
use crate::utils::*;

pub fn setup_taskbar_hook() {
    unsafe {
        use windows::Win32::UI::Accessibility::SetWinEventHook;
        use windows::Win32::UI::WindowsAndMessaging::{EVENT_OBJECT_SHOW, EVENT_OBJECT_LOCATIONCHANGE, WINEVENT_OUTOFCONTEXT};

        // Hook both "Show" and "Location Change" (happen when maximizing/switching apps)
        let _show_hook = SetWinEventHook(EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW, None, Some(taskbar_event_proc), 0, 0, WINEVENT_OUTOFCONTEXT);
        let _loc_hook = SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, None, Some(taskbar_event_proc), 0, 0, WINEVENT_OUTOFCONTEXT);
    }
}

static WINDOW_CHANGE_APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();
static LAST_WINDOW_CHANGE_MS: AtomicI64 = AtomicI64::new(0);
pub static SELECTED_MEDIA_SOURCE: Mutex<Option<String>> = Mutex::new(None);
static ACTIVE_MEDIA_SOURCE: Mutex<String> = Mutex::new(String::new());
static ACTIVE_MEDIA_TITLE: Mutex<String> = Mutex::new(String::new());
pub static MEDIA_REFRESH: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
struct MediaSessionCandidate {
    index: u32,
    source_id: String,
    title: String,
    is_playing: bool,
    last_updated_ticks: i64,
}

fn choose_media_session_index(
    entries: &[MediaSessionCandidate],
    selected_source: Option<&str>,
    prev_active_source: &str,
    prev_active_title: &str,
    current_session: Option<(&str, &str)>,
) -> Option<u32> {
    if entries.is_empty() {
        return None;
    }

    let is_current = |e: &MediaSessionCandidate| {
        current_session.is_some_and(|(cid, ctitle)| e.source_id == cid && e.title == ctitle)
    };
    let is_prev_exact = |e: &MediaSessionCandidate| {
        !prev_active_source.is_empty()
            && e.source_id == prev_active_source
            && e.title == prev_active_title
    };

    if let Some(sel) = selected_source {
        if let Some(chosen) = entries
            .iter()
            .filter(|e| e.source_id == sel)
            .max_by_key(|e| {
                (
                    e.is_playing,
                    is_current(e),
                    e.last_updated_ticks,
                    is_prev_exact(e),
                    std::cmp::Reverse(e.index),
                )
            })
        {
            return Some(chosen.index);
        }
    }

    if let Some(chosen) = entries
        .iter()
        .filter(|e| e.is_playing)
        .max_by_key(|e| {
            (
                is_current(e),
                e.last_updated_ticks,
                is_prev_exact(e),
                std::cmp::Reverse(e.index),
            )
        })
    {
        return Some(chosen.index);
    }

    entries
        .iter()
        .max_by_key(|e| {
            (
                is_prev_exact(e),
                !prev_active_source.is_empty() && e.source_id == prev_active_source,
                is_current(e),
                e.last_updated_ticks,
                std::cmp::Reverse(e.index),
            )
        })
        .map(|e| e.index)
}

fn media_app_name(source_id: &str) -> String {
    let id = source_id.rsplit('!').next().unwrap_or(source_id);
    let id = id.trim_end_matches(".exe").trim_end_matches(".EXE");
    let name = id.rsplit('.').next().unwrap_or(id);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn media_control_session(
    mgr: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager,
) -> windows::core::Result<windows::Media::Control::GlobalSystemMediaTransportControlsSession> {
    let active = ACTIVE_MEDIA_SOURCE.lock().map(|g| g.clone()).unwrap_or_default();
    let active_title = ACTIVE_MEDIA_TITLE.lock().map(|g| g.clone()).unwrap_or_default();
    if !active.is_empty() {
        if let Ok(sessions) = mgr.GetSessions() {
            let mut source_fallback = None;
            for i in 0..sessions.Size().unwrap_or(0) {
                if let Ok(session) = sessions.GetAt(i) {
                    if session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default() == active {
                        if !active_title.is_empty() {
                            let title = session
                                .TryGetMediaPropertiesAsync()
                                .and_then(|op| op.get())
                                .and_then(|p| p.Title())
                                .map(|t| t.to_string())
                                .unwrap_or_default();
                            if title == active_title {
                                return Ok(session);
                            }
                        }
                        if source_fallback.is_none() {
                            source_fallback = Some(session);
                        }
                    }
                }
            }
            if let Some(session) = source_fallback {
                return Ok(session);
            }
        }
    }
    mgr.GetCurrentSession()
}

pub fn setup_window_change_hook(app_handle: AppHandle) {
    unsafe {
        let _ = WINDOW_CHANGE_APP_HANDLE.set(app_handle);

        use windows::Win32::UI::Accessibility::SetWinEventHook;
        use windows::Win32::UI::WindowsAndMessaging::{
            WINEVENT_OUTOFCONTEXT, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY,
            EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE,
        };
        const EVENT_OBJECT_CLOAKED: u32 = 0x8017;
        const EVENT_OBJECT_UNCLOAKED: u32 = 0x8018;

        // Hook create and destroy events for top-level windows
        let _create_hook = SetWinEventHook(
            EVENT_OBJECT_CREATE, EVENT_OBJECT_CREATE,
            None, Some(window_change_event_proc), 0, 0, WINEVENT_OUTOFCONTEXT,
        );
        let _destroy_hook = SetWinEventHook(
            EVENT_OBJECT_DESTROY, EVENT_OBJECT_DESTROY,
            None, Some(window_change_event_proc), 0, 0, WINEVENT_OUTOFCONTEXT,
        );
        // Hook show/hide events (fires when windows become visible/invisible)
        let _show_hook = SetWinEventHook(
            EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW,
            None, Some(window_change_event_proc), 0, 0, WINEVENT_OUTOFCONTEXT,
        );
        let _hide_hook = SetWinEventHook(
            EVENT_OBJECT_HIDE, EVENT_OBJECT_HIDE,
            None, Some(window_change_event_proc), 0, 0, WINEVENT_OUTOFCONTEXT,
        );
        // Hook cloak/uncloak events so UWP Snipping Tool dismissal is detected promptly
        let _cloak_hook = SetWinEventHook(
            EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED,
            None, Some(window_change_event_proc), 0, 0, WINEVENT_OUTOFCONTEXT,
        );

        // Store hooks to prevent them from being dropped
        Box::leak(Box::new(_create_hook));
        Box::leak(Box::new(_destroy_hook));
        Box::leak(Box::new(_show_hook));
        Box::leak(Box::new(_hide_hook));
        Box::leak(Box::new(_cloak_hook));
    }
}

unsafe extern "system" fn window_change_event_proc(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _event_thread: u32,
    _ms_event_time: u32,
) {
    if hwnd.0.is_null() { return; }

    // Only process window-level events (OBJID_WINDOW = 0, CHILDID_SELF = 0).
    // Child controls, carets, scrollbars and menus must NOT trigger dock rescans.
    if id_object != 0 || id_child != 0 { return; }

    // A top-level window appeared/disappeared (or cloaked/uncloaked): capture UI
    // state may have changed. Cheap flag; the worker thread does the scan.
    {
        use windows::Win32::UI::WindowsAndMessaging::{EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE, EVENT_OBJECT_DESTROY};
        const EVENT_OBJECT_CLOAKED: u32 = 0x8017;
        const EVENT_OBJECT_UNCLOAKED: u32 = 0x8018;
        if event == EVENT_OBJECT_CLOAKED || event == EVENT_OBJECT_UNCLOAKED {
            CAPTURE_RECHECK.store(true, Ordering::Relaxed);
            return;
        }
        if event == EVENT_OBJECT_SHOW || event == EVENT_OBJECT_HIDE || event == EVENT_OBJECT_DESTROY {
            CAPTURE_RECHECK.store(true, Ordering::Relaxed);
        }
    }

    use windows::Win32::UI::WindowsAndMessaging::{
        IsWindow, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
        GetWindowThreadProcessId, EVENT_OBJECT_DESTROY,
    };

    // For non-destroy events, verify the window is valid and not a tool window or our own process.
    // When event == EVENT_OBJECT_DESTROY, the window is already destroyed so IsWindow returns false;
    // we must not drop the destroy event.
    if event != EVENT_OBJECT_DESTROY {
        if !IsWindow(Some(hwnd)).as_bool() { return; }

        // Skip tool windows (docks, trays, etc.)
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if (ex_style & WS_EX_TOOLWINDOW.0) != 0 { return; }

        // Skip our own process
        let my_pid = std::process::id();
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == my_pid { return; }
    }

    // Debounce: 50ms groups rapid bursts (e.g. window creations/closing) while
    // feeling instant to the user.
    let now = crate::utils::get_now_ms();
    LAST_WINDOW_CHANGE_MS.store(now, Ordering::Relaxed);

    if let Some(app_handle) = WINDOW_CHANGE_APP_HANDLE.get().cloned() {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            if LAST_WINDOW_CHANGE_MS.load(Ordering::Relaxed) == now {
                let _ = app_handle.emit("windows-changed", ());
            }
        });
    }
}

pub fn setup_thumbnail_capture(_app_handle: AppHandle) {
    unsafe {
        use windows::Win32::UI::Accessibility::SetWinEventHook;
        use windows::Win32::UI::WindowsAndMessaging::{WINEVENT_OUTOFCONTEXT, EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_FOREGROUND};

        let hook = SetWinEventHook(
            EVENT_SYSTEM_MINIMIZESTART,
            EVENT_SYSTEM_MINIMIZEEND,
            None,
            Some(thumbnail_capture_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        Box::leak(Box::new(hook));

        let focus_hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(focus_event_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        Box::leak(Box::new(focus_hook));
    }
}

unsafe extern "system" fn focus_event_proc(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _ms_event_time: u32,
) {
    if hwnd.0.is_null() { return; }
    // Wake the visibility loop now so Start/Search updates the dock without waiting for the next tick.
    if let Some(t) = VISIBILITY_THREAD.get() { t.unpark(); }
    // Foreground moved — e.g. Snipping Tool opened/closed or got minimised.
    CAPTURE_RECHECK.store(true, Ordering::Relaxed);
    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};

    if !IsWindow(Some(hwnd)).as_bool() { return; }
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 { return; }

    let my_pid = std::process::id();
    let mut pid = 0u32;
    windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == my_pid { return; }

    let hwnd_raw = hwnd.0 as isize;
    if let Some(map) = crate::state::FOCUS_TIMESTAMPS.get() {
        if let Ok(mut guard) = map.lock() {
            guard.insert(hwnd_raw, crate::utils::get_now_ms());
        }
    }

    // Keep the focused window's thumbnail warm so a later minimized one has a cached image.
    std::thread::spawn(move || {
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextW, IsIconic, IsWindow};

        std::thread::sleep(std::time::Duration::from_millis(450));
        let hwnd = HWND(hwnd_raw as *mut _);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() { return; }
        }

        // Skip fullscreen windows (games, video players): capturing them can hitch.
        if crate::utils::is_window_fullscreen(hwnd) { return; }

        let mut text = [0u16; 2];
        unsafe {
            if GetWindowTextW(hwnd, &mut text) == 0 { return; }
        }

        // Refresh at most once per window every two seconds
        if let Some(cache) = crate::state::THUMBNAIL_CACHE.get() {
            if let Ok(guard) = cache.lock() {
                if let Some((_, ts)) = guard.get(&hwnd_raw) {
                    if crate::utils::get_now_ms() - ts < 2000 { return; }
                }
            }
        }

        if THUMB_CAPTURE_IN_FLIGHT.swap(true, Ordering::Relaxed) { return; }
        let _guard = ThumbnailCaptureGuard;

        if let Some(img) = crate::utils::capture_hwnd_to_base64(hwnd, 320, 200) {
            if let Some(cache) = crate::state::THUMBNAIL_CACHE.get() {
                if let Ok(mut guard) = cache.lock() {
                    guard.insert(hwnd_raw, (img, crate::utils::get_now_ms()));
                }
            }
        }
    });
}

static THUMB_CAPTURE_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

struct ThumbnailCaptureGuard;

impl Drop for ThumbnailCaptureGuard {
    fn drop(&mut self) {
        THUMB_CAPTURE_IN_FLIGHT.store(false, Ordering::Relaxed);
    }
}

unsafe extern "system" fn thumbnail_capture_proc(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _event_thread: u32,
    _ms_event_time: u32,
) {
    if hwnd.0.is_null() { return; }

    // Only restore matters. On minimize start the window is mid-animation and
    // PrintWindow can grab a black frame over a good thumbnail.
    if event != windows::Win32::UI::WindowsAndMessaging::EVENT_SYSTEM_MINIMIZEEND { return; }

    use windows::Win32::UI::WindowsAndMessaging::{IsWindow, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};

    if !IsWindow(Some(hwnd)).as_bool() { return; }
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 { return; }

    let my_pid = std::process::id();
    let mut pid = 0u32;
    windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == my_pid { return; }

    let mut text = [0u16; 512];
    let len = windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut text);
    if len == 0 { return; }

    let hwnd_raw = hwnd.0 as isize;

    std::thread::spawn(move || {
        use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};

        std::thread::sleep(std::time::Duration::from_millis(450));
        let hwnd = HWND(hwnd_raw as *mut _);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() { return; }
        }
        if crate::utils::is_window_fullscreen(hwnd) { return; }

        // Skip if another capture refreshed this window very recently
        if let Some(cache) = crate::state::THUMBNAIL_CACHE.get() {
            if let Ok(guard) = cache.lock() {
                if let Some((_, ts)) = guard.get(&hwnd_raw) {
                    if crate::utils::get_now_ms() - ts < 300 {
                        return;
                    }
                }
            }
        }

        if THUMB_CAPTURE_IN_FLIGHT.swap(true, Ordering::Relaxed) { return; }
        let _guard = ThumbnailCaptureGuard;

        if let Some(img) = crate::utils::capture_hwnd_to_base64(hwnd, 320, 200) {
            if let Some(cache) = crate::state::THUMBNAIL_CACHE.get() {
                if let Ok(mut guard) = cache.lock() {
                    guard.insert(hwnd_raw, (img, crate::utils::get_now_ms()));
                    if guard.len() > 15 {
                        guard.retain(|&k, _| {
                            unsafe { IsWindow(Some(HWND(k as *mut _))).as_bool() }
                        });
                    }
                }
            }
        }
    });
}

unsafe extern "system" fn taskbar_event_proc(
    _h_win_event_hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _dw_event_thread: u32,
    _dwms_event_time: u32,
) {
    if NATIVE_TASKBAR_HIDDEN.load(Ordering::Relaxed) {
        let mut class_name = [0u8; 256];
        let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameA(hwnd, &mut class_name);
        let class_str = std::str::from_utf8(&class_name[..len as usize]).unwrap_or("");
        
        if class_str == "Shell_TrayWnd" || class_str == "Shell_SecondaryTrayWnd" {
            // Taskbar is trying to show or move: slap it back down.
            set_taskbar_visibility(false, false);
        }
    }
}


static AUDIO_CAPTURE_REFUSED: AtomicBool = AtomicBool::new(false);

fn consent_value(subkey: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let key: Vec<u16> = subkey.encode_utf16().chain(Some(0)).collect();
    let name: Vec<u16> = "Value".encode_utf16().chain(Some(0)).collect();
    let mut buf = [0u16; 32];
    let mut len = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut len))
    };
    if status != ERROR_SUCCESS { return None; }
    let n = (len as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n]))
}

fn microphone_blocked() -> bool {
    const BASE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";
    if consent_value(BASE).as_deref() == Some("Deny") { return true; }
    std::env::current_exe().ok().is_some_and(|exe| {
        let app_key = exe.to_string_lossy().replace('\\', "#");
        consent_value(&format!("{BASE}\\NonPackaged\\{app_key}")).as_deref() == Some("Deny")
    })
}

fn audio_visualization_wanted(app: &AppHandle) -> bool {
    let setting = crate::utils::get_setting_str(app, "fern-media-visualizer-enabled")
        .or_else(|| crate::utils::get_setting_str(app, "fern-visualizer-enabled"));
    ANY_MEDIA_PLAYING.load(Ordering::Relaxed)
        && setting.as_deref() != Some("false")
        && !AUDIO_CAPTURE_REFUSED.load(Ordering::Relaxed)
        && !microphone_blocked()
}

unsafe fn is_ieee_float_format(format_ptr: *const windows::Win32::Media::Audio::WAVEFORMATEX) -> bool {
    const WAVE_FORMAT_IEEE_FLOAT_TAG: u16 = 0x0003;
    const WAVE_FORMAT_EXTENSIBLE_TAG: u16 = 0xFFFE;
    let wf = &*format_ptr;
    if wf.wFormatTag == WAVE_FORMAT_IEEE_FLOAT_TAG {
        return true;
    }
    if wf.wFormatTag == WAVE_FORMAT_EXTENSIBLE_TAG && wf.cbSize >= 22 {
        const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: windows::core::GUID =
            windows::core::GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
        let ext_ptr = format_ptr as *const windows::Win32::Media::Audio::WAVEFORMATEXTENSIBLE;
        let subformat = std::ptr::read_unaligned(std::ptr::addr_of!((*ext_ptr).SubFormat));
        return subformat == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
    }
    false
}

fn decode_audio_frame(frame_bytes: &[u8], channels: usize, bytes_per_sample: usize, is_float: bool) -> f32 {
    if channels == 0 || bytes_per_sample == 0 || frame_bytes.len() < channels * bytes_per_sample {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for ch in 0..channels {
        let start = ch * bytes_per_sample;
        let s = &frame_bytes[start..start + bytes_per_sample];
        let val = if is_float && bytes_per_sample == 4 {
            let f = f32::from_le_bytes([s[0], s[1], s[2], s[3]]);
            if f.is_finite() { (f as f64).clamp(-1.0, 1.0) } else { 0.0 }
        } else if !is_float && bytes_per_sample == 2 {
            (i16::from_le_bytes([s[0], s[1]]) as f64) / 32768.0
        } else if !is_float && bytes_per_sample == 4 {
            (i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64) / 2147483648.0
        } else {
            0.0
        };
        sum += val;
    }
    (sum / channels as f64) as f32
}

pub fn setup_audio_visualization(app_handle: AppHandle) {
    std::thread::spawn(move || {
        use windows::Win32::Media::Audio::{
            IMMDeviceEnumerator, IMMDevice, IAudioClient, IAudioCaptureClient,
            eRender, eConsole, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
        };
        use windows::Win32::System::Com::{
            CoInitializeEx, CoTaskMemFree,
            COINIT_APARTMENTTHREADED, CoCreateInstance, CLSCTX_ALL
        };
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

            const NUM_BANDS: usize = 5;
            let mut max_band_energies = [0.01f32; NUM_BANDS];
            let mut prev_values = [0.1f32; NUM_BANDS];

            let mut failures = 0u32;
            loop {
                if !audio_visualization_wanted(&app_handle) {
                    std::thread::sleep(crate::memory_saver::poll_interval(std::time::Duration::from_millis(1000)));
                    continue;
                }
                let result: Result<(), String> = (|| {
                    let enumerator: IMMDeviceEnumerator =
                        CoCreateInstance(&windows::Win32::Media::Audio::MMDeviceEnumerator, None, CLSCTX_ALL)
                            .map_err(|e| format!("CoCreateInstance failed: {:?}", e))?;

                    let device: IMMDevice = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)
                        .map_err(|e| format!("GetDefaultAudioEndpoint failed: {:?}", e))?;

                    let current_id_str = if let Ok(id) = device.GetId() {
                        let id_str = windows::core::PCWSTR::from_raw(id.0).to_string().unwrap_or_default();
                        CoTaskMemFree(Some(id.0 as *const _));
                        id_str
                    } else { String::new() };

                    let audio_client: IAudioClient = device.Activate(CLSCTX_ALL, None)
                        .map_err(|e| format!("Activate failed: {:?}", e))?;

                    let format_ptr = audio_client.GetMixFormat()
                        .map_err(|e| format!("GetMixFormat failed: {:?}", e))?;
                    struct MixFormatGuard(*mut windows::Win32::Media::Audio::WAVEFORMATEX);
                    impl Drop for MixFormatGuard {
                        fn drop(&mut self) {
                            unsafe { CoTaskMemFree(Some(self.0 as *const _)); }
                        }
                    }
                    let _format_guard = MixFormatGuard(format_ptr);

                    let channels = (*format_ptr).nChannels as usize;
                    let bits_per_sample = (*format_ptr).wBitsPerSample;
                    let bytes_per_sample = (bits_per_sample / 8) as usize;
                    let is_float = is_ieee_float_format(format_ptr);

                    if channels == 0
                        || (is_float && bits_per_sample != 32)
                        || (!is_float && bits_per_sample != 16 && bits_per_sample != 32)
                    {
                        return Err(format!(
                            "Invalid audio format: channels={}, bits={}, float={}",
                            channels, bits_per_sample, is_float
                        ));
                    }

                    let buffer_duration = 10_000_000i64;
                    audio_client.Initialize(
                        AUDCLNT_SHAREMODE_SHARED,
                        AUDCLNT_STREAMFLAGS_LOOPBACK,
                        buffer_duration,
                        0,
                        format_ptr,
                        Some(std::ptr::null()),
                    ).map_err(|e| format!("Initialize failed: {:?}", e))?;

                    drop(_format_guard);

                    let capture_client: IAudioCaptureClient = audio_client.GetService()
                        .map_err(|e| format!("GetService failed: {:?}", e))?;

                    audio_client.Start()
                        .map_err(|e| format!("Start failed: {:?}", e))?;

                    const FFT_SIZE: usize = 512;
                    let mut fft_buffer = vec![0.0f32; FFT_SIZE];
                    let mut fft_input: Vec<rustfft::num_complex::Complex<f32>> = vec![rustfft::num_complex::Complex::new(0.0, 0.0); FFT_SIZE];
                    let mut buffer_pos = 0;

                    let mut planner = rustfft::FftPlanner::<f32>::new();
                    let fft = planner.plan_fft_forward(FFT_SIZE);

                    let mut last_device_check = std::time::Instant::now();
                    let mut idle_since = std::time::Instant::now();
                    loop {
                        let idle = !ANY_MEDIA_PLAYING.load(Ordering::Relaxed);
                        if !idle {
                            idle_since = std::time::Instant::now();
                        } else if idle_since.elapsed() > std::time::Duration::from_secs(15) {
                            return Ok(());
                        }
                        std::thread::sleep(std::time::Duration::from_millis(if idle { 200 } else { 32 }));

                        if last_device_check.elapsed().as_secs() >= 2 {
                            last_device_check = std::time::Instant::now();
                            if let Ok(new_device) = enumerator.GetDefaultAudioEndpoint(eRender, eConsole) {
                                if let Ok(new_id) = new_device.GetId() {
                                    let new_str = windows::core::PCWSTR::from_raw(new_id.0).to_string().unwrap_or_default();
                                    CoTaskMemFree(Some(new_id.0 as *const _));
                                    if current_id_str != new_str {
                                        return Err("Default device changed".into());
                                    }
                                }
                            }
                        }
                        
                        // Skip all heavy processing if nothing is playing
                        if !ANY_MEDIA_PLAYING.load(Ordering::Relaxed) {
                            // Still gotta clear the buffer to avoid lag when it starts
                            loop {
                                let len = match capture_client.GetNextPacketSize() {
                                    Ok(l) => l,
                                    Err(_) => {
                                        return Err("Device invalidated".into());
                                    }
                                };
                                if len == 0 { break; }
                                let mut data_ptr: *mut u8 = std::ptr::null_mut();
                                let mut num_frames = 0u32;
                                let mut flags = 0u32;
                                if capture_client.GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None).is_err() {
                                    return Err("Device invalidated".into());
                                }
                                let _ = capture_client.ReleaseBuffer(num_frames);
                            }
                            continue;
                        }

                        let mut device_invalidated = false;
                        loop {
                            let packet_length = match capture_client.GetNextPacketSize() {
                                Ok(len) => len,
                                Err(_) => { 
                                    device_invalidated = true; 
                                    break; 
                                },
                            };
                            if packet_length == 0 { break; }
                            let mut data_ptr: *mut u8 = std::ptr::null_mut();
                            let mut num_frames = 0u32;
                            let mut flags = 0u32;

                            if capture_client.GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None).is_err() {
                                device_invalidated = true;
                                break;
                            }

                            if num_frames > 0 && bytes_per_sample > 0 && channels > 0 {
                                let is_silent = (flags & (windows::Win32::Media::Audio::AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)) != 0
                                    || data_ptr.is_null();
                                let stride = channels * bytes_per_sample;
                                let raw_slice = if is_silent {
                                    &[]
                                } else {
                                    std::slice::from_raw_parts(data_ptr, num_frames as usize * stride)
                                };
                                for frame in 0..num_frames as usize {
                                    let normalized = if is_silent {
                                        0.0
                                    } else {
                                        let frame_bytes = &raw_slice[frame * stride..(frame + 1) * stride];
                                        decode_audio_frame(frame_bytes, channels, bytes_per_sample, is_float)
                                    };
                                    if buffer_pos < FFT_SIZE {
                                        let window = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * buffer_pos as f32 / FFT_SIZE as f32).cos());
                                        fft_buffer[buffer_pos] = normalized * window;
                                        buffer_pos += 1;
                                    }
                                    if buffer_pos >= FFT_SIZE {
                                        // Run FFT on the windowed buffer
                                        for (i, &sample) in fft_buffer.iter().enumerate() {
                                            fft_input[i] = rustfft::num_complex::Complex::new(sample, 0.0);
                                        }
                                        fft.process(&mut fft_input);

                                        let band_ranges = [(1, 2), (2, 6), (6, 18), (18, 60), (60, 200)];
                                        let mut output = [0.0f32; NUM_BANDS];
                                        for (band_idx, &(bin_start, bin_end)) in band_ranges.iter().enumerate() {
                                            let mut total_mag = 0.0f32;
                                            let mut count = 0u32;
                                            for bin in bin_start..bin_end {
                                                if bin >= FFT_SIZE / 2 { break; }
                                                let mag = fft_input[bin].norm();
                                                total_mag += mag;
                                                count += 1;
                                            }
                                            let avg_mag = total_mag / count.max(1) as f32;
                                            let mut scaled_mag = avg_mag;
                                            let weighting = [1.2, 1.2, 1.5, 2.8, 5.0];
                                            scaled_mag *= weighting[band_idx];
                                            if scaled_mag > max_band_energies[band_idx] {
                                                max_band_energies[band_idx] = scaled_mag;
                                            } else {
                                                max_band_energies[band_idx] *= 0.99;
                                            }
                                            let target = (scaled_mag / max_band_energies[band_idx].max(0.12)).min(1.0).powf(0.75);
                                            let is_rising = target > prev_values[band_idx];
                                            let smooth_factor = if is_rising { 0.10 } else { 0.20 };
                                            output[band_idx] = prev_values[band_idx] * smooth_factor + target * (1.0 - smooth_factor);
                                            output[band_idx] = output[band_idx].clamp(0.18, 1.0);
                                            prev_values[band_idx] = output[band_idx];
                                        }
                                        let _ = app_handle.emit("audio-visualization", AudioVisualizationData { frequencies: output.to_vec() });
                                        buffer_pos = 0;
                                    }
                                }
                                let _ = capture_client.ReleaseBuffer(num_frames);
                            }
                        }
                        
                        if device_invalidated {
                            return Err("Device invalidated".into());
                        }
                    }
                })();

                if let Err(e) = &result {
                    if e.contains("0x80070005") {
                        AUDIO_CAPTURE_REFUSED.store(true, Ordering::Relaxed);
                        crate::diagnostics::log("audio visualizer: Windows refused the capture stream, not retrying this session");
                    }
                }
                failures = match &result {
                    Err(e) if e.contains(" failed") => (failures + 1).min(8),
                    _ => 0,
                };
                let wait_ms = 2500u64 << failures.min(7);
                std::thread::sleep(std::time::Duration::from_millis(wait_ms.min(300_000)));
            }
            // if com_initialized { CoUninitialize(); }
        }
    });
}

pub fn setup_system_worker(app_handle: AppHandle) -> Sender<SystemCommand> {
    let (tx, rx) = channel::<SystemCommand>();
    let handle_system = app_handle.clone();
    std::thread::spawn(move || {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED, CoCreateInstance, CLSCTX_ALL, CoTaskMemFree};
        use windows::Win32::Media::Audio::{IMMDeviceEnumerator, eRender, eConsole};
        use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
        use windows::Media::Control::{GlobalSystemMediaTransportControlsSessionManager, GlobalSystemMediaTransportControlsSessionPlaybackStatus};
        use base64::{Engine as _, engine::general_purpose};
        use windows::Storage::Streams::DataReader;

        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator = CoCreateInstance::<_, IMMDeviceEnumerator>(&windows::Win32::Media::Audio::MMDeviceEnumerator, None, CLSCTX_ALL).ok();
            let mut device = enumerator.as_ref().and_then(|e| e.GetDefaultAudioEndpoint(eRender, eConsole).ok());
            let mut audio_endpoint_volume: Option<IAudioEndpointVolume> = device.as_ref().and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok());
            
            let mut current_device_id = String::new();
            if let Some(ref d) = device {
                if let Ok(id) = d.GetId() {
                    current_device_id = windows::core::PCWSTR::from_raw(id.0).to_string().unwrap_or_default();
                    CoTaskMemFree(Some(id.0 as *const _));
                }
            }

            let mut manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.get()).ok();
            let mut last_processed_media = std::time::Instant::now();
            let mut media_burst_until: Option<std::time::Instant> = None;
            let mut last_device_check = std::time::Instant::now();
            #[allow(clippy::type_complexity)]
            let mut last_emitted_info: Option<(String, String, bool, bool, Option<String>, i64, i64)> = None;
            let mut art_attempts: (String, u32) = (String::new(), 0);
            let mut last_volume: f32 = -1.0;
            let mut last_sessions_json = String::new();
            let mut last_muted: bool = false;

            // Only hide the native flyout when our overlay replaces it.
            let hide_osd = |overlay_key: &str| {
                if !overlay_enabled(&handle_system, overlay_key) { return; }
                use windows::Win32::UI::WindowsAndMessaging::{FindWindowA, ShowWindow, SW_HIDE};
                let class1 = windows::core::PCSTR(c"NativeHWNDHost".as_ptr() as *const u8);
                if let Ok(hwnd1) = FindWindowA(class1, windows::core::PCSTR::null()) { let _ = ShowWindow(hwnd1, SW_HIDE); }
            };

            loop {
                // Check for device change
                if last_device_check.elapsed().as_secs() >= 2 {
                    last_device_check = std::time::Instant::now();
                    if let Some(ref enum_ref) = enumerator {
                        if let Ok(new_device) = enum_ref.GetDefaultAudioEndpoint(eRender, eConsole) {
                            if let Ok(id) = new_device.GetId() {
                                let new_id = windows::core::PCWSTR::from_raw(id.0).to_string().unwrap_or_default();
                                CoTaskMemFree(Some(id.0 as *const _));
                                if new_id != current_device_id || audio_endpoint_volume.is_none() {
                                    current_device_id = new_id;
                                    device = Some(new_device);
                                    audio_endpoint_volume = device.as_ref().and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok());
                                    // Reset last_volume to force an update event
                                    last_volume = -1.0;
                                }
                            }
                        }
                    }
                }

                if manager.is_none() { manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.get()).ok(); }
                while let Ok(cmd) = rx.try_recv() {
                    // Only the volume commands need the audio endpoint, the rest must work with no output device.
                    {
                        match cmd {
                            SystemCommand::VolumeMute => { if let Some(aev) = audio_endpoint_volume.as_ref() { if let Ok(muted) = aev.GetMute() { let _ = aev.SetMute(!muted.as_bool(), std::ptr::null()); hide_osd("fern-volume-overlay-enabled"); } } }
                            SystemCommand::VolumeUp => {
                                if let Some(aev) = audio_endpoint_volume.as_ref() {
                                    if let (Ok(vol), Ok(muted)) = (aev.GetMasterVolumeLevelScalar(), aev.GetMute()) {
                                        let _ = aev.SetMasterVolumeLevelScalar((vol + 0.05).min(1.0), std::ptr::null());
                                        if muted.as_bool() { let _ = aev.SetMute(false, std::ptr::null()); }
                                        hide_osd("fern-volume-overlay-enabled");
                                    }
                                }
                            }
                            SystemCommand::VolumeDown => { if let Some(aev) = audio_endpoint_volume.as_ref() { if let Ok(vol) = aev.GetMasterVolumeLevelScalar() { let _ = aev.SetMasterVolumeLevelScalar((vol - 0.05).max(0.0), std::ptr::null()); hide_osd("fern-volume-overlay-enabled"); } } }
                            SystemCommand::SetVolume(volume) => {
                                if let Some(aev) = audio_endpoint_volume.as_ref() {
                                    let _ = aev.SetMasterVolumeLevelScalar(volume.clamp(0.0, 1.0), std::ptr::null());
                                    if volume > 0.0 { let _ = aev.SetMute(false, std::ptr::null()); }
                                    hide_osd("fern-volume-overlay-enabled");
                                }
                            }
                            SystemCommand::BrightnessUp => {
                                let new_val = (CURRENT_BRIGHTNESS.load(Ordering::Relaxed) + 10).min(100);
                                CURRENT_BRIGHTNESS.store(new_val, Ordering::Relaxed);
                                LAST_BRIGHTNESS_CHANGE.store(get_now_ms(), Ordering::Relaxed);
                                let _ = handle_system.emit("brightness-change", BrightnessChangeEvent { brightness: new_val });
                                if let Some(tx) = BRIGHTNESS_SENDER.get() { let _ = tx.send(new_val); }
                                hide_osd("fern-brightness-overlay-enabled");
                            }
                            SystemCommand::BrightnessDown => {
                                let current = CURRENT_BRIGHTNESS.load(Ordering::Relaxed);
                                let new_val = current.saturating_sub(10);
                                CURRENT_BRIGHTNESS.store(new_val, Ordering::Relaxed);
                                LAST_BRIGHTNESS_CHANGE.store(get_now_ms(), Ordering::Relaxed);
                                let _ = handle_system.emit("brightness-change", BrightnessChangeEvent { brightness: new_val });
                                if let Some(tx) = BRIGHTNESS_SENDER.get() { let _ = tx.send(new_val); }
                                hide_osd("fern-brightness-overlay-enabled");
                            }
                            SystemCommand::MediaPlayPause => { if let Some(ref mgr) = manager { if let Ok(session) = media_control_session(mgr) { let _ = session.TryTogglePlayPauseAsync(); MEDIA_REFRESH.store(true, Ordering::Relaxed); media_burst_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(1500)); } } }
                            SystemCommand::MediaNext => { if let Some(ref mgr) = manager { if let Ok(session) = media_control_session(mgr) { let _ = session.TrySkipNextAsync(); MEDIA_REFRESH.store(true, Ordering::Relaxed); media_burst_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(1500)); } } }
                            SystemCommand::MediaPrevious => { if let Some(ref mgr) = manager { if let Ok(session) = media_control_session(mgr) { let _ = session.TrySkipPreviousAsync(); MEDIA_REFRESH.store(true, Ordering::Relaxed); media_burst_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(1500)); } } }
                            SystemCommand::MediaSeek(position_ms) => {
                                if let Some(ref mgr) = manager {
                                    if let Ok(session) = media_control_session(mgr) {
                                        let ticks = position_ms * 10_000;
                                        let _ = session.TryChangePlaybackPositionAsync(ticks);
                                        MEDIA_REFRESH.store(true, Ordering::Relaxed);
                                        media_burst_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(1500));
                                    }
                                }
                            }
                            SystemCommand::ToggleVisibility(visible) => {
                                let _ = handle_system.emit("visibility-change", visible);
                                if let Some(w) = handle_system.get_webview_window("bottom-corners") { if visible { let _ = w.show(); } else { let _ = w.hide(); } }
                            }
                        }
                    }
                }
                if let Some(ref aev) = audio_endpoint_volume {
                    if let (Ok(vol), Ok(muted)) = (aev.GetMasterVolumeLevelScalar(), aev.GetMute()) {
                        let is_muted: bool = muted.into();
                        if (vol - last_volume).abs() > 0.001 || is_muted != last_muted {
                            last_volume = vol; last_muted = is_muted;
                            crate::state::CURRENT_VOLUME.store((vol * 100.0) as u32, Ordering::Relaxed);
                            let _ = handle_system.emit("volume-change", VolumeChangeEvent { volume: vol, is_muted });
                            hide_osd("fern-volume-overlay-enabled");
                        }
                    }
                }
                let media_poll_ms = if media_burst_until.is_some_and(|t| t > std::time::Instant::now()) { 100 } else { 400 };
                if last_processed_media.elapsed().as_millis() >= media_poll_ms || MEDIA_REFRESH.swap(false, Ordering::Relaxed) {
                    last_processed_media = std::time::Instant::now();
                    let mut best_info: Option<MediaInfo> = None;
                    if let Some(ref mgr) = manager {
                        if let Ok(sessions) = mgr.GetSessions() {
                            let current_session = mgr.GetCurrentSession().ok().and_then(|s| {
                                let cid = s.SourceAppUserModelId().map(|v| v.to_string()).unwrap_or_default();
                                let ctitle = s
                                    .TryGetMediaPropertiesAsync()
                                    .and_then(|op| op.get())
                                    .and_then(|p| p.Title())
                                    .map(|t| t.to_string())
                                    .unwrap_or_default();
                                if cid.is_empty() || ctitle.is_empty() { None } else { Some((cid, ctitle)) }
                            });
                            let mut entries: Vec<MediaSessionCandidate> = Vec::new();
                            for i in 0..sessions.Size().unwrap_or(0) {
                                if let Ok(session) = sessions.GetAt(i) {
                                    let title = session
                                        .TryGetMediaPropertiesAsync()
                                        .and_then(|op| op.get())
                                        .and_then(|p| p.Title())
                                        .map(|t| t.to_string())
                                        .unwrap_or_default();
                                    if title.is_empty() { continue; }
                                    let id = session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default();
                                    let playing = session.GetPlaybackInfo().ok().and_then(|p| p.PlaybackStatus().ok()) == Some(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing);
                                    let last_updated_ticks = session
                                        .GetTimelineProperties()
                                        .ok()
                                        .and_then(|tl| tl.LastUpdatedTime().ok())
                                        .map(|dt| dt.UniversalTime)
                                        .unwrap_or(0);
                                    entries.push(MediaSessionCandidate {
                                        index: i,
                                        source_id: id,
                                        title,
                                        is_playing: playing,
                                        last_updated_ticks,
                                    });
                                }
                            }
                            let selected = SELECTED_MEDIA_SOURCE.lock().ok().and_then(|g| g.clone());
                            let prev_active = ACTIVE_MEDIA_SOURCE.lock().ok().map(|g| g.clone()).unwrap_or_default();
                            let prev_title = ACTIVE_MEDIA_TITLE.lock().ok().map(|g| g.clone()).unwrap_or_default();
                            let chosen_i = choose_media_session_index(
                                &entries,
                                selected.as_deref(),
                                &prev_active,
                                &prev_title,
                                current_session.as_ref().map(|(id, t)| (id.as_str(), t.as_str())),
                            );
                            let chosen_entry = entries.iter().find(|e| Some(e.index) == chosen_i);
                            let active_id = chosen_entry.map(|e| e.source_id.clone()).unwrap_or_default();
                            let active_title = chosen_entry.map(|e| e.title.clone()).unwrap_or_default();
                            if let Ok(mut g) = ACTIVE_MEDIA_SOURCE.lock() { *g = active_id.clone(); }
                            if let Ok(mut g) = ACTIVE_MEDIA_TITLE.lock() { *g = active_title; }
                            let list: Vec<serde_json::Value> = entries
                                .iter()
                                .map(|e| serde_json::json!({ "id": e.source_id, "name": media_app_name(&e.source_id), "title": e.title, "is_playing": e.is_playing, "active": Some(e.index) == chosen_i }))
                                .collect();
                            let list_json = serde_json::to_string(&list).unwrap_or_default();
                            if list_json != last_sessions_json {
                                let _ = handle_system.emit("media-sessions", list);
                                last_sessions_json = list_json;
                            }
                            for i in 0..sessions.Size().unwrap_or(0) {
                                if Some(i) != chosen_i { continue; }
                                if let Ok(session) = sessions.GetAt(i) {
                                    let is_playing = session.GetPlaybackInfo().ok().and_then(|p| p.PlaybackStatus().ok()) == Some(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing);
                                    if let Ok(props) = session.TryGetMediaPropertiesAsync().and_then(|op| op.get()) {
                                        let title = props.Title().unwrap_or_default().to_string();
                                        if !title.is_empty() {
                                            let artist = props.Artist().unwrap_or_default().to_string();
                                            let mut artwork = None;
                                            let mut got_art_from_cache = false;
                                            let track_key = format!("{}|{}", title, artist);
                                            if art_attempts.0 != track_key {
                                                art_attempts = (track_key, 0);
                                            }
                                            if let Some((ref last_title, ref last_artist, _, _, ref last_art, _, _)) = last_emitted_info {
                                                // A track whose artwork failed to load is retried every poll at first, then every 15th poll, the player may not have it ready yet
                                                let retry_due = should_retry_artwork(art_attempts.1);
                                                if last_title == &title && last_artist == &artist && (last_art.is_some() || !retry_due) {
                                                    artwork = last_art.as_ref().map(|art| vec![art.clone()]);
                                                    got_art_from_cache = true;
                                                    if last_art.is_none() {
                                                        art_attempts.1 += 1;
                                                    }
                                                }
                                            }

                                            if !got_art_from_cache {
                                                artwork = (|| -> Option<Vec<String>> {
                                                    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
                                                    let content_type = stream.ContentType().ok()?.to_string().split(',').next().unwrap_or("image/jpeg").trim().to_string();
                                                    let reader = DataReader::CreateDataReader(&stream).ok()?;
                                                    let mut all_bytes = Vec::new();
                                                    let chunk_size = 65536u32;
                                                    loop {
                                                        let loaded = reader.LoadAsync(chunk_size).ok()?.get().ok()?;
                                                        if loaded == 0 { break; }
                                                        let mut chunk = vec![0u8; loaded as usize];
                                                        reader.ReadBytes(&mut chunk).ok()?;
                                                        all_bytes.extend_from_slice(&chunk);
                                                    }
                                                    if all_bytes.is_empty() { return None; }
                                                    Some(vec![format!("data:{};base64,{}", content_type, general_purpose::STANDARD.encode(all_bytes))])
                                                })();
                                                if artwork.is_none() {
                                                    art_attempts.1 += 1;
                                                }
                                            }

                                            // Extract timeline properties for progress bar
                                            let (position_ms, duration_ms, seek_enabled) = {
                                                let defaults = (0i64, 0i64, false);
                                                match session.GetTimelineProperties() {
                                                    Ok(timeline) => {
                                                        let start = timeline.StartTime().map(|t| t.Duration / 10_000).unwrap_or(0);
                                                        let end = timeline.EndTime().map(|t| t.Duration / 10_000).unwrap_or(0);
                                                        let pos = timeline.Position().map(|t| t.Duration / 10_000).unwrap_or(0);
                                                        let dur = (end - start).max(0);
                                                        // If duration is 0 but position > 0, some players don't report start/end
                                                        // but still track position — use position as fallback duration indicator
                                                        let effective_dur = if dur > 0 { dur } else { 0 };
                                                        // Check if seeking is supported
                                                        let seek = session.GetPlaybackInfo().ok()
                                                            .and_then(|pi| pi.Controls().ok())
                                                            .map(|c| c.IsPlaybackPositionEnabled().unwrap_or(false))
                                                            .unwrap_or(false);
                                                        (pos, effective_dur, seek)
                                                    }
                                                    Err(_) => defaults,
                                                }
                                            };

                                            let now_ms = std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_millis() as u64)
                                                .unwrap_or(0);
                                            let info = MediaInfo { title, artist, is_playing, has_media: true, artwork, position_ms, duration_ms, seek_enabled, position_updated_at: now_ms, source_id: active_id.clone() };
                                            if is_playing { best_info = Some(info); break; } else if best_info.is_none() { best_info = Some(info); }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let current = best_info.unwrap_or(MediaInfo { title: "".into(), artist: "".into(), is_playing: false, has_media: false, artwork: None, position_ms: 0, duration_ms: 0, seek_enabled: false, position_updated_at: 0, source_id: String::new() });
                    let art_str = current.artwork.as_ref().and_then(|a| a.first()).cloned();
                    if last_emitted_info.as_ref().is_none_or(|(t, a, p, h, art, pos, dur)| {
                        t != &current.title || a != &current.artist || p != &current.is_playing || h != &current.has_media || art != &art_str || dur != &current.duration_ms ||
                        // For position: emit if >1s difference (avoid spamming on every poll)
                        (*pos - current.position_ms).abs() > 1000
                    }) {
                        if let Ok(mut guard) = crate::state::LAST_MEDIA.lock() {
                            *guard = Some(current.clone());
                        }
                        let _ = handle_system.emit("media-update", current.clone());
                        ANY_MEDIA_PLAYING.store(current.is_playing, Ordering::Relaxed);
                        last_emitted_info = Some((current.title, current.artist, current.is_playing, current.has_media, art_str, current.position_ms, current.duration_ms));
                    }
                }
                std::thread::sleep(crate::memory_saver::poll_interval(std::time::Duration::from_millis(64)));
            }
        }
    });

    let handle_brightness = app_handle.clone();
    std::thread::spawn(move || {
        let com_lib = match COMLibrary::new() { Ok(lib) => lib, Err(_) => return };
        let wmi_con = match WMIConnection::with_namespace_path("root\\WMI", com_lib) { Ok(con) => con, Err(_) => return };
        let first = wmi_con.query::<WmiMonitorBrightness>();
        let supported = first.as_ref().map(|r| !r.is_empty()).unwrap_or(false);
        let poll = if supported { 1500 } else { 30_000 };
        let saved_brightness = if supported { None } else {
            handle_brightness.path().app_config_dir().ok()
                .and_then(|d| std::fs::read_to_string(d.join("last_brightness.txt")).ok())
                .and_then(|t| t.trim().parse::<u32>().ok())
                .map(|v| v.min(100))
        };
        let mut last_brightness = match first.ok().and_then(|res| res.first().map(|b| b.current_brightness as u32)) { Some(v) => v, None => saved_brightness.unwrap_or(50) };
        CURRENT_BRIGHTNESS.store(last_brightness, Ordering::Relaxed);
        loop {
            if get_now_ms() - LAST_BRIGHTNESS_CHANGE.load(Ordering::Relaxed) < 2000 { std::thread::sleep(std::time::Duration::from_millis(500)); continue; }
            if let Ok(results) = wmi_con.query::<WmiMonitorBrightness>() {
                if let Some(b) = results.first() {
                    let brightness = b.current_brightness as u32;
                    if brightness != last_brightness { last_brightness = brightness; CURRENT_BRIGHTNESS.store(brightness, Ordering::Relaxed); let _ = handle_brightness.emit("brightness-change", BrightnessChangeEvent { brightness }); }
                }
            }
            std::thread::sleep(crate::memory_saver::poll_interval(std::time::Duration::from_millis(poll)));
        }
    });

    let tx_clone = tx.clone();
    let handle_visibility = app_handle.clone();
    std::thread::spawn(move || {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, IsZoomed, IsIconic, GetWindowLongW, GWL_STYLE, WS_MAXIMIZE, GetClientRect};
        use windows::Win32::Graphics::Gdi::ClientToScreen;
        use windows::Win32::Foundation::{RECT, POINT};
        let _ = VISIBILITY_THREAD.set(std::thread::current());
        let mut last_visible = true;
        let mut last_dock_overlap: Option<bool> = None;
        let mut last_dock_fullscreen: Option<bool> = None;
        let mut last_dock_shell_surface: Option<bool> = None;
        let mut last_notch_fullscreen: Option<bool> = None;
        let mut last_secondary_dock_fullscreen = std::collections::HashMap::<String, (bool, Instant)>::new();
        let mut last_secondary_dock_shell_surface = std::collections::HashMap::<String, (bool, Instant)>::new();
        let mut last_notch_overlap: Option<bool> = None;
        let mut last_dock_maximized: Option<bool> = None;
        let mut last_fg_maximized = false;
        let mut last_hwnd = HWND(std::ptr::null_mut());
        let mut last_emit = Instant::now();
        let mut is_known_shell = false;
        let mut is_desktop_focused = false;
        let my_process_id = std::process::id();
        let mut last_monitor_update = Instant::now() - Duration::from_secs(5);
        let mut cached_scale = 1.0f64;
        let mut last_start_check = HWND(std::ptr::null_mut());
        let mut start_open = false;
        let mut start_focused_at = Instant::now();

        loop {
            if SHUTTING_DOWN.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(150));
                continue;
            }
            unsafe {
                let now = Instant::now();
                // Start and Search open above the screen's bottom edge, where the dock would cover
                // them. The check opens the process, so only redo it when the foreground changes.
                let foreground = GetForegroundWindow();
                if foreground != last_start_check {
                    last_start_check = foreground;
                    start_open = crate::commands::is_start_menu_window(foreground);
                    start_focused_at = now;
                }
                let root_fg = windows::Win32::UI::WindowsAndMessaging::GetAncestor(foreground, windows::Win32::UI::WindowsAndMessaging::GA_ROOT);
                let target_fg = if !root_fg.is_invalid() && !root_fg.0.is_null() { root_fg } else { foreground };
                let start_on_screen = start_open && IsWindowVisible(target_fg).as_bool() && {
                    let mut cloaked = 0u32;
                    DwmGetWindowAttribute(target_fg, windows::Win32::Graphics::Dwm::DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_err() || cloaked == 0
                };
                let start_shown = start_counts_as_shown(start_open, start_on_screen, now.duration_since(start_focused_at));
                let hide_on_start = crate::utils::get_setting_str(&handle_visibility, "fern-dock-hide-on-start")
                    .map(|v| v != "false")
                    .unwrap_or(true);
                let start_tap_monitor = if !hide_on_start
                    && !start_shown
                    && now_ms() - START_TAP_MS.load(Ordering::Relaxed) < START_TAP_GRACE_MS
                {
                    cursor_monitor_origin()
                } else {
                    None
                };
                let start_monitor = if start_shown { monitor_rect_for_hwnd(target_fg) } else { None };
                let start_on = |window: HWND| {
                    let win_mon = monitor_rect_for_hwnd(window);
                    let detected = match (start_monitor, win_mon) {
                        (Some((sx, sy, _, _)), Some((x, y, _, _))) => sx == x && sy == y,
                        _ => false,
                    };
                    let tapped = match (start_tap_monitor, win_mon) {
                        (Some((cx, cy)), Some((x, y, _, _))) => cx == x && cy == y,
                        _ => false,
                    };
                    detected || tapped
                };
                if now.duration_since(last_monitor_update) > Duration::from_millis(1000) {
                    if let Some(m) = handle_visibility.primary_monitor().ok().flatten() {
                        cached_scale = m.scale_factor();
                        last_monitor_update = now;
                    }
                }
                use windows::Win32::Graphics::Gdi::{MonitorFromWindow, GetMonitorInfoA, MONITORINFO, MONITOR_DEFAULTTONEAREST};
                let settings_hwnd = handle_visibility.get_webview_window("settings").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize);
                let calendar_hwnd = handle_visibility.get_webview_window("calendar").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize);
                use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, WS_EX_TOOLWINDOW};
                let mut hwnd = GetForegroundWindow();
                
                // Find the first meaningful window for overlap detection.
                let mut check_count = 0;
                let mut found_meaningful = false;
                while !hwnd.is_invalid() && check_count < 15 {
                    let mut process_id = 0u32;
                    GetWindowThreadProcessId(hwnd, Some(&mut process_id));
                    
                    let hwnd_val = hwnd.0 as isize;
                    let is_user_panel = settings_hwnd == Some(hwnd_val) || calendar_hwnd == Some(hwnd_val);
                    let is_fern = process_id == my_process_id && !is_user_panel;
                    let is_visible = IsWindowVisible(hwnd).as_bool();
                    let is_iconic = IsIconic(hwnd).as_bool();
                    
                    let mut cloaked = 0u32;
                    let is_cloaked = DwmGetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok() && cloaked != 0;

                    let mut rect = RECT::default();
                    let has_valid_rect = GetWindowRect(hwnd, &mut rect).is_ok() && (rect.right - rect.left) > 0 && (rect.bottom - rect.top) > 0;
                    let is_bg_toolwindow = check_count > 0
                        && (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0) != 0;
                    
                    if is_fern || !is_visible || is_iconic || is_cloaked || !has_valid_rect || is_bg_toolwindow {
                        hwnd = windows::Win32::UI::WindowsAndMessaging::GetWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::GW_HWNDNEXT).unwrap_or_default();
                        check_count += 1;
                    } else {
                        found_meaningful = true;
                        break;
                    }
                }
                if !found_meaningful {
                    hwnd = HWND(std::ptr::null_mut());
                }

                let mut should_overlap = false;
                let mut should_notch_overlap = false;
                let mut should_maximized = false;
                // Foreground window's monitor, rect and whether it covers that monitor (maximized or fullscreen).
                let mut fg_info: Option<(RECT, RECT, bool)> = None;
                let mut current_is_fs = false;

                // Checked every tick, the foreground hwnd doesn't change on maximize so the adaptive dock
                // would wait for the 3s refresh.
                let fg_is_maximized = !hwnd.is_invalid() && IsZoomed(hwnd).as_bool();

                if !hwnd.is_invalid() && (hwnd != last_hwnd || fg_is_maximized != last_fg_maximized || last_emit.elapsed() >= Duration::from_secs(3)) {
                    last_hwnd = hwnd;
                    last_fg_maximized = fg_is_maximized;
                    let mut class_name = [0u8; 256];
                    let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameA(hwnd, &mut class_name);
                    let class_str = std::str::from_utf8(&class_name[..len as usize]).unwrap_or("");
                    let mut text = [0u16; 512];
                    let text_len = windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut text);
                    let title = String::from_utf16_lossy(&text[..text_len as usize]);

                    let is_desktop = class_str == "Progman" || class_str == "WorkerW";
                    let is_start = (hwnd == foreground && start_open)
                        || ((class_str == "Windows.UI.Core.CoreWindow" || class_str == "SimpleWindow")
                            && (title == "Start" || title == "Search"));
                    let is_shell = class_str == "Shell_TrayWnd" || class_str == "Shell_SecondaryTrayWnd" || is_start;
                    
                    is_known_shell = is_desktop || is_shell;
                    is_desktop_focused = is_desktop;
                }

                if !hwnd.is_invalid() {
                    if is_known_shell {
                        should_overlap = false;
                        current_is_fs = false;
                    } else {
                        // Current hwnd is now guaranteed to be visible, non-iconic and non-cloaked
                        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
                        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                        
                        let is_transient = (ex_style & WS_EX_TOOLWINDOW.0) != 0;
                        
                        if !is_transient {
                            let mut rect = RECT::default();
                            let dwm_res = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32);
                            let has_rect = dwm_res.is_ok() || GetWindowRect(hwnd, &mut rect).is_ok();

                            if has_rect {
                                let h_monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                                
                                if GetMonitorInfoA(h_monitor, &mut mi).as_bool() {
                                    let screen_rect = mi.rcMonitor;
                                    let is_maximized = IsZoomed(hwnd).as_bool() || (style & WS_MAXIMIZE.0) != 0;

                                    
                                    let mut is_client_fullscreen = false;
                                    let mut client_rect = RECT::default();
                                    if GetClientRect(hwnd, &mut client_rect).is_ok() {
                                        let mut top_left = POINT { x: client_rect.left, y: client_rect.top };
                                        let mut bottom_right = POINT { x: client_rect.right, y: client_rect.bottom };
                                        let _ = ClientToScreen(hwnd, &mut top_left);
                                        let _ = ClientToScreen(hwnd, &mut bottom_right);
                                        
                                        is_client_fullscreen = top_left.x <= screen_rect.left + 2 && top_left.y <= screen_rect.top + 2 &&
                                                               bottom_right.x >= screen_rect.right - 2 && bottom_right.y >= screen_rect.bottom - 2;
                                    }

                                    let is_matches_screen = rect.left <= screen_rect.left + 2 && rect.top <= screen_rect.top + 2 && 
                                                            rect.right >= screen_rect.right - 2 && rect.bottom >= screen_rect.bottom - 2;
                                    
                                    let is_d3d_fs = windows::Win32::UI::Shell::SHQueryUserNotificationState()
                                        .is_ok_and(|state| state == windows::Win32::UI::Shell::QUNS_RUNNING_D3D_FULL_SCREEN);

                                    current_is_fs = ((is_client_fullscreen || is_matches_screen) && !is_maximized) || is_d3d_fs;
                                    fg_info = Some((screen_rect, rect, current_is_fs || is_maximized));

                                    if current_is_fs || is_maximized {
                                        should_overlap = true;
                                        should_notch_overlap = true;
                                        // Maximized (not fullscreen) windows leave the dock strip empty, so it can stretch to a full taskbar.
                                        should_maximized = is_maximized && !current_is_fs;
                                    } else {
                                        should_overlap = false;
                                        if let Some(dr) = crate::state::dock_overlap_rect("dock") {
                                            should_overlap = rect_overlaps_dock(screen_rect.left, screen_rect.bottom, rect, dr, cached_scale);
                                        }

                                        if let Some(nr) = crate::state::notch_rects().lock().ok().and_then(|m| m.get("main").copied()) {
                                            should_notch_overlap = rect_overlaps_notch(screen_rect.left, screen_rect.top, rect, nr, cached_scale);
                                        }
                                    }
                                }
                            }
                        } else {
                            should_overlap = false;
                            current_is_fs = false;
                        }
                    }
                } else {
                    last_hwnd = hwnd;
                    should_overlap = false;
                    let is_d3d_fs = windows::Win32::UI::Shell::SHQueryUserNotificationState()
                        .is_ok_and(|state| state == windows::Win32::UI::Shell::QUNS_RUNNING_D3D_FULL_SCREEN);
                    current_is_fs = is_d3d_fs;
                    is_known_shell = false;
                    is_desktop_focused = false;
                }

                // Overlap only counts on the monitor the foreground window is on, unless another
                // monitor has a maximized/fullscreen/covering window in front on its z-order.
                let monitor_of = |label: &str| {
                    handle_visibility.get_webview_window(label)
                        .and_then(|w| w.hwnd().ok())
                        .and_then(monitor_rect_for_hwnd)
                };
                let fg_on = |label: &str| match (fg_info, monitor_of(label)) {
                    (Some((screen, _, _)), Some((mx, my, _, _))) => screen.left == mx && screen.top == my,
                    _ => true,
                };
                let focused_monitor = resolve_focused_monitor(
                    !hwnd.is_invalid(),
                    is_desktop_focused,
                    is_known_shell,
                    if is_known_shell { cursor_monitor_origin() } else { None },
                    fg_info.map(|(screen, _, _)| (screen.left, screen.top)),
                );
                let covers = scan_monitor_covers(my_process_id);
                let fullscreen_monitors: Vec<(i32, i32)> = covers.iter().filter(|c| c.fullscreen).map(|c| c.origin).collect();
                let unfocused_panel_overlap = |label: &str, is_dock: bool| -> Option<bool> {
                    let (mx, my, _, _) = monitor_of(label)?;
                    if focused_monitor == Some((mx, my)) {
                        return None;
                    }
                    let Some(cover) = covers.iter().find(|c| c.origin == (mx, my)) else { return Some(false) };
                    if cover.covered {
                        return Some(true);
                    }
                    Some(if is_dock {
                        crate::state::dock_overlap_rect(label)
                            .is_some_and(|dr| cover.rects.iter().any(|r| rect_overlaps_dock(mx, cover.bottom, *r, dr, cached_scale)))
                    } else {
                        crate::state::notch_rects().lock().ok().and_then(|m| m.get(label).copied())
                            .is_some_and(|nr| cover.rects.iter().any(|r| rect_overlaps_notch(mx, my, *r, nr, cached_scale)))
                    })
                };
                if !fg_on("dock") {
                    should_overlap = false;
                    should_maximized = false;
                }
                if !fg_on("main") {
                    should_notch_overlap = false;
                }
                if let Some(overlap) = unfocused_panel_overlap("dock", true) {
                    should_overlap = overlap;
                }
                if let Some(overlap) = unfocused_panel_overlap("main", false) {
                    should_notch_overlap = overlap;
                }
                should_notch_overlap = notch_overlap_during_start(start_shown, should_notch_overlap, last_notch_overlap.unwrap_or(should_notch_overlap));

                // Only report dock overlap when the dock window is actually visible.
                // On autostart, the overlap thread starts before init_dock shows the window,
                // and without this guard it emits dock-overlap:true which hides the dock
                // right after init_dock emits dock-overlap:false.
                let dock_visible = handle_visibility.get_webview_window("dock")
                    .is_some_and(|w| w.is_visible().unwrap_or(false));
                let effective_dock_overlap = should_overlap && dock_visible;

                crate::state::set_overlap(crate::state::dock_overlap(), "dock", if effective_dock_overlap { 1 } else { 0 });
                crate::state::set_overlap(crate::state::notch_overlap(), "main", if should_notch_overlap { 1 } else { 0 });
                CURRENT_FOREGROUND_FULLSCREEN.store(current_is_fs, Ordering::Relaxed);
                // Start counts as fullscreen for the dock when fern-dock-hide-on-start is enabled;
                // otherwise it reveals the dock on that monitor via dock-shell-surface.
                let start_on_dock = handle_visibility.get_webview_window("dock")
                    .and_then(|w| w.hwnd().ok())
                    .is_some_and(start_on);
                CURRENT_START_ON_DOCK.store(start_on_dock, Ordering::Relaxed);
                let fullscreen_on = |label: &str| match monitor_of(label) {
                    Some((mx, my, _, _)) => monitor_fullscreen((mx, my), focused_monitor, current_is_fs, &fullscreen_monitors),
                    None => current_is_fs && fg_on(label),
                };
                let fullscreen_on_dock = fullscreen_on("dock") || (hide_on_start && start_on_dock);
                crate::state::set_flag(crate::state::dock_fullscreen(), "dock", fullscreen_on_dock);
                CURRENT_FOREGROUND_FULLSCREEN_ON_DOCK.store(fullscreen_on_dock, Ordering::Relaxed);
                if Some(fullscreen_on_dock) != last_dock_fullscreen || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("dock", "dock-fullscreen", fullscreen_on_dock);
                    last_dock_fullscreen = Some(fullscreen_on_dock);
                }
                let shell_surface_on_dock = !hide_on_start && start_on_dock;
                if Some(shell_surface_on_dock) != last_dock_shell_surface || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("dock", "dock-shell-surface", shell_surface_on_dock);
                    last_dock_shell_surface = Some(shell_surface_on_dock);
                }
                let fullscreen_on_notch = fullscreen_on("main");
                crate::state::set_flag(crate::state::notch_fullscreen(), "main", fullscreen_on_notch);
                if Some(fullscreen_on_notch) != last_notch_fullscreen || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("main", "notch-fullscreen", fullscreen_on_notch);
                    last_notch_fullscreen = Some(fullscreen_on_notch);
                }

                if Some(effective_dock_overlap) != last_dock_overlap || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("dock", "dock-overlap", effective_dock_overlap);
                    last_dock_overlap = Some(effective_dock_overlap);
                    last_emit = Instant::now();
                }

                if Some(should_notch_overlap) != last_notch_overlap || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("main", "notch-overlap", should_notch_overlap);
                    last_notch_overlap = Some(should_notch_overlap);
                }

                // Docks and notches on other monitors get their own overlap state.
                for (label, win) in handle_visibility.webview_windows() {
                    let is_dock = is_dock_label(&label);
                    if !(is_dock || is_notch_label(&label)) || crate::state::monitor_suffix(&label).is_none() {
                        continue;
                    }
                    let Ok(w_hwnd) = win.hwnd() else { continue };
                    let Some((mx, my, _mw, _mh)) = monitor_rect_for_hwnd(w_hwnd) else { continue };
                    {
                        let fullscreen_here = monitor_fullscreen((mx, my), focused_monitor, current_is_fs, &fullscreen_monitors)
                            || (is_dock && hide_on_start && start_on(w_hwnd));
                        if is_dock {
                            crate::state::set_flag(crate::state::dock_fullscreen(), &label, fullscreen_here);
                        } else {
                            crate::state::set_flag(crate::state::notch_fullscreen(), &label, fullscreen_here);
                        }
                        let needs_update = last_secondary_dock_fullscreen.get(&label)
                            .is_none_or(|(previous, sent_at)| *previous != fullscreen_here || sent_at.elapsed() >= Duration::from_secs(3));
                        if needs_update {
                            let event = if is_dock { "dock-fullscreen" } else { "notch-fullscreen" };
                            let _ = handle_visibility.emit_to(label.as_str(), event, fullscreen_here);
                            last_secondary_dock_fullscreen.insert(label.clone(), (fullscreen_here, now));
                        }
                        if is_dock {
                            let shell_here = !hide_on_start && start_on(w_hwnd);
                            let needs_shell_update = last_secondary_dock_shell_surface.get(&label)
                                .is_none_or(|(previous, sent_at)| *previous != shell_here || sent_at.elapsed() >= Duration::from_secs(3));
                            if needs_shell_update {
                                let _ = handle_visibility.emit_to(label.as_str(), "dock-shell-surface", shell_here);
                                last_secondary_dock_shell_surface.insert(label.clone(), (shell_here, now));
                            }
                        }
                    }
                    let overlap = match fg_info {
                        Some((screen, rect, covers)) if screen.left == mx && screen.top == my => {
                            if covers {
                                true
                            } else if is_dock {
                                crate::state::dock_overlap_rect(&label).is_some_and(|dr| {
                                    rect_overlaps_dock(mx, screen.bottom, rect, dr, cached_scale)
                                })
                            } else {
                                crate::state::notch_rects().lock().ok().and_then(|m| m.get(&label).copied()).is_some_and(|nr| {
                                    rect_overlaps_notch(mx, screen.top, rect, nr, cached_scale)
                                })
                            }
                        }
                        // Another monitor has focus: a maximized or fullscreen window there still covers the panel.
                        _ => unfocused_panel_overlap(label.as_str(), is_dock).unwrap_or(false),
                    };
                    // Same visibility guard as the primary dock.
                    let overlap = overlap && (!is_dock || win.is_visible().unwrap_or(false));
                    let map = if is_dock { crate::state::dock_overlap() } else { crate::state::notch_overlap() };
                    let overlap = if is_dock {
                        overlap
                    } else {
                        notch_overlap_during_start(start_shown, overlap, crate::state::get_overlap(map, &label) == 1)
                    };
                    let new_val = if overlap { 1 } else { 0 };
                    if crate::state::get_overlap(map, &label) != new_val {
                        crate::state::set_overlap(map, &label, new_val);
                        let event = if is_dock { "dock-overlap" } else { "notch-overlap" };
                        let _ = handle_visibility.emit_to(label.as_str(), event, overlap);
                    }
                }

                // Maximized signal for the adaptive dock. Not tied to dock visibility so init_dock can send the current value.
                CURRENT_FOREGROUND_MAXIMIZED.store(should_maximized, Ordering::Relaxed);
                if Some(should_maximized) != last_dock_maximized || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("dock", "dock-maximized", should_maximized);
                    last_dock_maximized = Some(should_maximized);
                }

                // Update full-screen visibility (hides primary TopBar/Corners only when primary monitor is fullscreen)
                if fullscreen_on_notch && last_visible {
                    let _ = tx_clone.send(SystemCommand::ToggleVisibility(false));
                    last_visible = false;
                } else if !fullscreen_on_notch && !last_visible {
                    let _ = tx_clone.send(SystemCommand::ToggleVisibility(true));
                    last_visible = true;
                }

                // Enforce native taskbar hiding (periodic check)
                if NATIVE_TASKBAR_HIDDEN.load(Ordering::Relaxed) {
                    use windows::Win32::UI::WindowsAndMessaging::{FindWindowA, IsWindowVisible};
                    let tray_class = windows::core::PCSTR(c"Shell_TrayWnd".as_ptr() as *const u8);

                    let mut should_rehide = false;
                    if let Ok(tray_hwnd) = FindWindowA(tray_class, windows::core::PCSTR::null()) {
                        if IsWindowVisible(tray_hwnd).as_bool() { should_rehide = true; }
                    }
                    if !should_rehide {
                        should_rehide = crate::utils::secondary_taskbars().iter().any(|&hwnd| IsWindowVisible(hwnd).as_bool());
                    }

                    if should_rehide {
                        set_taskbar_visibility(false, false);
                    }
                }
            }

            // Capture UI state is event-driven: WinEvent callbacks set the
            // recheck flag on top-level window show/hide/destroy and foreground
            // changes, so the scan below only runs when something changed
            // (throttled to coalesce bursts of window events).
            let scan_now = now_ms();
            if CAPTURE_RECHECK.load(Ordering::Relaxed)
                && scan_now - CAPTURE_LAST_SCAN_MS.load(Ordering::Relaxed) >= 400
            {
                CAPTURE_RECHECK.store(false, Ordering::Relaxed);
                CAPTURE_LAST_SCAN_MS.store(scan_now, Ordering::Relaxed);
                let active = is_capture_ui_present();
                if active != CAPTURE_UI_ACTIVE.load(Ordering::Relaxed) {
                    CAPTURE_UI_ACTIVE.store(active, Ordering::Relaxed);
                    apply_capture_ui_state(&handle_visibility, active);
                }
            }
            std::thread::park_timeout(crate::memory_saver::poll_interval(std::time::Duration::from_millis(150)));
        }
    });

    tx
}



static DDC_FOUND: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static DDC_APPLIED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Returns how many external monitors were found and how many took the new value.
fn set_physical_monitors_brightness(brightness: u32) -> (u32, u32) {
    DDC_FOUND.store(0, Ordering::Relaxed);
    DDC_APPLIED.store(0, Ordering::Relaxed);
    unsafe {
        use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
        use windows::Win32::Foundation::{RECT, LPARAM};
        use windows::core::BOOL;

        unsafe extern "system" fn monitor_enum_proc(
            hmonitor: HMONITOR,
            _: HDC,
            _: *mut RECT,
            lparam: LPARAM,
        ) -> BOOL {
            let brightness = lparam.0 as u32;
            #[repr(C)]
            #[derive(Clone, Copy)]
            struct PHYSICAL_MONITOR {
                h_physical_monitor: usize,
                sz_physical_monitor_description: [u16; 128],
            }
            #[link(name = "dxva2")]
            extern "system" {
                fn GetNumberOfPhysicalMonitorsFromHMONITOR(
                    hMonitor: HMONITOR,
                    pdwNumberOfPhysicalMonitors: *mut u32,
                ) -> BOOL;
                fn GetPhysicalMonitorsFromHMONITOR(
                    hMonitor: HMONITOR,
                    dwPhysicalMonitorArraySize: u32,
                    pPhysicalMonitorArray: *mut PHYSICAL_MONITOR,
                ) -> BOOL;
                fn DestroyPhysicalMonitors(
                    dwPhysicalMonitorArraySize: u32,
                    pPhysicalMonitorArray: *mut PHYSICAL_MONITOR,
                ) -> BOOL;
                fn SetMonitorBrightness(
                    hMonitor: usize,
                    dwNewBrightness: u32,
                ) -> BOOL;
                fn GetMonitorBrightness(
                    hMonitor: usize,
                    pdwMinimumBrightness: *mut u32,
                    pdwCurrentBrightness: *mut u32,
                    pdwMaximumBrightness: *mut u32,
                ) -> BOOL;
                fn GetVCPFeatureAndVCPFeatureReply(
                    hMonitor: usize,
                    bVCPCode: u8,
                    pvct: *mut u32,
                    pdwCurrentValue: *mut u32,
                    pdwMaximumValue: *mut u32,
                ) -> BOOL;
                fn SetVCPFeature(
                    hMonitor: usize,
                    bVCPCode: u8,
                    dwNewValue: u32,
                ) -> BOOL;
            }

            let mut count = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count).as_bool() && count > 0 {
                let mut monitors = vec![std::mem::zeroed::<PHYSICAL_MONITOR>(); count as usize];
                if GetPhysicalMonitorsFromHMONITOR(hmonitor, count, monitors.as_mut_ptr()).as_bool() {
                    for mon in &monitors {
                        // 0 is a valid handle here, the first monitor gets it.
                        let handle = mon.h_physical_monitor;
                        DDC_FOUND.fetch_add(1, Ordering::Relaxed);
                        // VCP 0x10 is brightness. Unlike the high-level call it needs no capabilities string, which many DisplayPort monitors lack.
                        let (mut kind, mut cur, mut vcp_max) = (0u32, 0u32, 0u32);
                        if !GetVCPFeatureAndVCPFeatureReply(handle, 0x10, &mut kind, &mut cur, &mut vcp_max).as_bool() || vcp_max == 0 {
                            vcp_max = 100;
                        }
                        let target = vcp_max * brightness / 100;
                        let mut done = false;
                        for attempt in 0..3 {
                            if SetVCPFeature(handle, 0x10, target).as_bool() {
                                done = true;
                                break;
                            }
                            if attempt < 2 {
                                std::thread::sleep(std::time::Duration::from_millis(60));
                            }
                        }
                        if done {
                            DDC_APPLIED.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let (mut min, mut now, mut max) = (0u32, 0u32, 0u32);
                        if GetMonitorBrightness(handle, &mut min, &mut now, &mut max).as_bool() && max > min
                            && SetMonitorBrightness(handle, min + (max - min) * brightness / 100).as_bool()
                        {
                            DDC_APPLIED.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let code = windows::Win32::Foundation::GetLastError().0;
                        let name = String::from_utf16_lossy(&mon.sz_physical_monitor_description).trim_end_matches('\0').to_string();
                        crate::diagnostics::log(&format!("brightness: monitor \"{name}\" did not accept a brightness change over DDC/CI, error {code}"));
                    }
                    let _ = DestroyPhysicalMonitors(count, monitors.as_mut_ptr());
                }
            }
            true.into()
        }

        let _ = EnumDisplayMonitors(
            None, None,
            Some(monitor_enum_proc),
            LPARAM(brightness as isize),
        );
    }
    (DDC_FOUND.load(Ordering::Relaxed), DDC_APPLIED.load(Ordering::Relaxed))
}

/// Keeps external monitors working when the laptop WMI setup fails.
fn external_brightness_only(rx: std::sync::mpsc::Receiver<u32>) {
    crate::diagnostics::log("brightness: WMI unavailable, controlling external monitors only");
    while let Ok(mut brightness) = rx.recv() {
        while let Ok(newer) = rx.try_recv() {
            brightness = newer;
        }
        let (found, applied) = set_physical_monitors_brightness(brightness.min(100));
        if found > 0 && applied == 0 {
            crate::diagnostics::log(&format!("brightness: {found} external monitor(s) found, none accepted {brightness}"));
        }
    }
}

pub fn setup_brightness_worker() {
    let (tx, rx) = channel::<u32>();
    let _ = BRIGHTNESS_SENDER.set(tx);
    std::thread::spawn(move || unsafe {
        // Direct WMI COM + DXVA2 implementation (zero child processes spawned).
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED, CoCreateInstance, CoSetProxyBlanket, CLSCTX_ALL,
            EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
        };
        use windows::Win32::System::Wmi::{IWbemLocator, IWbemClassObject, WbemLocator, WBEM_GENERIC_FLAG_TYPE};
        use windows::Win32::System::Variant::{VARIANT, VariantClear, VARENUM};

        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let locator: IWbemLocator = match CoCreateInstance(&WbemLocator, None, CLSCTX_ALL) {
            Ok(l) => l,
            Err(_) => {
                let _ = CoUninitialize();
                external_brightness_only(rx);
                return;
            }
        };
        let ns = windows::core::BSTR::from("root\\WMI");
        let empty_bstr = windows::core::BSTR::new();
        let services = match locator.ConnectServer(&ns, &empty_bstr, &empty_bstr, &empty_bstr, 0, &empty_bstr, None) {
            Ok(s) => s,
            Err(_) => {
                let _ = CoUninitialize();
                external_brightness_only(rx);
                return;
            }
        };

        // Without impersonation on the proxy WMI answers ACCESS_DENIED and every write was silently dropped.
        // 10 = RPC_C_AUTHN_WINNT, 0 = RPC_C_AUTHZ_NONE.
        let _ = CoSetProxyBlanket(
            &services, 10, 0, windows::core::PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE,
        );

        // GetMethod only works on the class definition, not an instance. Resolve the input signature once.
        let set_brightness_params = {
            let mut class_def: Option<IWbemClassObject> = None;
            let mut in_cls: Option<IWbemClassObject> = None;
            if services
                .GetObject(&windows::core::BSTR::from("WmiMonitorBrightnessMethods"), WBEM_GENERIC_FLAG_TYPE(0), None, Some(&mut class_def), None)
                .is_ok()
            {
                if let Some(ref class_def) = class_def {
                    let _ = class_def.GetMethod(windows::core::w!("WmiSetBrightness"), 0i32, &mut in_cls, std::ptr::null_mut());
                }
            }
            in_cls
        };

        while let Ok(mut brightness) = rx.recv() {
            // A drag queues a value per event and each write is slow, only the latest matters.
            while let Ok(newer) = rx.try_recv() {
                brightness = newer;
            }
            let brightness = brightness.min(100);
            // 1. Laptop internal panel via WMI WmiMonitorBrightnessMethods
            let wql = windows::core::BSTR::from("WQL");
            let q = windows::core::BSTR::from("SELECT * FROM WmiMonitorBrightnessMethods");
            if let Ok(enum_obj) = services.ExecQuery(&wql, &q, WBEM_GENERIC_FLAG_TYPE(0), None) {
                let mut row = [None::<IWbemClassObject>; 1];
                let mut returned = 0u32;
                while enum_obj.Next(-1i32, &mut row, &mut returned).is_ok() && returned > 0 {
                    if let Some(obj) = row[0].take() {
                        let mut var = VARIANT::default();
                        if obj.Get(windows::core::w!("__RELPATH"), 0i32, &mut var, None, None).is_ok() {
                            let relpath_str = var.Anonymous.Anonymous.Anonymous.bstrVal.to_string();
                            let _ = VariantClear(&mut var);
                            if !relpath_str.is_empty() {
                                let obj_path = windows::core::BSTR::from(relpath_str.as_str());
                                let method_name = windows::core::BSTR::from("WmiSetBrightness");

                                {
                                    if let Some(ref in_cls) = set_brightness_params {
                                        if let Ok(in_params) = in_cls.SpawnInstance(0i32) {
                                            let mut b_var = VARIANT::default();
                                            let b_anon = &mut b_var.Anonymous.Anonymous;
                                            b_anon.vt = VARENUM(17); // VT_UI1
                                            b_anon.Anonymous.bVal = brightness as u8;
                                            let _ = in_params.Put(windows::core::w!("Brightness"), 0i32, &b_var, 0);

                                            let mut t_var = VARIANT::default();
                                            let t_anon = &mut t_var.Anonymous.Anonymous;
                                            t_anon.vt = VARENUM(3); // VT_I4
                                            t_anon.Anonymous.lVal = 0i32;
                                            let _ = in_params.Put(windows::core::w!("Timeout"), 0i32, &t_var, 0);

                                            let _ = services.ExecMethod(
                                                &obj_path, &method_name, WBEM_GENERIC_FLAG_TYPE(0), None,
                                                Some(&in_params), None, None,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 2. Desktop external monitor via Physical Monitor API (DXVA2 DDC/CI)
            let (found, applied) = set_physical_monitors_brightness(brightness);
            if found > 0 && applied == 0 {
                crate::diagnostics::log(&format!("brightness: {found} external monitor(s) found, none accepted {brightness}"));
            }


        }
        let _ = CoUninitialize();
    });
}


static MH_LAST_MAIN_IGNORE: AtomicI32 = AtomicI32::new(-1);
static MH_LAST_DOCK_IGNORE: AtomicI32 = AtomicI32::new(-1);
static MH_LAST_OV_IGNORE: AtomicI32 = AtomicI32::new(-1);
static MH_LAST_EDGE_HOVER: AtomicI32 = AtomicI32::new(-1);
static MH_LAST_TOP_EDGE_HOVER: AtomicI32 = AtomicI32::new(-1);
static MH_LAST_LEFT_EDGE_HOVER: AtomicI32 = AtomicI32::new(-1);
static MH_LAST_RIGHT_EDGE_HOVER: AtomicI32 = AtomicI32::new(-1);
static MH_DOCK_EXPIRY_MS: AtomicI64 = AtomicI64::new(0);
static MH_TOPBAR_EXPIRY_MS: AtomicI64 = AtomicI64::new(0);
static MH_LEFT_EXPIRY_MS: AtomicI64 = AtomicI64::new(0);
static MH_LEFT_ENTER_MS: AtomicI64 = AtomicI64::new(0);
static MH_RIGHT_ENTER_MS: AtomicI64 = AtomicI64::new(0);

fn edge_dwell_met(entered: &AtomicI64, inside: bool, now: i64, dwell_ms: i64) -> bool {
    if !inside {
        entered.store(0, Ordering::Relaxed);
        return false;
    }
    let since = entered.load(Ordering::Relaxed);
    if since == 0 {
        entered.store(now, Ordering::Relaxed);
    }
    dwell_ms <= 0 || (since != 0 && now - since >= dwell_ms)
}

fn edge_dwell_waiting(inside: bool, confirmed: bool) -> bool {
    inside && !confirmed
}
static MH_RIGHT_EXPIRY_MS: AtomicI64 = AtomicI64::new(0);
static MH_LAST_MONITOR_UPDATE_MS: AtomicI64 = AtomicI64::new(0);
static MH_CACHED_MON_POS: Mutex<Option<(i32, i32)>> = Mutex::new(None);
static MH_CACHED_MON_SIZE: Mutex<Option<(u32, u32)>> = Mutex::new(None);
static MH_CACHED_DOCK_MON_POS: Mutex<Option<(i32, i32)>> = Mutex::new(None);
static MH_CACHED_DOCK_MON_SIZE: Mutex<Option<(u32, u32)>> = Mutex::new(None);
static MH_CACHED_NOTCH_MON_POS: Mutex<Option<(i32, i32)>> = Mutex::new(None);
static MH_CACHED_NOTCH_MON_SIZE: Mutex<Option<(u32, u32)>> = Mutex::new(None);
static CAPTURE_UI_ACTIVE: AtomicBool = AtomicBool::new(false);
static CAPTURE_RECHECK: AtomicBool = AtomicBool::new(true);
static CAPTURE_LAST_SCAN_MS: AtomicI64 = AtomicI64::new(0);

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

/// Pure predicate that distinguishes an active screen-capture overlay from a
/// normal captioned/maximized Snipping Tool editor window.
fn is_capture_overlay_window(exe_name: &str, style: u32, ex_style: u32, covers_monitor: bool) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{WS_CAPTION, WS_EX_TOPMOST};
    if !covers_monitor {
        return false;
    }
    if exe_name.eq_ignore_ascii_case("screenclippinghost.exe") {
        return true;
    }
    if exe_name.eq_ignore_ascii_case("snippingtool.exe")
        || exe_name.eq_ignore_ascii_case("screensketch.exe")
    {
        return (style & WS_CAPTION.0) == 0 || (ex_style & WS_EX_TOPMOST.0) != 0;
    }
    false
}

/// True while a screen-capture UI (Windows Snipping Tool) has a visible window.
/// the way even if the capture window isn't recognised as fullscreen.
unsafe extern "system" fn capture_ui_enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    use windows::Win32::Graphics::Dwm::DWMWA_CLOAKED;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongW, GetWindowRect, IsIconic, GWL_EXSTYLE, GWL_STYLE, WS_CAPTION, WS_EX_TOPMOST,
    };
    let found = &mut *(lparam.0 as *mut bool);
    if *found { return BOOL(0); }
    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() { return BOOL(1); }

    let mut cloaked: u32 = 0;
    if DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut _ as *mut _,
        std::mem::size_of::<u32>() as u32,
    )
    .is_ok()
        && cloaked != 0
    {
        return BOOL(1);
    }

    let mut rect = windows::Win32::Foundation::RECT::default();
    let covers_monitor = GetWindowRect(hwnd, &mut rect).is_ok()
        && monitor_rect_for_hwnd(hwnd).is_some_and(|(_, _, mw, mh)| {
            (rect.right - rect.left) * 10 >= mw * 9 && (rect.bottom - rect.top) * 10 >= mh * 9
        });
    if !covers_monitor { return BOOL(1); }

    let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid != 0 && pid != std::process::id() {
        if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            if QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok() {
                let path = String::from_utf16_lossy(&buf[..len as usize]).to_lowercase();
                let name = path.rsplit('\\').next().unwrap_or("");
                if is_capture_overlay_window(name, style, ex_style, covers_monitor) {
                    *found = true;
                }
            }
            let _ = CloseHandle(handle);
        }
    }

    if !*found && ((style & WS_CAPTION.0) == 0 || (ex_style & WS_EX_TOPMOST.0) != 0) {
        use windows::Win32::UI::WindowsAndMessaging::GetClassNameA;
        let mut class_buf = [0u8; 256];
        let len = GetClassNameA(hwnd, &mut class_buf);
        let class = std::str::from_utf8(&class_buf[..len as usize]).unwrap_or("").to_lowercase();
        if class.contains("snipping") { *found = true; }
    }

    BOOL(1)
}

fn is_capture_ui_present() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    let mut found = false;
    unsafe {
        let _ = EnumWindows(Some(capture_ui_enum_proc), LPARAM(&mut found as *mut bool as isize));
    }
    found
}

fn apply_capture_ui_state(app: &AppHandle, active: bool) {
    if SHUTTING_DOWN.load(Ordering::Relaxed) {
        return;
    }
    let should_show_notch = crate::state::SPLASH_FINISHED.load(Ordering::Relaxed)
        && crate::utils::get_setting_str(app, "fern-notch-enabled").as_deref() != Some("false");
    for (label, win) in app.webview_windows() {
        if crate::state::is_notch_label(&label) {
            if active {
                crate::utils::set_window_click_through(&win, true);
                if let Ok(hwnd) = win.hwnd() {
                    unsafe {
                        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
                        let _ = ShowWindow(hwnd, SW_HIDE);
                    }
                }
            } else {
                position_notch_window(&win);
                if should_show_notch {
                    if let Ok(hwnd) = win.hwnd() {
                        unsafe {
                            use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
                            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                        }
                        re_assert_topmost(hwnd);
                    }
                }
            }
        } else if crate::state::is_dock_label(&label) && active {
            crate::utils::set_window_click_through(&win, true);
        }
    }
    if active {
        if let Some(ov_win) = app.get_webview_window("overlay") {
            crate::utils::set_window_click_through(&ov_win, true);
        }
    }
    // Force the mouse hook to re-evaluate hit-testing on the next move.
    MH_LAST_MAIN_IGNORE.store(-1, Ordering::Relaxed);
    MH_LAST_DOCK_IGNORE.store(-1, Ordering::Relaxed);
    MH_LAST_OV_IGNORE.store(-1, Ordering::Relaxed);
    if let Ok(mut m) = crate::state::dock_extra_last_ignore().lock() { for v in m.values_mut() { *v = -1; } }
    if let Ok(mut m) = crate::state::notch_extra_last_ignore().lock() { for v in m.values_mut() { *v = -1; } }
}

static MOUSE_X: AtomicI32 = AtomicI32::new(0);
static MOUSE_Y: AtomicI32 = AtomicI32::new(0);
static MOUSE_DIRTY: AtomicBool = AtomicBool::new(false);
static MOUSE_WORKER: OnceLock<std::thread::Thread> = OnceLock::new();

/// Hover intent for the top edge: touching the band starts a dwell clock, and the edge only arms
/// (peek, claim input) after resting there for fern-notch-edge-delay ms (0 = instant).
/// Stops a pass-through to a maximized window's title bar from triggering it.
const TOP_EDGE_DWELL_MS: i64 = 200;
const TOP_EDGE_POLL_MS: u64 = 40;
static CURSOR_RECHECK_PENDING: AtomicBool = AtomicBool::new(false);

/// The webview only learns the cursor left through mouse events, and a click-through window gets none.
/// So when the hit test makes the dock click-through with the hover still on (the cursor was over a
/// preview that closed, say), only the backend can end it. Left on, it keeps a Smart dock from hiding.
fn dock_hover_went_stale(should_ignore: bool, hovered: bool) -> bool {
    should_ignore && hovered
}

/// A hover running out needs another look when it expires, and a still cursor won't trigger one.
fn hover_waits_on_expiry(is_hovered: bool, final_hover: bool) -> bool {
    final_hover && !is_hovered
}

/// Returns the sibling panel labels to dismiss immediately when `active_label` triggers an active
/// edge hover on its monitor. On single-monitor setups (or when `active_edge` is false), returns empty.
fn cross_monitor_handoff_targets<'a>(
    active_label: &str,
    active_edge: bool,
    all_labels: &[&'a str],
) -> Vec<&'a str> {
    if !active_edge {
        return Vec::new();
    }
    all_labels
        .iter()
        .copied()
        .filter(|&label| label != active_label)
        .collect()
}

fn handoff_other_dock_windows(app_handle: &AppHandle, active_label: &str) {
    let labels: Vec<String> = app_handle
        .webview_windows()
        .into_keys()
        .filter(|l| is_dock_label(l))
        .collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    for other in cross_monitor_handoff_targets(active_label, true, &label_refs) {
        if other == "dock" {
            MH_DOCK_EXPIRY_MS.store(0, Ordering::Relaxed);
            if crate::state::get_flag(crate::state::dock_hovered(), "dock") {
                crate::state::set_flag(crate::state::dock_hovered(), "dock", false);
                let _ = app_handle.emit_to("dock", "dock-cursor-left", ());
            }
            if MH_LAST_EDGE_HOVER
                .compare_exchange(1, 0, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                let _ = app_handle.emit_to("dock", "dock-edge-hover", false);
            }
        } else {
            crate::state::set_i64(crate::state::dock_extra_expiry_ms(), other, 0);
            if crate::state::get_flag(crate::state::dock_hovered(), other) {
                crate::state::set_flag(crate::state::dock_hovered(), other, false);
                let _ = app_handle.emit_to(other, "dock-cursor-left", ());
            }
            if crate::state::get_i32(crate::state::dock_extra_last_edge_hover(), other, 0) == 1 {
                crate::state::set_i32(crate::state::dock_extra_last_edge_hover(), other, 0);
                let _ = app_handle.emit_to(other, "dock-edge-hover", false);
            }
        }
    }
}

fn handoff_other_notch_windows(app_handle: &AppHandle, active_label: &str) {
    let labels: Vec<String> = app_handle
        .webview_windows()
        .into_keys()
        .filter(|l| is_notch_label(l))
        .collect();
    let label_refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    for other in cross_monitor_handoff_targets(active_label, true, &label_refs) {
        if other == "main" {
            MH_TOPBAR_EXPIRY_MS.store(0, Ordering::Relaxed);
            if crate::state::get_flag(crate::state::notch_hovered(), "main") {
                crate::state::set_flag(crate::state::notch_hovered(), "main", false);
                let _ = app_handle.emit_to("main", "notch-cursor-left", ());
            }
            if MH_LAST_TOP_EDGE_HOVER
                .compare_exchange(1, 0, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                let _ = app_handle.emit_to("main", "notch-edge-hover", false);
            }
        } else {
            crate::state::set_i64(crate::state::notch_extra_expiry_ms(), other, 0);
            if crate::state::get_flag(crate::state::notch_hovered(), other) {
                crate::state::set_flag(crate::state::notch_hovered(), other, false);
                let _ = app_handle.emit_to(other, "notch-cursor-left", ());
            }
            if crate::state::get_i32(crate::state::notch_extra_last_edge_hover(), other, 0) == 1 {
                crate::state::set_i32(crate::state::notch_extra_last_edge_hover(), other, 0);
                let _ = app_handle.emit_to(other, "notch-edge-hover", false);
            }
        }
    }
}

/// Re-runs the hit test for a cursor that hasn't moved, when what's under it changed.
pub fn request_cursor_recheck() {
    if SHUTTING_DOWN.load(Ordering::Relaxed) {
        return;
    }
    MOUSE_DIRTY.store(true, Ordering::Relaxed);
    if let Some(worker) = MOUSE_WORKER.get() {
        worker.unpark();
    }
}

fn top_edge_armed(app_handle: &AppHandle, label: &str, in_band: bool, now: i64) -> bool {
    let enter = crate::state::notch_edge_enter_ms();
    let armed = crate::state::notch_edge_armed();
    // A fixed notch is always visible, so no gate.
    let fixed = crate::monitors::resolve_mode_for_label(app_handle, crate::types::WindowKind::Notch, label) == "fixed";
    if !in_band || fixed {
        crate::state::set_i64(enter, label, 0);
        crate::state::set_flag(armed, label, false);
        return false;
    }
    if crate::state::get_flag(armed, label) {
        return true;
    }
    let dwell_ms = crate::utils::get_setting_str(app_handle, "fern-notch-edge-delay")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(TOP_EDGE_DWELL_MS)
        .clamp(0, 2000);
    if dwell_ms <= 0 {
        crate::state::set_flag(armed, label, true);
        return true;
    }
    let entered = crate::state::get_i64(enter, label, 0);
    if entered == 0 {
        crate::state::set_i64(enter, label, now);
        CURSOR_RECHECK_PENDING.store(true, Ordering::Relaxed);
        return false;
    }
    if now - entered >= dwell_ms {
        crate::state::set_flag(armed, label, true);
        return true;
    }
    CURSOR_RECHECK_PENDING.store(true, Ordering::Relaxed);
    false
}

fn edge_area_narrow(app_handle: &AppHandle, key: &str) -> bool {
    crate::utils::get_setting_str(app_handle, key).as_deref() == Some("narrow")
}

fn edge_area_allows_x(narrow: bool, span: Option<(i32, i32)>, cursor_x: i32, pad: i32) -> bool {
    !narrow || span.map_or(true, |(l, r)| cursor_x >= l - pad && cursor_x <= r + pad)
}

fn overlay_edge_y_span(mon_y: i32, mon_h: i32, scale: f64) -> (i32, i32) {
    let nh = (196.0 * scale) as i32;
    let ny = mon_y + (mon_h / 2) - (nh / 2);
    (ny, ny + nh)
}

fn notch_x_span_for(label: &str, scale: f64) -> Option<(i32, i32)> {
    let (win_pos, _) = crate::state::main_window_rects().lock().ok().and_then(|m| m.get(label).copied())?;
    let region = crate::state::notch_rects().try_lock().ok()?;
    let r = region.get(label).copied()?;
    let left = win_pos.x + (r.x as f64 * scale) as i32;
    Some((left, left + (r.width as f64 * scale) as i32))
}

fn start_mouse_worker(app_handle: AppHandle) {
    let worker = std::thread::Builder::new()
        .name("mouse-worker".into())
        .spawn(move || loop {
            // A pending dwell has to finish with a still cursor, but the hook only fires on movement,
            // so also wake on a timer.
            if CURSOR_RECHECK_PENDING.load(Ordering::Relaxed) {
                std::thread::park_timeout(Duration::from_millis(TOP_EDGE_POLL_MS));
                MOUSE_DIRTY.store(true, Ordering::Relaxed);
            } else {
                std::thread::park();
            }
            if !MOUSE_DIRTY.swap(false, Ordering::Relaxed) {
                continue;
            }
            let cursor = windows::Win32::Foundation::POINT {
                x: MOUSE_X.load(Ordering::Relaxed),
                y: MOUSE_Y.load(Ordering::Relaxed),
            };
            unsafe { process_mouse_move(&app_handle, cursor) };
            std::thread::sleep(Duration::from_millis(32));
        });
    if let Ok(worker) = worker {
        let _ = MOUSE_WORKER.set(worker.thread().clone());
    }
}

fn start_mouse_hook_thread() {
    let _ = std::thread::Builder::new().name("mouse-hook".into()).spawn(|| unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, GetMessageW, TranslateMessage, MSG};
        if let Err(e) = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), None, 0) {
            crate::diagnostics::log(&format!("mouse hook install failed: {e}"));
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

/// Mixer bounds (notch + panel) in physical pixels, converted from the overlay's CSS px.
fn volume_mixer_physical_rect_for(app: &AppHandle) -> Option<(i32, i32, i32, i32)> {
    let (x, y, width, height) = (*crate::state::VOLUME_MIXER_RECT.lock().ok()?)?;
    let ov_win = app.get_webview_window("overlay")?;
    let monitor = ov_win.primary_monitor().ok().flatten()?;
    let scale = monitor.scale_factor();
    let pos = monitor.position();
    Some((
        pos.x + (x * scale) as i32,
        pos.y + (y * scale) as i32,
        (width * scale) as i32,
        (height * scale) as i32,
    ))
}

/// The hook only fires on movement, so a cursor that stops just outside the mixer would leave it
/// open. This closes the card once the cursor is off it.
fn setup_volume_mixer_watchdog(app_handle: AppHandle) {
    let mut hook_missed_at: Option<Instant> = None;
    std::thread::spawn(move || loop {
        std::thread::sleep(cursor_watch_interval(hook_missed_at.map(|at| at.elapsed())));
        if SHUTTING_DOWN.load(Ordering::Relaxed) {
            continue;
        }
        let mut pt = windows::Win32::Foundation::POINT::default();
        if unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut pt) }.is_err() {
            continue;
        }
        // The hook goes silent over elevated windows and while one has the foreground (Task Manager).
        // The cursor being somewhere the hook didn't report proves that, and from then on this loop
        // does the hook's job, or the dock would stay click-through with its clicks going underneath.
        if pt.x != MOUSE_X.load(Ordering::Relaxed) || pt.y != MOUSE_Y.load(Ordering::Relaxed) {
            hook_missed_at = Some(Instant::now());
            crate::memory_saver::note_input();
            MOUSE_X.store(pt.x, Ordering::Relaxed);
            MOUSE_Y.store(pt.y, Ordering::Relaxed);
            MOUSE_DIRTY.store(true, Ordering::Relaxed);
            if let Some(worker) = MOUSE_WORKER.get() {
                worker.unpark();
            }
        }
        let Some((rx, ry, rw, rh)) = volume_mixer_physical_rect_for(&app_handle) else {
            continue;
        };
        let inside = pt.x >= rx && pt.x <= rx + rw && pt.y >= ry && pt.y <= ry + rh;
        if inside {
            continue;
        }
        let at_left_edge = app_handle
            .primary_monitor()
            .ok()
            .flatten()
            .is_some_and(|m| {
                let pos = m.position();
                let size = m.size();
                let on_right = crate::utils::get_setting_str(&app_handle, "fern-volume-side").as_deref() == Some("right");
                let in_band = if on_right { pt.x >= pos.x + size.width as i32 - 8 && pt.x <= pos.x + size.width as i32 } else { pt.x >= pos.x && pt.x <= pos.x + 8 };
                in_band && pt.y >= pos.y && pt.y <= pos.y + size.height as i32
            });
        if !at_left_edge && MH_LAST_LEFT_EDGE_HOVER.swap(0, Ordering::Relaxed) != 0 {
            MH_LEFT_EXPIRY_MS.store(0, Ordering::Relaxed);
            let _ = app_handle.emit("volume-edge-hover", false);
        }
    });
}

/// Start takes the foreground a moment before it appears, and judging the windows under it in that
/// moment would flash the dock. But its window can also stay foreground after closing, off screen,
/// so this only holds for a moment or the dock could stay hidden with Start gone.
fn start_counts_as_shown(start_open: bool, on_screen: bool, since_focused: Duration) -> bool {
    const APPEARING: Duration = Duration::from_millis(1000);
    start_open && (on_screen || since_focused < APPEARING)
}

/// Start doesn't reach the top of the screen, so it has no say over the notch. Judged like any other
/// window it would count as "nothing overlaps", and a Smart notch hidden by a maximized window would
/// come down for as long as Start is open.
fn notch_overlap_during_start(start_shown: bool, computed: bool, last: bool) -> bool {
    if start_shown { last } else { computed }
}

/// Slow while the hook reports the cursor, at the hook's own pace for a while after it missed a move.
fn cursor_watch_interval(since_hook_missed: Option<Duration>) -> Duration {
    const TAKEOVER: Duration = Duration::from_millis(1000);
    match since_hook_missed {
        Some(elapsed) if elapsed < TAKEOVER => Duration::from_millis(32),
        _ => crate::memory_saver::poll_interval(Duration::from_millis(200)),
    }
}

pub fn setup_mouse_hook(app_handle: AppHandle) {
    setup_volume_mixer_watchdog(app_handle.clone());
    start_mouse_worker(app_handle);
    start_mouse_hook_thread();
}

unsafe extern "system" fn mouse_hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> windows::Win32::Foundation::LRESULT {
    if code >= 0 && wparam.0 == WM_MOUSEMOVE as usize && !SHUTTING_DOWN.load(Ordering::Relaxed) {
        crate::memory_saver::note_input();
        crate::diagnostics::HOOK_EVENTS.fetch_add(1, Ordering::Relaxed);
        crate::diagnostics::HOOK_LAST_EVENT_MS.store(now_ms(), Ordering::Relaxed);
        let pt = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        MOUSE_X.store(pt.pt.x, Ordering::Relaxed);
        MOUSE_Y.store(pt.pt.y, Ordering::Relaxed);
        MOUSE_DIRTY.store(true, Ordering::Relaxed);
        if let Some(worker) = MOUSE_WORKER.get() {
            worker.unpark();
        }
    }
    if code >= 0 && wparam.0 == WM_LBUTTONDOWN as usize && !SHUTTING_DOWN.load(Ordering::Relaxed) {
        crate::state::START_OPEN_AT_PRESS.store(crate::commands::is_start_menu_open(), Ordering::Relaxed);
        let pt = (*(lparam.0 as *const MSLLHOOKSTRUCT)).pt;
        if in_show_desktop_corner(pt.x, pt.y) {
            std::thread::spawn(|| crate::commands::send_win_chord(windows::Win32::UI::Input::KeyboardAndMouse::VK_D));
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

fn in_show_desktop_corner(x: i32, y: i32) -> bool {
    const CORNER: i32 = 6;
    if CURRENT_FOREGROUND_FULLSCREEN.load(Ordering::Relaxed) {
        return false;
    }
    let setting = |key: &str| {
        crate::state::SETTINGS_CACHE
            .get()
            .and_then(|cache| cache.lock().ok())
            .and_then(|guard| guard.get(key).and_then(|v| v.as_str().map(|s| s.to_string())))
    };
    if setting("fern-dock-show-desktop-corner").as_deref() != Some("true") || setting("fern-dock-enabled").as_deref() == Some("false") {
        return false;
    }
    let (Some((mx, my)), Some((mw, mh))) = (
        MH_CACHED_DOCK_MON_POS.lock().ok().and_then(|g| *g),
        MH_CACHED_DOCK_MON_SIZE.lock().ok().and_then(|g| *g),
    ) else {
        return false;
    };
    let (right, bottom) = (mx + mw as i32, my + mh as i32);
    x >= right - CORNER && x < right && y >= bottom - CORNER && y < bottom
}

fn should_retry_artwork(attempts: u32) -> bool {
    attempts < 8 || (attempts - 8) % 15 == 0
}

fn should_suppress_notch_for_mode(mode: &str) -> bool {
    mode != "fixed"
}

fn should_ignore_notch_cursor(suppressed: bool, interactive: bool) -> bool {
    suppressed || !interactive
}

/// Smart and Peek notches stay hidden while a fullscreen window covers their monitor,
/// or whenever the notch is permanently turned off via `fern-notch-enabled`.
fn notch_suppressed(app_handle: &AppHandle, label: &str) -> bool {
    if crate::utils::get_setting_str(app_handle, "fern-notch-enabled").as_deref() == Some("false") {
        return true;
    }
    crate::state::get_flag(crate::state::notch_fullscreen(), label)
        && should_suppress_notch_for_mode(&crate::monitors::resolve_mode_for_label(
            app_handle,
            crate::types::WindowKind::Notch,
            label,
        ))
}

fn dock_edge_hover_allowed(fullscreen_on_dock: bool, at_bottom_edge: bool, dock_hovered: bool) -> bool {
    !fullscreen_on_dock && (at_bottom_edge || dock_hovered)
}

unsafe fn process_mouse_move(app_handle: &AppHandle, cursor: windows::Win32::Foundation::POINT) {
    let now = now_ms();
    CURSOR_RECHECK_PENDING.store(false, Ordering::Relaxed);
    {
        {

            // click-through and skipped entirely so the tool owns the screen.
            if CAPTURE_UI_ACTIVE.load(Ordering::Relaxed) {
                if MH_LAST_MAIN_IGNORE.load(Ordering::Relaxed) != 1 {
                    if let Some(w) = app_handle.get_webview_window("main") { crate::utils::set_window_click_through(&w, true); }
                    MH_LAST_MAIN_IGNORE.store(1, Ordering::Relaxed);
                }
                if MH_LAST_DOCK_IGNORE.load(Ordering::Relaxed) != 1 {
                    if let Some(w) = app_handle.get_webview_window("dock") { crate::utils::set_window_click_through(&w, true); }
                    MH_LAST_DOCK_IGNORE.store(1, Ordering::Relaxed);
                }
                if MH_LAST_OV_IGNORE.load(Ordering::Relaxed) != 1 {
                    if let Some(w) = app_handle.get_webview_window("overlay") { crate::utils::set_window_click_through(&w, true); }
                    MH_LAST_OV_IGNORE.store(1, Ordering::Relaxed);
                }
                if MH_LAST_EDGE_HOVER.swap(0, Ordering::Relaxed) != 0 {
                    let _ = app_handle.emit_to("dock", "dock-edge-hover", false);
                }
                if MH_LAST_TOP_EDGE_HOVER.swap(0, Ordering::Relaxed) != 0 {
                    let _ = app_handle.emit_to("main", "notch-edge-hover", false);
                }
                return;
            }

            // Refresh cached monitor info every 1s
            if now - MH_LAST_MONITOR_UPDATE_MS.load(Ordering::Relaxed) > 1000 {
                if let Ok(Some(monitor)) = app_handle.primary_monitor() {
                    let pos = *monitor.position();
                    let size = *monitor.size();
                    *MH_CACHED_MON_POS.lock().unwrap() = Some((pos.x, pos.y));
                    *MH_CACHED_MON_SIZE.lock().unwrap() = Some((size.width, size.height));
                }
                if let Some(monitor) = crate::monitors::monitor_for_window_label(app_handle, crate::types::WindowKind::Dock, "dock") {
                    let pos = *monitor.position();
                    let size = *monitor.size();
                    *MH_CACHED_DOCK_MON_POS.lock().unwrap() = Some((pos.x, pos.y));
                    *MH_CACHED_DOCK_MON_SIZE.lock().unwrap() = Some((size.width, size.height));
                }
                if let Some(monitor) = crate::monitors::monitor_for_window_label(app_handle, crate::types::WindowKind::Notch, "main") {
                    let pos = *monitor.position();
                    let size = *monitor.size();
                    *MH_CACHED_NOTCH_MON_POS.lock().unwrap() = Some((pos.x, pos.y));
                    *MH_CACHED_NOTCH_MON_SIZE.lock().unwrap() = Some((size.width, size.height));
                }
                MH_LAST_MONITOR_UPDATE_MS.store(now, Ordering::Relaxed);
            }

            let cached_pos = MH_CACHED_MON_POS.lock().unwrap().unwrap_or((0, 0));
            let cached_size = MH_CACHED_MON_SIZE.lock().unwrap().unwrap_or((1920, 1080));
            let mon_x = cached_pos.0;
            let mon_y = cached_pos.1;
            let mon_w = cached_size.0 as i32;
            let mon_h = cached_size.1 as i32;

            let dock_cached_pos = MH_CACHED_DOCK_MON_POS.lock().unwrap().unwrap_or((mon_x, mon_y));
            let dock_cached_size = MH_CACHED_DOCK_MON_SIZE.lock().unwrap().unwrap_or((mon_w as u32, mon_h as u32));
            let (dock_mon_x, dock_mon_y) = dock_cached_pos;
            let (dock_mon_w, dock_mon_h) = (dock_cached_size.0 as i32, dock_cached_size.1 as i32);

            let notch_cached_pos = MH_CACHED_NOTCH_MON_POS.lock().unwrap().unwrap_or((mon_x, mon_y));
            let notch_cached_size = MH_CACHED_NOTCH_MON_SIZE.lock().unwrap().unwrap_or((mon_w as u32, mon_h as u32));
            let (notch_mon_x, notch_mon_y) = notch_cached_pos;
            let (notch_mon_w, notch_mon_h) = (notch_cached_size.0 as i32, notch_cached_size.1 as i32);

            let fg_fs = CURRENT_FOREGROUND_FULLSCREEN.load(Ordering::Relaxed);
            let dock_fg_fs = CURRENT_FOREGROUND_FULLSCREEN_ON_DOCK.load(Ordering::Relaxed);

            // --- Dock Interaction ---
            if let Some(dock_win) = app_handle.get_webview_window("dock") {
                if dock_win.is_visible().unwrap_or(false) {
                    let mut is_click_interactive = false;
                    let mut is_hovered = false;
                    let mut dock_span: Option<(i32, i32)> = None;
                    let mut dbg_box: Option<(i32, i32, i32, i32)> = None;
                    let mut dbg_rect: Option<crate::types::IntRect> = None;
                    let mut dbg_scale = 1.0;

                    let dock_rect_val = crate::state::dock_window_rects().lock().ok().and_then(|m| m.get("dock").copied());

                    if let Some((win_pos, win_size)) = dock_rect_val {
                        let in_window = cursor.x >= win_pos.x && cursor.x <= (win_pos.x + win_size.width as i32) &&
                                         cursor.y >= win_pos.y && cursor.y <= (win_pos.y + win_size.height as i32);

                        if in_window {
                            if let Ok(region) = crate::state::dock_rects().lock() {
                                if let Some(r) = region.get("dock").copied() {
                                    let scale = dock_win.scale_factor().unwrap_or(1.0);
                                    let pad_x = (5.0 * scale) as i32;
                                    let pad_y_top = (8.0 * scale) as i32;
                                    let pad_y_bottom = (5.0 * scale) as i32;
                                    // Hysteresis keeps the dock interactive a little past
                                    // its bounds once grabbed, so removing the edge-forced
                                    // interactivity doesn't reintroduce boundary flicker.
                                    let hyst = if MH_LAST_DOCK_IGNORE.load(Ordering::Relaxed) == 0 { (10.0 * scale) as i32 } else { 0 };
                                    let (left, top, right, bottom) = css_rect_to_physical_bounds(win_pos.x, win_pos.y, r, scale);
                                    let rx = left - pad_x - hyst;
                                    let ry = top - pad_y_top - hyst;
                                    let rw = (right - left) + (pad_x * 2) + (hyst * 2);
                                    let rh = (bottom - top) + pad_y_top + pad_y_bottom + (hyst * 2);
                                    if cursor.x >= rx && cursor.x <= (rx + rw) && cursor.y >= ry && cursor.y <= (ry + rh) {
                                        is_click_interactive = true;
                                    }
                                    dock_span = Some((rx, rx + rw));
                                    dbg_box = Some((rx, ry, rw, rh));
                                    dbg_rect = Some(r);
                                    dbg_scale = scale;
                                }
                            }

                            if !is_click_interactive && MENU_IS_OPEN.load(Ordering::Relaxed) {
                                if let Ok(rect) = MENU_RECT.lock() {
                                    if let Some(r) = *rect {
                                        let scale = dock_win.scale_factor().unwrap_or(1.0);
                                        let rx = win_pos.x + (r.x as f64 * scale) as i32 - (5.0 * scale) as i32;
                                        let ry = win_pos.y + (r.y as f64 * scale) as i32 - (5.0 * scale) as i32;
                                        let rw = (r.width as f64 * scale) as i32 + (10.0 * scale) as i32;
                                        let rh = (r.height as f64 * scale) as i32 + (10.0 * scale) as i32;
                                        if cursor.x >= rx && cursor.x <= (rx + rw) && cursor.y >= ry && cursor.y <= (ry + rh) {
                                            is_click_interactive = true;
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // Hot-edge detection (Bottom edge)
                    let in_dock_hover = crate::state::get_flag(crate::state::dock_hovered(), "dock");
                    let scale = dock_win.scale_factor().unwrap_or(1.0);
                    let over_dock_span = edge_area_allows_x(
                        edge_area_narrow(app_handle, "fern-dock-edge-area"),
                        dock_span,
                        cursor.x,
                        (60.0 * scale) as i32,
                    );
                    let at_bottom_edge = cursor.y >= (dock_mon_y + dock_mon_h - (8.0 * scale) as i32) &&
                                         cursor.x >= dock_mon_x && cursor.x <= (dock_mon_x + dock_mon_w) && over_dock_span;

                    if dock_fg_fs {
                        MH_DOCK_EXPIRY_MS.store(0, Ordering::Relaxed);
                    }
                    if dock_edge_hover_allowed(dock_fg_fs, at_bottom_edge, in_dock_hover) {
                        is_hovered = true;
                        MH_DOCK_EXPIRY_MS.store(now + 500, Ordering::Relaxed);
                    }

                    // Approaching along the bottom edge keeps the dock interactive
                    // while near its horizontal span, so the reveal can't be clicked
                    if at_bottom_edge && !dock_fg_fs {
                        handoff_other_dock_windows(app_handle, "dock");
                        if let Some((span_left, span_right)) = dock_span {
                            let edge_pad = (60.0 * scale) as i32;
                            if cursor.x >= span_left - edge_pad && cursor.x <= span_right + edge_pad {
                                is_click_interactive = true;
                            }
                        }
                    }

                    let final_dock_hover = is_hovered || now < MH_DOCK_EXPIRY_MS.load(Ordering::Relaxed);
                    if hover_waits_on_expiry(is_hovered, final_dock_hover) {
                        CURSOR_RECHECK_PENDING.store(true, Ordering::Relaxed);
                    }
                    let prev = MH_LAST_EDGE_HOVER.load(Ordering::Relaxed);
                    let new_val = if final_dock_hover { 1 } else { 0 };
                    if prev != new_val {
                        let _ = app_handle.emit_to("dock", "dock-edge-hover", final_dock_hover);
                        MH_LAST_EDGE_HOVER.store(new_val, Ordering::Relaxed);
                    }

                    crate::diagnostics::hitbox_log(0, now, || format!(
                        "hit dock cursor=({},{}) window={:?} css_rect={:?} scale={} box={:?} interactive={} ignore_before={}",
                        cursor.x, cursor.y,
                        dock_rect_val.map(|(p, sz)| (p.x, p.y, sz.width, sz.height)),
                        dbg_rect.map(|r| (r.x, r.y, r.width, r.height)),
                        dbg_scale, dbg_box, is_click_interactive, MH_LAST_DOCK_IGNORE.load(Ordering::Relaxed),
                    ));

                    let should_ignore = dock_fg_fs || (!is_click_interactive
                        && !MENU_IS_OPEN.load(Ordering::Relaxed)
                        && !DOCK_IS_DRAGGING.load(Ordering::Relaxed));
                    if dock_hover_went_stale(should_ignore, in_dock_hover) {
                        crate::state::set_flag(crate::state::dock_hovered(), "dock", false);
                        let _ = app_handle.emit_to("dock", "dock-cursor-left", ());
                    }
                    let prev_ignore = MH_LAST_DOCK_IGNORE.load(Ordering::Relaxed);
                    let new_ignore = if should_ignore { 1 } else { 0 };
                    if prev_ignore != new_ignore {
                        if let Ok(hwnd) = dock_win.hwnd() { re_assert_topmost(hwnd); }
                        crate::utils::set_window_click_through(&dock_win, should_ignore);
                        MH_LAST_DOCK_IGNORE.store(new_ignore, Ordering::Relaxed);
                    }
                } else {
                    if MH_LAST_EDGE_HOVER.swap(0, Ordering::Relaxed) != 0 {
                        let _ = app_handle.emit_to("dock", "dock-edge-hover", false);
                    }
                    if MH_LAST_DOCK_IGNORE.swap(1, Ordering::Relaxed) != 1 {
                        crate::utils::set_window_click_through(&dock_win, true);
                    }
                }
            }

            // --- Main (TopBar) Interaction ---
            {
                if let Some(main_win) = app_handle.get_webview_window("main") {
                    if main_win.is_visible().unwrap_or(false) {
                        let notch_fs = notch_suppressed(app_handle, "main");
                        let in_notch_hover = !notch_fs && crate::state::get_flag(crate::state::notch_hovered(), "main");
                        let mut is_notch_hovered = false;
                        let scale = main_win.scale_factor().unwrap_or(1.0);
                        let over_notch_span = edge_area_allows_x(
                            edge_area_narrow(app_handle, "fern-notch-edge-area"),
                            notch_x_span_for("main", scale),
                            cursor.x,
                            (60.0 * scale) as i32,
                        );
                        let notch_top_y = notch_target_top(notch_mon_x, notch_mon_y, notch_mon_w, notch_mon_h);
                        let in_top_band = (notch_top_y == notch_mon_y || cursor.y >= notch_top_y)
                            && cursor.y <= (notch_top_y + (8.0 * scale) as i32)
                            && cursor.x >= notch_mon_x && cursor.x <= (notch_mon_x + notch_mon_w) && over_notch_span;
                        let at_top_edge = top_edge_armed(app_handle, "main", in_top_band && !notch_fs, now);

                        if notch_fs {
                            MH_TOPBAR_EXPIRY_MS.store(0, Ordering::Relaxed);
                        }
                        if at_top_edge || in_notch_hover {
                            is_notch_hovered = true;
                            MH_TOPBAR_EXPIRY_MS.store(now + 500, Ordering::Relaxed);
                        }

                        let mut is_click_interactive = false;
                        let mut dbg_box: Option<(i32, i32, i32, i32)> = None;
                        let mut dbg_rect: Option<crate::types::IntRect> = None;
                        let main_rect_val = crate::state::main_window_rects().lock().ok().and_then(|m| m.get("main").copied());

                        if let Some((win_pos, _)) = main_rect_val {
                            if let Ok(region) = crate::state::notch_rects().lock() {
                                if let Some(r) = region.get("main").copied() {
                                    if r.width > 0 && r.height > 0 {
                                        let scale = main_win.scale_factor().unwrap_or(1.0);
                                        let pad_x = (20.0 * scale) as i32;
                                        let pad_y_bottom = (5.0 * scale) as i32;
                                        // Hysteresis keeps the notch interactive a little past
                                        // its bounds once grabbed, so removing the edge-forced
                                        // interactivity doesn't reintroduce boundary flicker.
                                        let hyst = if MH_LAST_MAIN_IGNORE.load(Ordering::Relaxed) == 0 { (10.0 * scale) as i32 } else { 0 };
                                        let rx = win_pos.x + (r.x as f64 * scale) as i32 - pad_x - hyst;
                                        let rw = (r.width as f64 * scale) as i32 + (pad_x * 2) + (hyst * 2);
                                        let y_off = (r.y.max(0) as f64 * scale) as i32;
                                        let ry_top = if r.y > 0 {
                                            (win_pos.y + y_off - (5.0 * scale) as i32 - hyst).max(win_pos.y)
                                        } else {
                                            win_pos.y
                                        };
                                        let ry_bottom = win_pos.y + y_off + (r.height as f64 * scale) as i32 + pad_y_bottom + hyst;

                                        // A hidden notch must not eat clicks, in smart/peek it's off screen most of the time.
                                        let notch_hidden = crate::state::get_flag(crate::state::notch_hidden(), "main");
                                        let hover_active = is_notch_hovered || now < MH_TOPBAR_EXPIRY_MS.load(Ordering::Relaxed);
                                        if (!notch_hidden || hover_active)
                                            && cursor.x >= rx && cursor.x <= (rx + rw) && cursor.y >= ry_top && cursor.y <= ry_bottom {
                                            is_click_interactive = true;
                                        }
                                        dbg_box = Some((rx, ry_top, rw, ry_bottom - ry_top));
                                        dbg_rect = Some(r);

                                        // Approaching along the top edge keeps the window
                                        // interactive while near the notch's horizontal
                                        // span, so peek/hover can't flicker at the
                                        // boundary. The screen corners stay click-through.
                                        // Keep this immediate (not dwell-gated), the notch rect lags the animation and flipping
                                        // click-through mid-hover makes it flicker. Still gated on the notch being visible or hovered.
                                        let edge_pad = (60.0 * scale) as i32;
                                        if !fg_fs && in_top_band && (!notch_hidden || hover_active)
                                            && cursor.x >= rx - edge_pad && cursor.x <= rx + rw + edge_pad {
                                            is_click_interactive = true;
                                        }

                                        if left_button_held() {
                                            let over_span = cursor.x >= rx - edge_pad && cursor.x <= rx + rw + edge_pad;
                                            let over_box = cursor.x >= rx && cursor.x <= rx + rw && cursor.y >= ry_top && cursor.y <= ry_bottom;
                                            if over_box || (in_top_band && over_span) {
                                                is_click_interactive = true;
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        if is_click_interactive && !in_notch_hover && !left_button_held() && caption_buttons_under(cursor) {
                            is_click_interactive = false;
                            is_notch_hovered = false;
                            MH_TOPBAR_EXPIRY_MS.store(0, Ordering::Relaxed);
                        }

                        if at_top_edge && !notch_fs && is_notch_hovered {
                            handoff_other_notch_windows(app_handle, "main");
                        }

                        let final_notch_hover = is_notch_hovered || now < MH_TOPBAR_EXPIRY_MS.load(Ordering::Relaxed);
                        if hover_waits_on_expiry(is_notch_hovered, final_notch_hover) {
                            CURSOR_RECHECK_PENDING.store(true, Ordering::Relaxed);
                        }
                        let prev = MH_LAST_TOP_EDGE_HOVER.load(Ordering::Relaxed);
                        let new_val = if final_notch_hover { 1 } else { 0 };
                        if prev != new_val {
                            let _ = app_handle.emit_to("main", "notch-edge-hover", final_notch_hover);
                            MH_LAST_TOP_EDGE_HOVER.store(new_val, Ordering::Relaxed);
                        }

                        crate::diagnostics::hitbox_log(1, now, || format!(
                            "hit notch cursor=({},{}) window={:?} css_rect={:?} scale={} box={:?} interactive={} at_top_edge={} ignore_before={}",
                            cursor.x, cursor.y,
                            main_rect_val.map(|(p, sz)| (p.x, p.y, sz.width, sz.height)),
                            dbg_rect.map(|r| (r.x, r.y, r.width, r.height)),
                            scale, dbg_box, is_click_interactive, at_top_edge, MH_LAST_MAIN_IGNORE.load(Ordering::Relaxed),
                        ));

                        let final_ignore = should_ignore_notch_cursor(
                            notch_fs,
                            is_click_interactive,
                        );
                        if dock_hover_went_stale(final_ignore, in_notch_hover) {
                            crate::state::set_flag(crate::state::notch_hovered(), "main", false);
                            let _ = app_handle.emit_to("main", "notch-cursor-left", ());
                        }
                        let prev_ignore = MH_LAST_MAIN_IGNORE.load(Ordering::Relaxed);
                        let new_ignore = if final_ignore { 1 } else { 0 };
                        if prev_ignore != new_ignore {
                            if let Ok(hwnd) = main_win.hwnd() { re_assert_topmost(hwnd); }
                            crate::utils::set_window_click_through(&main_win, final_ignore);
                            MH_LAST_MAIN_IGNORE.store(new_ignore, Ordering::Relaxed);
                        }
                    } else {
                        let prev = MH_LAST_TOP_EDGE_HOVER.load(Ordering::Relaxed);
                        if prev != 0 {
                            let _ = app_handle.emit_to("main", "notch-edge-hover", false);
                            MH_LAST_TOP_EDGE_HOVER.store(0, Ordering::Relaxed);
                        }
                        if MH_LAST_MAIN_IGNORE.swap(1, Ordering::Relaxed) != 1 {
                            crate::utils::set_window_click_through(&main_win, true);
                        }
                    }
                }
            }

            // Mixer open: its rect covers notch + panel, so it stays clickable.
            let mixer_rect = volume_mixer_physical_rect_for(app_handle);
            let in_mixer = mixer_rect.is_some_and(|(rx, ry, rw, rh)| {
                cursor.x >= rx && cursor.x <= rx + rw && cursor.y >= ry && cursor.y <= ry + rh
            });

            // The overlay zooms by fern-scale, so the cards are bigger than 42x196 off the default scale.
            let overlay_fern_scale = crate::utils::get_fern_scale(app_handle);

            let side_of = |key: &str, default: &str| -> bool {
                crate::utils::get_setting_str(app_handle, key).unwrap_or_else(|| default.to_string()) == "right"
            };
            let edge_dwell = crate::utils::get_setting_str(app_handle, "fern-edge-delay")
                .and_then(|v| v.parse::<i64>().ok())
                .unwrap_or(300)
                .clamp(0, 3000);
            let volume_right = side_of("fern-volume-side", "left");
            let brightness_right = side_of("fern-brightness-side", "right");

            if !fg_fs {
                let overlay_narrow = edge_area_narrow(app_handle, "fern-overlay-edge-area");
                let overlay_y_allowed = if !overlay_narrow {
                    true
                } else {
                    let (span, pad) = app_handle.primary_monitor().ok().flatten().map_or((None, 0), |m| {
                        let ms = m.size();
                        let mp = m.position();
                        let sc = m.scale_factor() * overlay_fern_scale;
                        (Some(overlay_edge_y_span(mp.y, ms.height as i32, sc)), (60.0 * sc) as i32)
                    });
                    edge_area_allows_x(true, span, cursor.y, pad)
                };
                let at_edge = |right: bool| {
                    overlay_y_allowed
                        && cursor.y >= mon_y && cursor.y <= (mon_y + mon_h)
                        && if right { cursor.x >= (mon_x + mon_w - 8) && cursor.x <= mon_x + mon_w } else { cursor.x >= mon_x && cursor.x <= (mon_x + 8) }
                };

                // The collapsed card sticks out past the 8px edge band, keep the hover alive over it too
                // or the HUD hides mid-click.
                let over_side_card = |right: bool| {
                    app_handle.primary_monitor().ok().flatten().is_some_and(|m| {
                        let ms = m.size();
                        let mp = m.position();
                        let sc = m.scale_factor() * overlay_fern_scale;
                        let nw = (42.0 * sc) as i32;
                        let nh = (196.0 * sc) as i32;
                        let nx = if right { mp.x + ms.width as i32 - nw } else { mp.x };
                        let ny = mp.y + (ms.height as i32 / 2) - (nh / 2);
                        cursor.x >= nx && cursor.x <= nx + nw && cursor.y >= ny && cursor.y <= ny + nh
                    })
                };

                let at_volume_edge = at_edge(volume_right);
                let over_card = !at_volume_edge
                    && !in_mixer
                    && MH_LAST_LEFT_EDGE_HOVER.load(Ordering::Relaxed) != 0
                    && over_side_card(volume_right);

                let volume_enter = if volume_right { &MH_RIGHT_ENTER_MS } else { &MH_LEFT_ENTER_MS };
                let volume_confirmed = edge_dwell_met(volume_enter, at_volume_edge, now, edge_dwell);
                if edge_dwell_waiting(at_volume_edge, volume_confirmed) {
                    CURSOR_RECHECK_PENDING.store(true, Ordering::Relaxed);
                }
                let volume_active = volume_confirmed || in_mixer || over_card;
                if volume_active {
                    MH_LEFT_EXPIRY_MS.store(now + 500, Ordering::Relaxed);
                }

                let final_volume_hover = now < MH_LEFT_EXPIRY_MS.load(Ordering::Relaxed);
                if hover_waits_on_expiry(volume_active, final_volume_hover) {
                    CURSOR_RECHECK_PENDING.store(true, Ordering::Relaxed);
                }
                let prev = MH_LAST_LEFT_EDGE_HOVER.load(Ordering::Relaxed);
                let new_val = if final_volume_hover { 1 } else { 0 };
                if prev != new_val {
                    let _ = app_handle.emit("volume-edge-hover", final_volume_hover);
                    MH_LAST_LEFT_EDGE_HOVER.store(new_val, Ordering::Relaxed);
                }

                let brightness_enter = if brightness_right { &MH_RIGHT_ENTER_MS } else { &MH_LEFT_ENTER_MS };
                let brightness_band = at_edge(brightness_right) && brightness_right != volume_right;
                let over_brightness_card = !brightness_band
                    && brightness_right != volume_right
                    && MH_LAST_RIGHT_EDGE_HOVER.load(Ordering::Relaxed) != 0
                    && over_side_card(brightness_right);
                let brightness_confirmed = edge_dwell_met(brightness_enter, brightness_band, now, edge_dwell);
                if edge_dwell_waiting(brightness_band, brightness_confirmed) {
                    CURSOR_RECHECK_PENDING.store(true, Ordering::Relaxed);
                }
                let brightness_active = brightness_confirmed || over_brightness_card;
                if brightness_active {
                    MH_RIGHT_EXPIRY_MS.store(now + 500, Ordering::Relaxed);
                }

                let final_brightness_hover = now < MH_RIGHT_EXPIRY_MS.load(Ordering::Relaxed);
                if hover_waits_on_expiry(brightness_active, final_brightness_hover) {
                    CURSOR_RECHECK_PENDING.store(true, Ordering::Relaxed);
                }
                let prev = MH_LAST_RIGHT_EDGE_HOVER.load(Ordering::Relaxed);
                let new_val = if final_brightness_hover { 1 } else { 0 };
                if prev != new_val {
                    let _ = app_handle.emit("brightness-edge-hover", final_brightness_hover);
                    MH_LAST_RIGHT_EDGE_HOVER.store(new_val, Ordering::Relaxed);
                }
            } else {
                if MH_LAST_LEFT_EDGE_HOVER.swap(0, Ordering::Relaxed) != 0 {
                    let _ = app_handle.emit("volume-edge-hover", false);
                }
                if MH_LAST_RIGHT_EDGE_HOVER.swap(0, Ordering::Relaxed) != 0 {
                    let _ = app_handle.emit("brightness-edge-hover", false);
                }
            }

            // --- Overlay cursor passthrough ---
            if let Some(ov_win) = app_handle.get_webview_window("overlay") {
                if fg_fs {
                    let prev = MH_LAST_OV_IGNORE.load(Ordering::Relaxed);
                    if prev != 1 {
                        if let Ok(hwnd) = ov_win.hwnd() { re_assert_topmost(hwnd); }
                        crate::utils::set_window_click_through(&ov_win, true);
                        MH_LAST_OV_IGNORE.store(1, Ordering::Relaxed);
                    }
                } else {
                    let over_left = in_mixer || if let Ok(Some(m)) = ov_win.primary_monitor() {
                        let ms = m.size();
                        let mp = m.position();
                        let sc = m.scale_factor() * overlay_fern_scale;
                        let nw = (42.0 * sc) as i32;
                        let nh = (196.0 * sc) as i32;
                        let nx = mp.x;
                        let ny = mp.y + (ms.height as i32 / 2) - (nh / 2);
                        cursor.x >= nx && cursor.x <= nx + nw && cursor.y >= ny && cursor.y <= ny + nh
                    } else { false };

                    let over_right = if let Ok(Some(m)) = ov_win.primary_monitor() {
                        let ms = m.size();
                        let mp = m.position();
                        let sc = m.scale_factor() * overlay_fern_scale;
                        let nw = (42.0 * sc) as i32;
                        let nh = (196.0 * sc) as i32;
                        let nx = mp.x + ms.width as i32 - nw;
                        let ny = mp.y + (ms.height as i32 / 2) - (nh / 2);
                        cursor.x >= nx && cursor.x <= nx + nw && cursor.y >= ny && cursor.y <= ny + nh
                    } else { false };

                    let should_ignore = !(over_left || over_right);
                    let prev = MH_LAST_OV_IGNORE.load(Ordering::Relaxed);
                    let new_val = if should_ignore { 1 } else { 0 };
                    if prev != new_val {
                        if let Ok(hwnd) = ov_win.hwnd() { re_assert_topmost(hwnd); }
                        crate::utils::set_window_click_through(&ov_win, should_ignore);
                        MH_LAST_OV_IGNORE.store(new_val, Ordering::Relaxed);
                    }
                }
            }

            hit_test_extra_dock_windows(app_handle, cursor, now, fg_fs);
            hit_test_extra_notch_windows(app_handle, cursor, now, fg_fs);
        }
    }
}

fn monitor_rect_for_hwnd(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, GetMonitorInfoA, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    unsafe {
        let h_monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoA(h_monitor, &mut mi).as_bool() {
            let r = mi.rcMonitor;
            Some((r.left, r.top, r.right - r.left, r.bottom - r.top))
        } else {
            None
        }
    }
}

fn cursor_monitor_origin() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoA, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    unsafe {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return None;
        }
        let h_monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoA(h_monitor, &mut mi).as_bool() {
            Some((mi.rcMonitor.left, mi.rcMonitor.top))
        } else {
            None
        }
    }
}

/// What the z-order shows on one monitor: the windows down to (and including) the first maximized
/// or monitor-sized one, which hides everything below it.
struct MonitorCover {
    origin: (i32, i32),
    bottom: i32,
    rects: Vec<windows::Win32::Foundation::RECT>,
    covered: bool,
    fullscreen: bool,
}

struct CoverScan {
    own_pid: u32,
    monitors: Vec<MonitorCover>,
}

unsafe extern "system" fn cover_scan_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::DWMWA_CLOAKED;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoA, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowRect, IsIconic, IsZoomed};
    let scan = &mut *(lparam.0 as *mut CoverScan);
    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        return true.into();
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == scan.own_pid || (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0) != 0 {
        return true.into();
    }
    let mut cloaked = 0u32;
    if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok() && cloaked != 0 {
        return true.into();
    }
    let mut class = [0u16; 64];
    let len = GetClassNameW(hwnd, &mut class).max(0) as usize;
    if matches!(String::from_utf16_lossy(&class[..len]).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
        return true.into();
    }
    let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if !GetMonitorInfoA(monitor, &mut mi).as_bool() {
        return true.into();
    }
    let screen = mi.rcMonitor;
    let origin = (screen.left, screen.top);
    let index = match scan.monitors.iter().position(|m| m.origin == origin) {
        Some(i) => i,
        None => {
            scan.monitors.push(MonitorCover { origin, bottom: screen.bottom, rects: Vec::new(), covered: false, fullscreen: false });
            scan.monitors.len() - 1
        }
    };
    let entry = &mut scan.monitors[index];
    if entry.covered {
        return true.into();
    }
    let mut rect = RECT::default();
    let has_rect = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_ok()
        || GetWindowRect(hwnd, &mut rect).is_ok();
    if !has_rect {
        return true.into();
    }
    entry.rects.push(rect);
    // Small windows (popups, picture-in-picture) above a fullscreen app don't decide whether the
    // monitor is fullscreen; a maximized or monitor-sized window does.
    let covers = rect.left <= screen.left && rect.top <= screen.top && rect.right >= screen.right && rect.bottom >= screen.bottom;
    if covers || IsZoomed(hwnd).as_bool() {
        entry.covered = true;
        entry.fullscreen = crate::utils::is_window_fullscreen(hwnd);
    }
    true.into()
}

/// Per monitor, the windows that are not hidden behind a maximized or monitor-sized one. Read from
/// the z-order rather than focus, so a window keeps its monitor's panels hidden while another
/// monitor has focus.
fn scan_monitor_covers(own_pid: u32) -> Vec<MonitorCover> {
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    let mut scan = CoverScan { own_pid, monitors: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(cover_scan_proc), LPARAM(&mut scan as *mut CoverScan as isize));
    }
    scan.monitors
}

/// Monitor whose overlap and fullscreen state follow the foreground window rather than the
/// z-order scan. Returns `None` unless the foreground window was actually measured (`fg_info`)
/// or is a known shell surface, so transient/unmeasured windows never claim a monitor.
fn resolve_focused_monitor(
    hwnd_valid: bool,
    is_desktop_focused: bool,
    is_known_shell: bool,
    cursor_origin: Option<(i32, i32)>,
    fg_screen_origin: Option<(i32, i32)>,
) -> Option<(i32, i32)> {
    if !hwnd_valid || is_desktop_focused {
        None
    } else if is_known_shell {
        cursor_origin
    } else {
        fg_screen_origin
    }
}

/// Fullscreen state of the monitor at `origin`. The focused monitor keeps the foreground check;
/// the others use the z-order scan.
fn monitor_fullscreen(origin: (i32, i32), focused_monitor: Option<(i32, i32)>, focused_is_fullscreen: bool, covered: &[(i32, i32)]) -> bool {
    if focused_monitor == Some(origin) { focused_is_fullscreen } else { covered.contains(&origin) }
}

struct CaptionProbe {
    point: windows::Win32::Foundation::POINT,
    own_pid: u32,
    over_buttons: bool,
}

unsafe extern "system" fn caption_probe_proc(hwnd: HWND, lparam: windows::Win32::Foundation::LPARAM) -> windows::core::BOOL {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CAPTION_BUTTON_BOUNDS, DWMWA_CLOAKED};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible, GWL_EXSTYLE,
        WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    };
    let probe = &mut *(lparam.0 as *mut CaptionProbe);
    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        return true.into();
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == probe.own_pid {
        return true.into();
    }
    let ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex & (WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0) != 0 {
        return true.into();
    }
    let mut cloaked = 0u32;
    if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok() && cloaked != 0 {
        return true.into();
    }
    let mut class = [0u16; 64];
    let len = GetClassNameW(hwnd, &mut class).max(0) as usize;
    if matches!(String::from_utf16_lossy(&class[..len]).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
        return true.into();
    }
    let mut wr = RECT::default();
    if GetWindowRect(hwnd, &mut wr).is_err() {
        return true.into();
    }
    let p = probe.point;
    if p.x < wr.left || p.x >= wr.right || p.y < wr.top || p.y >= wr.bottom {
        return true.into();
    }
    let mut cb = RECT::default();
    let known = DwmGetWindowAttribute(hwnd, DWMWA_CAPTION_BUTTON_BOUNDS, &mut cb as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_ok()
        && cb.right > cb.left && cb.bottom > cb.top;
    probe.over_buttons = if known {
        let (x, y) = (p.x - wr.left, p.y - wr.top);
        x >= cb.left && x < cb.right && y >= cb.top && y < cb.bottom
    } else {
        p.y - wr.top < 40 && wr.right - p.x < 150
    };
    false.into()
}

/// True when the top-most app window under the cursor has its close, minimize or maximize buttons there.
fn caption_buttons_under(point: windows::Win32::Foundation::POINT) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    let mut probe = CaptionProbe { point, own_pid: std::process::id(), over_buttons: false };
    unsafe {
        let _ = EnumWindows(Some(caption_probe_proc), windows::Win32::Foundation::LPARAM(&mut probe as *mut _ as isize));
    }
    probe.over_buttons
}


/// Converts a CSS-pixel rect inside a window at `(win_x, win_y)` to physical screen bounds
/// `(left, top, right, bottom)`, rounding outwards so fractional DPI edges are never clipped.
fn css_rect_to_physical_bounds(win_x: i32, win_y: i32, r: crate::types::IntRect, scale: f64) -> (i32, i32, i32, i32) {
    let left = win_x + (r.x as f64 * scale).floor() as i32;
    let top = win_y + (r.y as f64 * scale).floor() as i32;
    let right = win_x + ((r.x + r.width) as f64 * scale).ceil() as i32;
    let bottom = win_y + ((r.y + r.height) as f64 * scale).ceil() as i32;
    (left, top, right, bottom)
}

fn hit_test_extra_dock_windows(app_handle: &AppHandle, cursor: windows::Win32::Foundation::POINT, now: i64, _fg_fs: bool) {
    let windows: Vec<(String, tauri::WebviewWindow)> = app_handle.webview_windows().into_iter()
        .filter(|(l, _)| is_dock_label(l) && crate::state::monitor_suffix(l).is_some())
        .collect();

    for (label, dock_win) in windows {
        if !dock_win.is_visible().unwrap_or(false) {
            if crate::state::get_i32(crate::state::dock_extra_last_ignore(), &label, -1) != 1 {
                crate::utils::set_window_click_through(&dock_win, true);
                crate::state::set_i32(crate::state::dock_extra_last_ignore(), &label, 1);
            }
            continue;
        }
        let Ok(hwnd) = dock_win.hwnd() else { continue };
        let Some((mon_x, mon_y, mon_w, mon_h)) = monitor_rect_for_hwnd(hwnd) else { continue };

        let mut is_click_interactive = false;
        let mut dock_span: Option<(i32, i32)> = None;
        let scale = dock_win.scale_factor().unwrap_or(1.0);

        let dock_rect_val = crate::state::dock_window_rects().lock().ok().and_then(|m| m.get(&label).copied());
        if let Some((win_pos, win_size)) = dock_rect_val {
            let in_window = cursor.x >= win_pos.x && cursor.x <= (win_pos.x + win_size.width as i32) &&
                             cursor.y >= win_pos.y && cursor.y <= (win_pos.y + win_size.height as i32);
            if in_window {
                if let Some(r) = crate::state::dock_rects().lock().ok().and_then(|m| m.get(&label).copied()) {
                    let pad_x = (5.0 * scale) as i32;
                    let pad_y_top = (8.0 * scale) as i32;
                    let pad_y_bottom = (5.0 * scale) as i32;
                    let last_ignore = crate::state::get_i32(crate::state::dock_extra_last_ignore(), &label, -1);
                    let hyst = if last_ignore == 0 { (10.0 * scale) as i32 } else { 0 };
                    let (left, top, right, bottom) = css_rect_to_physical_bounds(win_pos.x, win_pos.y, r, scale);
                    let rx = left - pad_x - hyst;
                    let ry = top - pad_y_top - hyst;
                    let rw = (right - left) + (pad_x * 2) + (hyst * 2);
                    let rh = (bottom - top) + pad_y_top + pad_y_bottom + (hyst * 2);
                    if cursor.x >= rx && cursor.x <= (rx + rw) && cursor.y >= ry && cursor.y <= (ry + rh) {
                        is_click_interactive = true;
                    }
                    dock_span = Some((rx, rx + rw));
                }
            }

            if !is_click_interactive && MENU_IS_OPEN.load(Ordering::Relaxed) {
                if let Ok(rect) = MENU_RECT.lock() {
                    if let Some(r) = *rect {
                        let rx = win_pos.x + (r.x as f64 * scale) as i32 - (5.0 * scale) as i32;
                        let ry = win_pos.y + (r.y as f64 * scale) as i32 - (5.0 * scale) as i32;
                        let rw = (r.width as f64 * scale) as i32 + (10.0 * scale) as i32;
                        let rh = (r.height as f64 * scale) as i32 + (10.0 * scale) as i32;
                        if cursor.x >= rx && cursor.x <= (rx + rw) && cursor.y >= ry && cursor.y <= (ry + rh) {
                            is_click_interactive = true;
                        }
                    }
                }
            }
        }

        let in_dock_hover = crate::state::get_flag(crate::state::dock_hovered(), &label);
        let over_dock_span = edge_area_allows_x(
            edge_area_narrow(app_handle, "fern-dock-edge-area"),
            dock_span,
            cursor.x,
            (60.0 * scale) as i32,
        );
        let at_bottom_edge = cursor.y >= (mon_y + mon_h - (8.0 * scale) as i32) &&
                             cursor.x >= mon_x && cursor.x <= (mon_x + mon_w) && over_dock_span;

        let dock_fs = crate::state::get_flag(crate::state::dock_fullscreen(), &label);
        if dock_fs {
            crate::state::set_i64(crate::state::dock_extra_expiry_ms(), &label, 0);
        }

        let mut is_hovered = false;
        if !dock_fs && (at_bottom_edge || in_dock_hover) {
            is_hovered = true;
            crate::state::set_i64(crate::state::dock_extra_expiry_ms(), &label, now + 500);
        }

        if at_bottom_edge && !dock_fs {
            handoff_other_dock_windows(app_handle, &label);
            if let Some((span_left, span_right)) = dock_span {
                let edge_pad = (60.0 * scale) as i32;
                if cursor.x >= span_left - edge_pad && cursor.x <= span_right + edge_pad {
                    is_click_interactive = true;
                }
            }
        }

        let expiry = crate::state::get_i64(crate::state::dock_extra_expiry_ms(), &label, 0);
        let final_dock_hover = is_hovered || now < expiry;
        if hover_waits_on_expiry(is_hovered, final_dock_hover) {
            CURSOR_RECHECK_PENDING.store(true, Ordering::Relaxed);
        }
        let prev_hover = crate::state::get_i32(crate::state::dock_extra_last_edge_hover(), &label, -1);
        let new_hover = if final_dock_hover { 1 } else { 0 };
        if prev_hover != new_hover {
            let _ = app_handle.emit_to(label.as_str(), "dock-edge-hover", final_dock_hover);
            crate::state::set_i32(crate::state::dock_extra_last_edge_hover(), &label, new_hover);
        }

        let should_ignore = dock_fs || (!is_click_interactive
            && !MENU_IS_OPEN.load(Ordering::Relaxed)
            && !DOCK_IS_DRAGGING.load(Ordering::Relaxed));
        if dock_hover_went_stale(should_ignore, in_dock_hover) {
            crate::state::set_flag(crate::state::dock_hovered(), &label, false);
            let _ = app_handle.emit_to(label.as_str(), "dock-cursor-left", ());
        }
        let prev_ignore = crate::state::get_i32(crate::state::dock_extra_last_ignore(), &label, -1);
        let new_ignore = if should_ignore { 1 } else { 0 };
        if prev_ignore != new_ignore {
            re_assert_topmost(hwnd);
            crate::utils::set_window_click_through(&dock_win, should_ignore);
            crate::state::set_i32(crate::state::dock_extra_last_ignore(), &label, new_ignore);
        }
    }
}

/// Held long enough to be a drag rather than a click.
fn left_button_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
    static DOWN_SINCE_MS: AtomicI64 = AtomicI64::new(0);
    let down = unsafe { (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0 };
    if !down {
        DOWN_SINCE_MS.store(0, Ordering::Relaxed);
        return false;
    }
    let now = now_ms();
    let since = DOWN_SINCE_MS.load(Ordering::Relaxed);
    if since == 0 {
        DOWN_SINCE_MS.store(now, Ordering::Relaxed);
        return false;
    }
    now - since >= 200
}

fn hit_test_extra_notch_windows(app_handle: &AppHandle, cursor: windows::Win32::Foundation::POINT, now: i64, fg_fs: bool) {
    let windows: Vec<(String, tauri::WebviewWindow)> = app_handle.webview_windows().into_iter()
        .filter(|(l, _)| is_notch_label(l) && crate::state::monitor_suffix(l).is_some())
        .collect();

    for (label, main_win) in windows {
        if !main_win.is_visible().unwrap_or(false) {
            if crate::state::get_i32(crate::state::notch_extra_last_ignore(), &label, -1) != 1 {
                crate::utils::set_window_click_through(&main_win, true);
                crate::state::set_i32(crate::state::notch_extra_last_ignore(), &label, 1);
            }
            continue;
        }
        let Ok(hwnd) = main_win.hwnd() else { continue };
        let Some((mon_x, mon_y, mon_w, mon_h)) = monitor_rect_for_hwnd(hwnd) else { continue };

        let scale = main_win.scale_factor().unwrap_or(1.0);
        let notch_fs = notch_suppressed(app_handle, &label);
        let in_notch_hover = !notch_fs && crate::state::get_flag(crate::state::notch_hovered(), &label);
        let over_notch_span = edge_area_allows_x(
            edge_area_narrow(app_handle, "fern-notch-edge-area"),
            notch_x_span_for(&label, scale),
            cursor.x,
            (60.0 * scale) as i32,
        );
        let notch_top_y = notch_target_top(mon_x, mon_y, mon_w, mon_h);
        let in_top_band = (notch_top_y == mon_y || cursor.y >= notch_top_y)
            && cursor.y <= (notch_top_y + (8.0 * scale) as i32)
            && cursor.x >= mon_x && cursor.x <= (mon_x + mon_w) && over_notch_span;
        let at_top_edge = top_edge_armed(app_handle, &label, in_top_band && !notch_fs, now);

        let mut is_notch_hovered = false;
        if notch_fs {
            crate::state::set_i64(crate::state::notch_extra_expiry_ms(), &label, 0);
        }
        if at_top_edge || in_notch_hover {
            is_notch_hovered = true;
            crate::state::set_i64(crate::state::notch_extra_expiry_ms(), &label, now + 500);
        }

        let mut is_click_interactive = false;
        let main_rect_val = crate::state::main_window_rects().lock().ok().and_then(|m| m.get(&label).copied());
        if let Some((win_pos, _)) = main_rect_val {
            if let Some(r) = crate::state::notch_rects().lock().ok().and_then(|m| m.get(&label).copied()) {
                if r.width > 0 && r.height > 0 {
                    let pad_x = (20.0 * scale) as i32;
                    let pad_y_bottom = (5.0 * scale) as i32;
                    let last_ignore = crate::state::get_i32(crate::state::notch_extra_last_ignore(), &label, -1);
                    let hyst = if last_ignore == 0 { (10.0 * scale) as i32 } else { 0 };
                    let rx = win_pos.x + (r.x as f64 * scale) as i32 - pad_x - hyst;
                    let rw = (r.width as f64 * scale) as i32 + (pad_x * 2) + (hyst * 2);
                    let y_off = (r.y.max(0) as f64 * scale) as i32;
                    let ry_top = if r.y > 0 {
                        (win_pos.y + y_off - (5.0 * scale) as i32 - hyst).max(win_pos.y)
                    } else {
                        win_pos.y
                    };
                    let ry_bottom = win_pos.y + y_off + (r.height as f64 * scale) as i32 + pad_y_bottom + hyst;

                    let notch_hidden = crate::state::get_flag(crate::state::notch_hidden(), &label);
                    let hover_active = is_notch_hovered
                        || now < crate::state::get_i64(crate::state::notch_extra_expiry_ms(), &label, 0);
                    if (!notch_hidden || hover_active)
                        && cursor.x >= rx && cursor.x <= (rx + rw) && cursor.y >= ry_top && cursor.y <= ry_bottom {
                        is_click_interactive = true;
                    }

                    let edge_pad = (60.0 * scale) as i32;
                    if !fg_fs && in_top_band && (!notch_hidden || hover_active)
                        && cursor.x >= rx - edge_pad && cursor.x <= rx + rw + edge_pad {
                        is_click_interactive = true;
                    }

                    if left_button_held() {
                        let over_span = cursor.x >= rx - edge_pad && cursor.x <= rx + rw + edge_pad;
                        let over_box = cursor.x >= rx && cursor.x <= rx + rw && cursor.y >= ry_top && cursor.y <= ry_bottom;
                        if over_box || (in_top_band && over_span) {
                            is_click_interactive = true;
                        }
                    }
                }
            }
        }

        if is_click_interactive && !in_notch_hover && !left_button_held() && caption_buttons_under(cursor) {
            is_click_interactive = false;
            is_notch_hovered = false;
            crate::state::set_i64(crate::state::notch_extra_expiry_ms(), &label, 0);
        }

        if at_top_edge && !notch_fs && is_notch_hovered {
            handoff_other_notch_windows(app_handle, &label);
        }

        let expiry = crate::state::get_i64(crate::state::notch_extra_expiry_ms(), &label, 0);
        let final_notch_hover = is_notch_hovered || now < expiry;
        if hover_waits_on_expiry(is_notch_hovered, final_notch_hover) {
            CURSOR_RECHECK_PENDING.store(true, Ordering::Relaxed);
        }
        let prev_hover = crate::state::get_i32(crate::state::notch_extra_last_edge_hover(), &label, -1);
        let new_hover = if final_notch_hover { 1 } else { 0 };
        if prev_hover != new_hover {
            let _ = app_handle.emit_to(label.as_str(), "notch-edge-hover", final_notch_hover);
            crate::state::set_i32(crate::state::notch_extra_last_edge_hover(), &label, new_hover);
        }

        let final_ignore = should_ignore_notch_cursor(
            notch_fs,
            is_click_interactive,
        );
        if dock_hover_went_stale(final_ignore, in_notch_hover) {
            crate::state::set_flag(crate::state::notch_hovered(), &label, false);
            let _ = app_handle.emit_to(label.as_str(), "notch-cursor-left", ());
        }
        let prev_ignore = crate::state::get_i32(crate::state::notch_extra_last_ignore(), &label, -1);
        let new_ignore = if final_ignore { 1 } else { 0 };
        if prev_ignore != new_ignore {
            re_assert_topmost(hwnd);
            crate::utils::set_window_click_through(&main_win, final_ignore);
            crate::state::set_i32(crate::state::notch_extra_last_ignore(), &label, new_ignore);
        }
    }
}

pub fn trigger_app_scan() {
    if IS_SCANNING.load(Ordering::Relaxed) { return; }
    IS_SCANNING.store(true, Ordering::Relaxed);
    
    std::thread::spawn(|| {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, CoTaskMemFree};
        use windows::Win32::UI::Shell::{SHGetKnownFolderIDList, FOLDERID_AppsFolder, SHGetDesktopFolder, IShellFolder, IEnumIDList, SHGetNameFromIDList, SIGDN_NORMALDISPLAY, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH, SIGDN_URL};
        let mut apps = Vec::new();
        unsafe {
            let com_ok = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            // Use a scope to ensure COM objects are dropped before CoUninitialize
            {
                if let Ok(pidl_apps) = SHGetKnownFolderIDList(&FOLDERID_AppsFolder, 0, None) {
                    if let Ok(desktop) = SHGetDesktopFolder() {
                        if let Ok(apps_folder) = desktop.BindToObject::<_, IShellFolder>(pidl_apps, None) {
                            let mut enum_id: Option<IEnumIDList> = None;
                            let res = apps_folder.EnumObjects(HWND(std::ptr::null_mut()), (windows::Win32::UI::Shell::SHCONTF_FOLDERS.0 | windows::Win32::UI::Shell::SHCONTF_NONFOLDERS.0) as u32, &mut enum_id);
                            
                            if res.is_ok() {
                                if let Some(enum_id) = enum_id {
                                    let mut items = [std::ptr::null_mut(); 1];
                                    let mut fetched = 0;
                                    while enum_id.Next(&mut items, Some(&mut fetched)).is_ok() && fetched > 0 {
                                        let pidl_item = items[0];
                                        let abs_item = windows::Win32::UI::Shell::ILCombine(Some(pidl_apps as *const _), Some(pidl_item as *const _));

                                        let name = if let Ok(n_ptr) = SHGetNameFromIDList(abs_item, SIGDN_NORMALDISPLAY) {
                                            let s = String::from_utf16_lossy(windows::core::PCWSTR(n_ptr.0).as_wide());
                                            CoTaskMemFree(Some(n_ptr.0 as *const _));
                                            s
                                        } else { "Unknown".to_string() };

                                        // Parsing name: exe path for Win32 apps, AUMID for packaged ones.
                                        let path = if let Ok(p_ptr) = SHGetNameFromIDList(abs_item, SIGDN_DESKTOPABSOLUTEPARSING) {
                                            let s = String::from_utf16_lossy(windows::core::PCWSTR(p_ptr.0).as_wide());
                                            CoTaskMemFree(Some(p_ptr.0 as *const _));
                                            s
                                        } else if let Ok(p_ptr) = SHGetNameFromIDList(abs_item, SIGDN_FILESYSPATH) {
                                            let s = String::from_utf16_lossy(windows::core::PCWSTR(p_ptr.0).as_wide());
                                            CoTaskMemFree(Some(p_ptr.0 as *const _));
                                            s
                                        } else if let Ok(p_ptr) = SHGetNameFromIDList(abs_item, SIGDN_URL) {
                                            let s = String::from_utf16_lossy(windows::core::PCWSTR(p_ptr.0).as_wide());
                                            CoTaskMemFree(Some(p_ptr.0 as *const _));
                                            s
                                        } else { name.clone() };

                                        if is_launchable_entry(&name, &path) {
                                            // Icons are fetched lazily by get_app_icon and cached in ICON_CACHE so get_installed_apps stays tiny and instant.
                                            let executable = if path.contains('\\') || path.contains('/') {
                                                std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).map(|s| s.to_string())
                                            } else {
                                                None
                                            };
                                            apps.push(AppInfo {
                                                name,
                                                path,
                                                icon: None,
                                                is_running: false,
                                                hwnd: None,
                                                executable,
                                                all_hwnds: None,
                                                args: None,
                                            });
                                        }
                                        if !abs_item.is_null() { CoTaskMemFree(Some(abs_item as *const _)); }
                                        CoTaskMemFree(Some(pidl_item as *const _));
                                        items[0] = std::ptr::null_mut();
                                    }
                                }
                            }
                        }
                    }
                    CoTaskMemFree(Some(pidl_apps as *const _));
                }
            } // Close COM scope
            if com_ok { CoUninitialize(); }
        }

        // Scan Start Menu .lnk shortcuts (depth-limited, no .exe scanning)
        // This catches Win32 apps that FOLDERID_AppsFolder may miss
        let mut start_menu_dirs: Vec<String> = Vec::new();
        if let Ok(programdata) = std::env::var("PROGRAMDATA") {
            start_menu_dirs.push(format!(r"{}\Microsoft\Windows\Start Menu\Programs", programdata));
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            start_menu_dirs.push(format!(r"{}\Microsoft\Windows\Start Menu\Programs", appdata));
        }

        // Shortcut resolution uses IShellLinkW, which needs COM on this thread.
        let mut resolved_target_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut shortcut_targets: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let shortcut_com_ok = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
        for dir in &start_menu_dirs {
            let root = std::path::Path::new(dir);
            if root.exists() {
                collect_shortcuts(root, &mut apps, &mut resolved_target_names, &mut shortcut_targets, 0);
            }
        }
        if shortcut_com_ok { unsafe { CoUninitialize(); } }

        if let Ok(mut map) = RESOLVED_TARGET_NAMES
            .get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()))
            .write()
        {
            *map = resolved_target_names;
        }

        if let Some(c) = INSTALLED_APPS_CACHE.get() {
            if let Ok(mut lock) = c.lock() {
                *lock = apps;
            }
        }
        IS_SCANNING.store(false, Ordering::Relaxed);
    });
}

/// Uninstallers, installers, help, docs, troubleshooters. Whole words only, so "Helper" or "Remover" stay.
fn is_helper_name(name: &str) -> bool {
    const HELPER_WORDS: &[&str] = &[
        "uninstall", "uninstaller", "installer", "setup", "help", "readme", "manual",
        "documentation", "troubleshoot", "troubleshooting", "repair",
    ];
    const HELPER_PHRASES: &[&str] = &[
        "release notes",
        "desktop overlay host",
        "debuggable package manager",
        "developer command prompt",
        "developer powershell",
        "crash reporter",
        "crash handler",
    ];
    let lower = name.to_lowercase();
    if HELPER_PHRASES.iter().any(|p| lower.contains(p)) {
        return true;
    }
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|token| HELPER_WORDS.contains(&token))
}

/// Drops entries that aren't real apps (web links, documents, protocol handlers).
fn is_launchable_entry(name: &str, path: &str) -> bool {
    if name.is_empty() || name == "Unknown" || is_helper_name(name) { return false; }
    let lower = path.to_lowercase();
    if lower.is_empty() { return false; }
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("file:")
        || lower.starts_with("steam:") || lower.starts_with("::{")
        || (lower.starts_with("shell:") && !lower.starts_with("shell:appsfolder")) {
        return false;
    }
    true
}

fn is_shared_host_exe(exe: &str) -> bool {
    const SHARED_HOST_EXES: &[&str] = &[
        "cmd.exe", "powershell.exe", "pwsh.exe", "wt.exe", "explorer.exe", "mmc.exe",
        "rundll32.exe", "mshta.exe", "wscript.exe", "cscript.exe", "javaw.exe", "java.exe",
        "python.exe", "pythonw.exe",
    ];
    SHARED_HOST_EXES.iter().any(|&host| exe.eq_ignore_ascii_case(host))
}

fn shortcut_display_name(path: &std::path::Path) -> String {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_DISPLAYNAME, SHGetFileInfoW};

    let fallback = || {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    };

    unsafe {
        let mut shfi: SHFILEINFOW = std::mem::zeroed();
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let res = SHGetFileInfoW(
            windows::core::PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_DISPLAYNAME,
        );
        if res != 0 {
            let len = shfi
                .szDisplayName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(shfi.szDisplayName.len());
            let raw = String::from_utf16_lossy(&shfi.szDisplayName[..len]).trim().to_string();
            let cleaned = if raw.to_ascii_lowercase().ends_with(".lnk") {
                raw[..raw.len() - 4].trim().to_string()
            } else {
                raw
            };
            if !cleaned.is_empty() {
                return cleaned;
            }
        }
    }
    fallback()
}

static RESOLVED_TARGET_NAMES: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, String>>> =
    std::sync::OnceLock::new();

fn normalize_exe_path(path: &str) -> String {
    path.trim().replace('/', "\\").to_lowercase()
}

fn exe_parent_dir(path: &str) -> Option<String> {
    let norm = normalize_exe_path(path);
    if !norm.ends_with(".exe") {
        return None;
    }
    let idx = norm.rfind('\\')?;
    Some(norm[..idx].to_string())
}

fn is_duplicate_shortcut(
    existing: &AppInfo,
    existing_resolved_target: Option<&str>,
    name: &str,
    path_str: &str,
    is_pwa: bool,
    executable: Option<&str>,
    resolved_target: Option<&str>,
) -> bool {
    if existing.name.eq_ignore_ascii_case(name) || existing.path.eq_ignore_ascii_case(path_str) {
        return true;
    }
    if is_pwa {
        return false;
    }
    let Some(exe) = executable else {
        return false;
    };
    if is_shared_host_exe(exe) {
        return false;
    }
    let same_exe = existing
        .executable
        .as_deref()
        .is_some_and(|e| e.eq_ignore_ascii_case(exe));
    if !same_exe {
        return false;
    }
    let existing_target = existing_resolved_target.or_else(|| {
        existing
            .path
            .to_ascii_lowercase()
            .ends_with(".exe")
            .then_some(existing.path.as_str())
    });
    match (existing_target, resolved_target) {
        (Some(et), Some(rt)) => {
            normalize_exe_path(et) == normalize_exe_path(rt)
                || exe_parent_dir(et)
                    .is_some_and(|ed| exe_parent_dir(rt).as_deref() == Some(ed.as_str()))
        }
        _ => false,
    }
}

fn collect_shortcuts(
    dir: &std::path::Path,
    apps: &mut Vec<AppInfo>,
    resolved_target_names: &mut std::collections::HashMap<String, String>,
    shortcut_targets: &mut std::collections::HashMap<String, String>,
    depth: i32,
) {
    if depth > 3 { return; }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_shortcuts(&path, apps, resolved_target_names, shortcut_targets, depth + 1);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
                let name = shortcut_display_name(&path);
                if name.is_empty() || is_helper_name(&name) || name.starts_with("Install") { continue; }

                // Keep the .lnk itself, its args identify PWAs and must be passed on launch.
                // The target goes in as the exe name so pins still match running windows.
                let path_str = path.to_string_lossy().to_string();
                let resolved = crate::utils::resolve_shortcut(&path_str);
                let resolved_target = resolved.as_ref().map(|(t, _)| t.as_str());
                let args = resolved.as_ref().map(|(_, a)| a.trim().to_string()).filter(|a| !a.is_empty());
                let is_pwa = args.as_deref().is_some_and(|a| a.contains("--app-id=") || a.contains("--app="));
                let executable = if is_pwa {
                    None
                } else {
                    resolved_target.and_then(|target| {
                        let file = std::path::Path::new(target).file_name().and_then(|n| n.to_str())?.to_string();
                        // UWP shortcuts launch explorer.exe with shell:AppsFolder, storing that would match File Explorer windows.
                        if file.eq_ignore_ascii_case("explorer.exe") { None } else { Some(file) }
                    })
                };

                if !is_pwa {
                    if let (Some(exe), Some(target)) = (executable.as_deref(), resolved_target) {
                        if !is_shared_host_exe(exe) {
                            let norm_target = normalize_exe_path(target);
                            if norm_target.ends_with(".exe") {
                                resolved_target_names.entry(norm_target).or_insert_with(|| name.clone());
                            }
                        }
                    }
                }

                let duplicate = apps.iter().any(|a| {
                    let existing_target = shortcut_targets.get(&a.path.to_lowercase()).map(|s| s.as_str());
                    is_duplicate_shortcut(
                        a,
                        existing_target,
                        &name,
                        &path_str,
                        is_pwa,
                        executable.as_deref(),
                        resolved_target,
                    )
                });
                if !duplicate {
                    if let Some(target) = resolved_target {
                        shortcut_targets.insert(path_str.to_lowercase(), target.to_string());
                    }
                    apps.push(AppInfo {
                        name,
                        path: path_str,
                        icon: None,
                        is_running: false,
                        hwnd: None,
                        executable,
                        all_hwnds: None,
                        args,
                    });
                }
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("url")) {
                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                if is_helper_name(&name) || name.starts_with("Install") { continue; }
                let path_str = path.to_string_lossy().to_string();
                if !apps.iter().any(|a| a.name == name || a.path == path_str) {
                    apps.push(AppInfo {
                        name,
                        path: path_str,
                        icon: None,
                        is_running: false,
                        hwnd: None,
                        executable: None,
                        all_hwnds: None,
                        args: None,
                    });
                }
            }
        }
    }
}

pub fn sync_overlays(app: &AppHandle) {
    // Don't reposition while splash is playing
    if crate::state::OVERLAY_IN_SPLASH.load(Ordering::Relaxed) {
        return;
    }
    // Full-screen overlay — notches render at left/right edges via CSS
    // The window covers the entire primary monitor so it never needs repositioning
    if let Some(ov_win) = app.get_webview_window("overlay") {
        let hwnd_res = ov_win.hwnd();
        if let Ok(Some(monitor)) = ov_win.primary_monitor() {
            let size = monitor.size();
            let pos = monitor.position();
            let already_positioned = hwnd_res.as_ref().ok().is_some_and(|&hwnd| unsafe {
                use windows::Win32::Foundation::RECT;
                use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
                let mut current_rect = RECT::default();
                GetWindowRect(hwnd, &mut current_rect).is_ok()
                    && current_rect.left == pos.x
                    && current_rect.top == pos.y
                    && (current_rect.right - current_rect.left) == size.width as i32
                    && (current_rect.bottom - current_rect.top) == size.height as i32
            });
            if !already_positioned {
                let _ = ov_win.set_position(tauri::PhysicalPosition::new(pos.x, pos.y));
                let _ = ov_win.set_size(tauri::PhysicalSize::new(size.width, size.height));
            }
        }
        if let Ok(hwnd) = hwnd_res { re_assert_topmost(hwnd); }
    }
}

/// Computes the vertical top coordinate for a notch window on a monitor.
/// When `fern-notch-respect-workarea` is disabled (default), returns `monitor_pos_y` unchanged.
/// When enabled, returns `rcWork.top.max(monitor_pos_y)` so the notch sits below top status bars
/// such as YASB or Zebar instead of overlapping them.
pub fn notch_target_top(monitor_pos_x: i32, monitor_pos_y: i32, monitor_w: i32, monitor_h: i32) -> i32 {
    if !crate::utils::setting_flag("fern-notch-respect-workarea") {
        return monitor_pos_y;
    }
    unsafe {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        let pt = POINT {
            x: monitor_pos_x + (monitor_w / 2),
            y: monitor_pos_y + (monitor_h / 2),
        };
        let h_monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(h_monitor, &mut mi).as_bool() {
            return mi.rcWork.top.max(monitor_pos_y);
        }
    }
    monitor_pos_y
}

/// Ensures a notch window has `WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`, is positioned at
/// `(m_pos.x, target_top, m_size.width, ph)` without activating it, and updates `main_window_rects`.
/// Returns `true` if a matching monitor was found and positioning was applied/verified.
pub fn position_notch_window(window: &tauri::WebviewWindow) -> bool {
    let label = window.label().to_string();
    let Some(monitor) = crate::monitors::monitor_for_window_label(
        window.app_handle(),
        crate::types::WindowKind::Notch,
        &label,
    ) else {
        return false;
    };
    let Ok(hwnd) = window.hwnd() else {
        return false;
    };
    let m_size = monitor.size();
    let m_pos = monitor.position();
    let scale = monitor.scale_factor();
    let fern_scale = crate::utils::get_fern_scale(window.app_handle());
    let ph = ((640.0 * fern_scale) * scale) as i32;
    let target_left = m_pos.x;
    let target_top = notch_target_top(m_pos.x, m_pos.y, m_size.width as i32, m_size.height as i32);
    let target_width = m_size.width as i32;

    unsafe {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongPtrW, GetWindowRect, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE,
            SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, WS_EX_NOACTIVATE as WS_EX_NA,
            WS_EX_TOOLWINDOW,
        };

        let mut ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as usize;
        ex_style |= (WS_EX_TOOLWINDOW.0 | WS_EX_NA.0) as usize;
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);

        let mut current_rect = RECT::default();
        let mut already_positioned = false;
        if GetWindowRect(hwnd, &mut current_rect).is_ok() {
            let current_width = current_rect.right - current_rect.left;
            let current_height = current_rect.bottom - current_rect.top;
            if current_rect.left == target_left
                && current_rect.top == target_top
                && current_width == target_width
                && current_height == ph
            {
                already_positioned = true;
            }
        }

        if !already_positioned {
            let _ = SetWindowPos(
                hwnd,
                None,
                target_left,
                target_top,
                target_width,
                ph,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
    }

    if let Ok(mut map) = crate::state::main_window_rects().lock() {
        map.insert(
            label,
            (
                tauri::PhysicalPosition::new(target_left, target_top),
                tauri::PhysicalSize::new(target_width as u32, ph.max(0) as u32),
            ),
        );
    }

    true
}

pub fn register_appbar(window: tauri::WebviewWindow) {
    let label = window.label().to_string();
    if label != "main" && label != "dock" && !crate::state::SPLASH_FINISHED.load(Ordering::Relaxed) {
        return;
    }
    if position_notch_window(&window) {
        crate::state::set_flag(crate::state::main_appbar_registered(), &label, true);
        if let Ok(hwnd) = window.hwnd() {
            // Re-assert topmost after repositioning — use re_assert_topmost instead of
            // set_always_on_top(true) to include SWP_NOACTIVATE and re-stamp WS_EX_NOACTIVATE.
            // to blank/hide when other windows were minimized or closed.
            re_assert_topmost(hwnd);
        }

        if !window.is_visible().unwrap_or(false) {
            let _ = window.show();
        }
    } else {
        let w = window.clone();
        tauri::async_runtime::spawn(async move {
            for _ in 0..10 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if crate::monitors::monitor_for_window_label(w.app_handle(), crate::types::WindowKind::Notch, w.label()).is_some() {
                    register_appbar(w);
                    break;
                }
            }
        });
    }
}

pub fn register_dock_appbar(window: tauri::WebviewWindow) {
    register_dock_appbar_inner(window, 0);
}

fn register_dock_appbar_inner(window: tauri::WebviewWindow, attempt: i32) {
    let label = window.label().to_string();
    if label != "main" && label != "dock" && !crate::state::SPLASH_FINISHED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(monitor) = crate::monitors::monitor_for_window_label(window.app_handle(), crate::types::WindowKind::Dock, &label) {
        let m_size = monitor.size();
        let m_pos = monitor.position();
        let hwnd = window.hwnd().unwrap();
        let scale = monitor.scale_factor();
        let fern_scale = crate::utils::get_fern_scale(window.app_handle());
        
        // ph = full physical window height. outer_size() can return 0 before the
        // window has rendered. Never guess a value — bail and let the retry wrapper handle it.
        let ph = window.outer_size().map(|s| s.height as i32).unwrap_or(0);
        if ph <= 0 {
            if attempt < 10 {
                let w = window.clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    register_dock_appbar_inner(w, attempt + 1);
                });
            }
            return;
        }

        let pr = ((56.0 * fern_scale) * scale) as i32;
        let reserve_space = crate::utils::setting_flag_default_true("fern-dock-reserve-space");

        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, GetWindowRect, SWP_NOZORDER, SWP_NOACTIVATE, GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_TOOLWINDOW, WS_EX_NOACTIVATE as WS_EX_NA, SWP_FRAMECHANGED};
            use windows::Win32::Foundation::RECT;
            use windows::Win32::UI::Shell::{SHAppBarMessage, APPBARDATA, ABM_NEW, ABM_QUERYPOS, ABM_SETPOS, ABE_BOTTOM};
            
            // Set extended styles (ToolWindow and NoActivate)
            let mut ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as usize;
            ex_style |= (WS_EX_TOOLWINDOW.0 | WS_EX_NA.0) as usize;
            let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style as isize);

            // Hide native taskbar first to free up space
            set_taskbar_visibility(false, false);

            let (final_left, final_y, final_width) = if reserve_space {
                let mut abd = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: hwnd, ..Default::default() };

                if !crate::state::get_flag(crate::state::dock_appbar_registered(), &label) {
                    SHAppBarMessage(ABM_NEW, &mut abd);
                    crate::state::set_flag(crate::state::dock_appbar_registered(), &label, true);
                }

                abd.uEdge = ABE_BOTTOM;
                abd.rc = RECT { 
                    left: m_pos.x, 
                    top: m_pos.y + m_size.height as i32 - pr, 
                    right: m_pos.x + m_size.width as i32, 
                    bottom: m_pos.y + m_size.height as i32 
                };
                
                SHAppBarMessage(ABM_QUERYPOS, &mut abd);
                SHAppBarMessage(ABM_SETPOS, &mut abd);
                
                // Critical: Force the window to the actual bottom of the screen, 
                // ignoring what ABM_SETPOS might have tried to "correct" (like stacking on invisible taskbar)
                (abd.rc.left, m_pos.y + m_size.height as i32 - ph, abd.rc.right - abd.rc.left)
            } else {
                if crate::state::get_flag(crate::state::dock_appbar_registered(), &label) {
                    unregister_appbar_native(hwnd);
                    crate::state::set_flag(crate::state::dock_appbar_registered(), &label, false);
                }
                (m_pos.x, m_pos.y + m_size.height as i32 - ph, m_size.width as i32)
            };
            
            let mut current_rect = RECT::default();
            let mut already_positioned = false;
            if GetWindowRect(hwnd, &mut current_rect).is_ok() {
                let current_width = current_rect.right - current_rect.left;
                let current_height = current_rect.bottom - current_rect.top;
                if current_rect.left == final_left 
                    && current_rect.top == final_y 
                    && current_width == final_width 
                    && current_height == ph 
                {
                    already_positioned = true;
                }
            }
            
            if !already_positioned {
                let _ = SetWindowPos(hwnd, None, final_left, final_y, final_width, ph, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
            }

            // Re-assert topmost — same reason as register_appbar.
            re_assert_topmost(hwnd);

            // Always ensure the window is visible — show() is idempotent
            if !window.is_visible().unwrap_or(false) {
                let _ = window.show();
            }
        }
    } else {
        let w = window.clone();
        tauri::async_runtime::spawn(async move {
            for _ in 0..10 {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if crate::monitors::monitor_for_window_label(w.app_handle(), crate::types::WindowKind::Dock, w.label()).is_some() {
                    register_dock_appbar(w);
                    break;
                }
            }
        });
    }
}

/// True for a property sheet dialog (has a tab control). Works in any locale, unlike matching the title.
unsafe fn dialog_has_tab_control(hwnd: HWND) -> bool {
    unsafe extern "system" fn child_proc(child: HWND, lparam: LPARAM) -> BOOL {
        let found = &mut *(lparam.0 as *mut bool);
        let mut class_name = [0u8; 64];
        let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameA(child, &mut class_name);
        if std::str::from_utf8(&class_name[..len as usize]).unwrap_or("") == "SysTabControl32" {
            *found = true;
            return BOOL(0);
        }
        BOOL(1)
    }

    let mut found = false;
    let _ = windows::Win32::UI::WindowsAndMessaging::EnumChildWindows(
        Some(hwnd),
        Some(child_proc),
        LPARAM(&mut found as *mut bool as isize),
    );
    found
}

// "Fern" is an ordinary word, so a window is only ours if our process owns it. Other apps' windows
// with the same title, or a program of the same name, stay in the dock.
fn hides_own_window(window_pid: u32, own_pid: u32, title: &str) -> bool {
    if window_pid != own_pid {
        return false;
    }
    let lower = title.to_lowercase();
    let is_settings = lower.contains("settings") || lower.contains("ayarlar") || title == "Settings";
    !is_settings
}

fn should_include_window_title(title_len: i32, window_class: &str, console_windows_enabled: bool) -> bool {
    title_len > 0 || (window_class == "ConsoleWindowClass" && console_windows_enabled)
}

pub unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let apps = &mut *(lparam.0 as *mut Vec<AppInfo>);

    if IsWindowVisible(hwnd).as_bool() {
        // DWM-cloaked windows aren't on screen (suspended UWP apps, other virtual desktops)
        // but IsWindowVisible is still true, so check cloaking.
        {
            let mut cloaked = 0u32;
            let size = std::mem::size_of::<u32>() as u32;
            if windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                hwnd,
                windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut _,
                size,
            ).is_ok() && cloaked != 0 {
                return true.into();
            }
        }

        let mut text = [0u16; 512];
        let len = windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut text);
        let mut class_name = [0u8; 256];
        let class_len = windows::Win32::UI::WindowsAndMessaging::GetClassNameA(hwnd, &mut class_name);
        let window_class = std::str::from_utf8(&class_name[..class_len.max(0) as usize]).unwrap_or("");
        let console_windows_enabled = crate::utils::setting_flag("fern-dock-console-windows");
        if should_include_window_title(len, window_class, console_windows_enabled) {
            let title = if len > 0 { String::from_utf16_lossy(&text[..len as usize]) } else { String::new() };
            
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            
            // Basic filter for top-level app windows
            // We include windows without captions if they don't have the ToolWindow style,
            // as many games and modern apps (like Spotify/Valorant) lack WS_CAPTION.
            if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
                return true.into();
            }

            // Filter out system containers and background stuff
            if title == "Program Manager" {
                return true.into();
            }

            let mut process_id = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut process_id));
            
            if let Ok(process_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) {
                let mut path_buf = [0u16; 1024];
                let mut path_len = path_buf.len() as u32;
                if QueryFullProcessImageNameW(process_handle, PROCESS_NAME_WIN32, windows::core::PWSTR(path_buf.as_mut_ptr()), &mut path_len).is_ok() {
                    let path = String::from_utf16_lossy(&path_buf[..path_len as usize]);
                    let lowercase_path = path.to_lowercase();
                    let is_console_window = window_class == "ConsoleWindowClass";

                    // Explorer folder windows and property sheets (#32770 with a tab control) both count,
                    // so Properties shows under File Explorer.
                    let is_explorer_window = window_class == "CabinetWClass"
                        || window_class == "ExploreWClass"
                        || (window_class == "#32770" && dialog_has_tab_control(hwnd));

                    let has_visible_owner = windows::Win32::UI::WindowsAndMessaging::GetWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::GW_OWNER)
                        .map(|owner| !owner.0.is_null() && IsWindowVisible(owner).as_bool())
                        .unwrap_or(false);
                    if has_visible_owner && !is_explorer_window && !is_console_window {
                        let _ = CloseHandle(process_handle);
                        return true.into();
                    }

                    if hides_own_window(process_id, std::process::id(), &title) ||
                       (lowercase_path.contains("conhost.exe")
                           && !(is_console_window && console_windows_enabled)) ||
                       (lowercase_path.contains("explorer.exe") && !is_explorer_window) ||
                       lowercase_path.contains("shellexperiencehost.exe") ||
                       lowercase_path.contains("searchhost.exe") ||
                       lowercase_path.contains("textinputhost.exe") ||
                       lowercase_path.ends_with("gamingservicesui.exe") ||
                       (lowercase_path.contains("applicationframehost.exe") && window_class != "ApplicationFrameWindow") {
                        let _ = CloseHandle(process_handle);
                        return true.into();
                    }

                    let name = Path::new(&path).file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&title)
                        .replace(".exe", "");

                    let exe_name = Path::new(&path).file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string());

                    // A UWP app's real window is the ApplicationFrameWindow from ApplicationFrameHost, with the
                    // title and package id. Skip the inner CoreWindow so there's no duplicate item.
                    let is_uwp_core_window = window_class == "Windows.UI.Core.CoreWindow";
                    let is_uwp_frame = window_class == "ApplicationFrameWindow";

                    let is_browser_host = lowercase_path.contains("msedge.exe")
                        || lowercase_path.contains("chrome.exe")
                        || lowercase_path.contains("brave.exe");

                    // A browser window and a PWA in the same browser can share a title. The AUMID tells them apart:
                    // PWAs carry a web app id or package id, plain browser windows just the browser.
                    // No id falls back to the title.
                    let window_aumid = if is_uwp_frame
                        || lowercase_path.contains("\\windowsapps\\")
                        || is_browser_host
                    {
                        crate::utils::get_window_app_user_model_id(hwnd)
                    } else {
                        None
                    };
                    let is_browser_pwa = is_browser_host
                        && window_aumid.as_deref().map_or(false, crate::commands::is_browser_pwa_aumid);

                    let final_name = if name.eq_ignore_ascii_case("conhost") {
                        if !title.trim().is_empty() { title.clone() } else { "Console".to_string() }
                    } else if (name.eq_ignore_ascii_case("javaw") || name.eq_ignore_ascii_case("java")) && !title.is_empty() {
                        title.clone()
                    } else if ((is_browser_host && (is_browser_pwa || window_aumid.is_none()))
                        || name == "ApplicationFrameHost"
                        || name == "SystemSettings")
                        && !title.is_empty()
                    {
                        // Extract a cleaner name from the window title for host processes (PWAs, UWP apps)
                        title.split(" - ").next().map(|s| s.trim()).unwrap_or(&title).to_string()
                    } else if name.eq_ignore_ascii_case("explorer") {
                        crate::taskbar_pins::shell_app_name("Microsoft.Windows.Explorer", "File Explorer")
                    } else {
                        let from_start_menu = INSTALLED_APPS_CACHE
                            .get()
                            .and_then(|c| c.lock().ok())
                            .and_then(|apps| {
                                apps.iter().find(|a| a.path.eq_ignore_ascii_case(&path)).map(|a| a.name.clone())
                            })
                            .or_else(|| {
                                let norm = normalize_exe_path(&path);
                                RESOLVED_TARGET_NAMES
                                    .get()
                                    .and_then(|m| m.read().ok())
                                    .and_then(|m| m.get(&norm).cloned())
                            });
                        from_start_menu
                            .or_else(|| crate::utils::exe_description(&path))
                            .unwrap_or_else(|| name.clone())
                    };

                    // Only avoid adding the exact same window handle (HWND) multiple times
                    let already_exists = apps.iter().any(|a| a.hwnd == Some(hwnd.0 as isize));

                    if !already_exists && !is_uwp_core_window {
                        // Packaged apps expose their package id on the window. Using it as the path makes pins
                        // match running windows and gives the icon code an exact AUMID.
                        let path = if is_uwp_frame
                            || lowercase_path.contains("\\windowsapps\\")
                            || is_browser_host
                        {
                            match window_aumid {
                                Some(aumid) if aumid.contains('!') => aumid,
                                _ => path,
                            }
                        } else {
                            path
                        };
                        let is_control_panel = window_class == "CabinetWClass"
                            && lowercase_path.ends_with("\\explorer.exe")
                            && crate::utils::get_window_app_user_model_id(hwnd)
                                .map_or(false, |id| id.eq_ignore_ascii_case("Microsoft.Windows.ControlPanel"));
                        let (final_name, path, exe_name) = if process_id == std::process::id() {
                            ("Settings".to_string(), "fern-settings".to_string(), None)
                        } else if is_control_panel {
                            let control = format!(r"{}\System32\control.exe", std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()));
                            (crate::taskbar_pins::shell_app_name("Microsoft.Windows.ControlPanel", "Control Panel"), control, Some("control.exe".to_string()))
                        } else {
                            (final_name, path, exe_name)
                        };
                        let window_title = if title.trim().is_empty() { final_name.clone() } else { title.clone() };
                        apps.push(AppInfo {
                            name: final_name,
                            path,
                            icon: None,
                            is_running: true,
                            hwnd: Some(hwnd.0 as isize),
                            executable: exe_name,
                            all_hwnds: Some(vec![(hwnd.0 as isize, window_title)]),
                            args: None,
                        });
                    }
                }
                let _ = CloseHandle(process_handle);
            } else if process_exe_name(process_id).is_some_and(|exe| exe.eq_ignore_ascii_case("consent.exe")) {
                // Parked UAC prompt (consent.exe runs as SYSTEM so OpenProcess fails).
                if !apps.iter().any(|a| a.hwnd == Some(hwnd.0 as isize)) {
                    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
                    let window_title = if title.trim().is_empty() { "User Account Control".to_string() } else { title.clone() };
                    apps.push(AppInfo {
                        name: window_title.clone(),
                        path: format!(r"{system_root}\System32\consent.exe"),
                        icon: None,
                        is_running: true,
                        hwnd: Some(hwnd.0 as isize),
                        executable: Some("consent.exe".into()),
                        all_hwnds: Some(vec![(hwnd.0 as isize, window_title)]),
                        args: None,
                    });
                }
            }
        }
    }
    true.into()
}

/// Returns the executable file name of a process (including elevated SYSTEM processes
/// that cannot be opened with `OpenProcess`) via `CreateToolhelp32Snapshot`.
unsafe fn process_exe_name(process_id: u32) -> Option<String> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = None;
    if Process32FirstW(snapshot, &mut entry).is_ok() {
        loop {
            if entry.th32ProcessID == process_id {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                found = Some(String::from_utf16_lossy(&entry.szExeFile[..len]));
                break;
            }
            if Process32NextW(snapshot, &mut entry).is_err() {
                break;
            }
        }
    }
    let _ = CloseHandle(snapshot);
    found
}

pub fn unregister_appbar_native(hwnd: HWND) {
    unsafe {
        use windows::Win32::UI::Shell::{SHAppBarMessage, APPBARDATA, ABM_REMOVE};
        let mut abd = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: hwnd, ..Default::default() };
        SHAppBarMessage(ABM_REMOVE, &mut abd);
    }
}

pub fn wire_window_events(app: &AppHandle, window: &tauri::WebviewWindow, kind: crate::types::WindowKind, is_dynamic: bool) {
    let label = window.label().to_string();

    if let Ok(hwnd) = window.hwnd() {
        crate::active_look::prevent_minimize(hwnd);
    }

    let win_for_rect = window.clone();
    let label_for_rect = label.clone();
    let update_rect = move || {
        if let (Ok(p), Ok(s)) = (win_for_rect.outer_position(), win_for_rect.outer_size()) {
            let map = match kind {
                crate::types::WindowKind::Dock => crate::state::dock_window_rects(),
                crate::types::WindowKind::Notch => crate::state::main_window_rects(),
            };
            if let Ok(mut m) = map.lock() {
                m.insert(label_for_rect.clone(), (p, s));
            }
        }
    };
    update_rect();

    let u = update_rect.clone();
    let win_for_events = window.clone();
    let app_for_events = app.clone();
    let label_for_events = label.clone();
    window.on_window_event(move |e| {
        match e {
            tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                u();
                sync_overlays(&app_for_events);
            }
            tauri::WindowEvent::ScaleFactorChanged { .. } => {
                u();
                match kind {
                    crate::types::WindowKind::Notch => {
                        let w = win_for_events.clone();
                        let h = app_for_events.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            register_appbar(w);
                            sync_overlays(&h);
                        });
                    }
                    crate::types::WindowKind::Dock => {
                        let h = app_for_events.clone();
                        if crate::state::get_flag(crate::state::dock_appbar_registered(), &label_for_events) {
                            let w = win_for_events.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                register_dock_appbar(w);
                                sync_overlays(&h);
                            });
                        } else {
                            sync_overlays(&h);
                        }
                    }
                }
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                if is_dynamic {
                    let _ = win_for_events.hide();
                } else {
                    crate::commands::restore_taskbar_and_exit(&app_for_events);
                }
            }
            _ => {}
        }
    });
}

pub fn create_monitor_window(app: &AppHandle, kind: crate::types::WindowKind, label: &str, monitor: &tauri::Monitor) {
    let app = app.clone();
    let label = label.to_string();
    let monitor = monitor.clone();
    let base_label = match kind {
        crate::types::WindowKind::Dock => "dock",
        crate::types::WindowKind::Notch => "main",
    };

    tauri::async_runtime::spawn(async move {
        let base_config = match app.config().app.windows.iter().find(|c| c.label == base_label) {
            Some(c) => c.clone(),
            None => return,
        };

        let mut conf = base_config;
        conf.label = label.clone();

        let pos = monitor.position();
        let scale = monitor.scale_factor();
        conf.x = Some(pos.x as f64 / scale);
        conf.y = Some(pos.y as f64 / scale);
        conf.visible = false;

        let builder = match tauri::WebviewWindowBuilder::from_config(&app, &conf) {
            Ok(b) => b,
            Err(_) => return,
        };
        let window = match builder.build() {
            Ok(w) => w,
            Err(_) => return,
        };

        let _ = window.set_ignore_cursor_events(true);
        wire_window_events(&app, &window, kind, true);
        watch_webview_process(&app, &window);

        // Extra monitors wait for the splash
        if !crate::state::SPLASH_FINISHED.load(Ordering::Relaxed) {
            crate::commands::wait_for_splash().await;
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }

        match kind {
            crate::types::WindowKind::Dock => {
                let mode = crate::utils::get_setting_str(&app, "fern-dock-mode").unwrap_or_else(|| "smart".to_string());
                crate::commands::init_dock_window(&app, window, &mode).await;
            }
            crate::types::WindowKind::Notch => {
                let mode = crate::utils::get_setting_str(&app, "fern-notch-mode").unwrap_or_else(|| "fixed".to_string());
                crate::commands::init_notch_window(&app, window, &mode).await;
            }
        }
    });
}

fn reposition_all_windows(app_handle: &AppHandle) {
    if SHUTTING_DOWN.load(Ordering::Relaxed) {
        return;
    }
    crate::monitors::sync_monitor_windows(app_handle);

    for (label, win) in app_handle.webview_windows() {
        if crate::state::is_notch_label(&label) {
            if crate::state::get_flag(crate::state::main_appbar_registered(), &label) {
                register_appbar(win);
            } else if crate::utils::setting_flag("fern-notch-respect-workarea") {
                position_notch_window(&win);
            }
        } else if crate::state::is_dock_label(&label) {
            // Only reposition the dock if it's enabled in settings.
            // Without this guard, power events (plug/unplug, wake) would re-show
            // a dock that the user had previously disabled.
            let dock_enabled = get_setting_str(app_handle, "fern-dock-enabled")
                .unwrap_or_else(|| "true".to_string());
            let waiting_for_splash = !crate::state::SPLASH_FINISHED.load(Ordering::Relaxed);
            if dock_enabled == "true" && !waiting_for_splash {
                if crate::state::get_flag(crate::state::dock_appbar_registered(), &label) {
                    register_dock_appbar(win);
                } else {
                    reposition_autohide_dock(app_handle, win);
                }
            }
        }
    }
    if NATIVE_TASKBAR_HIDDEN.load(Ordering::Relaxed) {
        set_taskbar_visibility(false, false);
    }
    sync_overlays(app_handle);
    // Re-assert topmost on all windows after repositioning to recover from any z-order loss
    for (label, win) in app_handle.webview_windows() {
        if crate::state::is_notch_label(&label) || crate::state::is_dock_label(&label) {
            if let Ok(hwnd) = win.hwnd() { re_assert_topmost(hwnd); }
        }
    }
}

fn reposition_autohide_dock(app_handle: &AppHandle, dock_win: tauri::WebviewWindow) {
    let dock_clone = dock_win.clone();
    let ah = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        for _attempt in 0..5 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let hwnd_val = match dock_clone.hwnd() {
                Ok(h) => h.0 as isize,
                Err(_) => continue,
            };
            let ph = match dock_clone.outer_size() {
                Ok(s) => s.height as i32,
                Err(_) => continue,
            };
            if ph <= 10 {
                continue;
            }
            let monitor_info = crate::monitors::monitor_for_window_label(dock_clone.app_handle(), crate::types::WindowKind::Dock, dock_clone.label()).map(|m| {
                let s = m.size();
                let p = m.position();
                (s.width as i32, s.height as i32, p.x, p.y)
            });
            if let Some((m_w, m_h, m_x, m_y)) = monitor_info {
                let final_y = m_y + m_h - ph;
                unsafe {
                    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOZORDER, SWP_NOACTIVATE, SWP_FRAMECHANGED};
                    use windows::Win32::Foundation::HWND;
                    let _ = SetWindowPos(
                        HWND(hwnd_val as *mut _), None,
                        m_x, final_y, m_w, ph,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    );
                }
                if let Ok(hwnd) = dock_clone.hwnd() { re_assert_topmost(hwnd); }
                let _ = dock_clone.show();
                break;
            }
        }
        if NATIVE_TASKBAR_HIDDEN.load(Ordering::Relaxed) {
            set_taskbar_visibility(false, false);
        }
        sync_overlays(&ah);
    });
}

pub fn setup_display_change_monitor(app_handle: AppHandle) {
    let _ = crate::state::DISPLAY_MONITOR_HANDLE.set(app_handle);

    std::thread::spawn(|| {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::*;
            use windows::Win32::System::LibraryLoader::GetModuleHandleW;

            let class_name = windows::core::PCSTR(c"FernDisplayMonitor".as_ptr() as *const u8);
            let h_inst = GetModuleHandleW(None).unwrap_or_default().into();

            let wnd_class = WNDCLASSEXA {
                cbSize: std::mem::size_of::<WNDCLASSEXA>() as u32,
                lpfnWndProc: Some(display_monitor_proc),
                hInstance: h_inst,
                lpszClassName: class_name,
                ..Default::default()
            };

            if RegisterClassExA(&wnd_class) == 0 {
                return;
            }

            let hwnd = CreateWindowExA(
                WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0),
                class_name,
                windows::core::PCSTR::null(),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(h_inst),
                None,
            );

            let hwnd = match hwnd {
                Ok(h) => h,
                Err(_) => return,
            };

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, Some(hwnd), 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
}

unsafe extern "system" fn display_monitor_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::LRESULT;

    const WM_DISPLAYCHANGE: u32 = 0x007E;
    const WM_POWERBROADCAST: u32 = 0x0218;
    const PBT_APMPOWERSTATUSCHANGE: u32 = 0x000A;
    const PBT_APMRESUMESUSPEND: u32 = 0x0007;
    const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
    const WM_SETTINGCHANGE: u32 = 0x001A;

    match msg {
        WM_DISPLAYCHANGE => {
            let now = get_now_ms();
            let last = LAST_DISPLAY_CHANGE_MS.load(Ordering::Relaxed);
            if now - last > 500 {
                LAST_DISPLAY_CHANGE_MS.store(now, Ordering::Relaxed);
                if let Some(app_handle) = DISPLAY_MONITOR_HANDLE.get() {
                    let ah = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        reposition_all_windows(&ah);
                    });
                }
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            let pw = wparam.0 as u32;
            if pw == PBT_APMPOWERSTATUSCHANGE || pw == PBT_APMRESUMESUSPEND {
                let now = get_now_ms();
                let last = LAST_DISPLAY_CHANGE_MS.load(Ordering::Relaxed);
                if now - last > 1000 {
                    LAST_DISPLAY_CHANGE_MS.store(now, Ordering::Relaxed);
                    if let Some(app_handle) = DISPLAY_MONITOR_HANDLE.get() {
                        let ah = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            // Longer delay for wake from sleep to allow display to fully initialize
                            let delay = if pw == PBT_APMRESUMESUSPEND { 500 } else { 300 };
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            reposition_all_windows(&ah);
                        });
                    }
                }
            }
            LRESULT(0)
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => {
            if !crate::windows_accent::is_recent_self_broadcast() {
                emit_system_accent_change();
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SETTINGCHANGE => {
            const SPI_SETWORKAREA: usize = 0x002F;
            static LAST_WORKAREA_CHANGE_MS: AtomicI64 = AtomicI64::new(0);
            if wparam.0 == SPI_SETWORKAREA && crate::utils::setting_flag("fern-notch-respect-workarea") {
                let now = get_now_ms();
                let last = LAST_WORKAREA_CHANGE_MS.load(Ordering::Relaxed);
                if now - last > 300 {
                    LAST_WORKAREA_CHANGE_MS.store(now, Ordering::Relaxed);
                    if let Some(app_handle) = DISPLAY_MONITOR_HANDLE.get() {
                        let ah = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                            reposition_all_windows(&ah);
                        });
                    }
                }
            }
            let area = if lparam.0 != 0 {
                unsafe { windows::core::PCWSTR(lparam.0 as *const u16).to_string() }.unwrap_or_default()
            } else {
                String::new()
            };
            if should_emit_setting_change(wparam.0, &area, crate::windows_accent::is_recent_self_broadcast()) {
                emit_system_accent_change();
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn rect_overlaps_dock(
    monitor_left: i32,
    monitor_bottom: i32,
    fg_rect: windows::Win32::Foundation::RECT,
    dr: crate::types::IntRect,
    scale: f64,
) -> bool {
    let d_left = monitor_left + (dr.x as f64 * scale) as i32;
    let d_right = d_left + (dr.width as f64 * scale) as i32;
    let trigger_y = monitor_bottom - (56.0 * scale) as i32;
    fg_rect.left < d_right - 4 && fg_rect.right > d_left + 4 && fg_rect.bottom > trigger_y + 4
}

fn rect_overlaps_notch(
    monitor_left: i32,
    monitor_top: i32,
    fg_rect: windows::Win32::Foundation::RECT,
    nr: crate::types::IntRect,
    scale: f64,
) -> bool {
    if nr.width <= 0 || nr.height <= 0 {
        return false;
    }
    let n_left = monitor_left + (nr.x as f64 * scale) as i32;
    let n_right = n_left + (nr.width as f64 * scale) as i32;
    let trigger_y = monitor_top + ((nr.y.max(0) + 36) as f64 * scale) as i32;
    fg_rect.left < n_right - 4 && fg_rect.right > n_left + 4 && fg_rect.top < trigger_y - 4
}

fn should_emit_setting_change(wparam: usize, area: &str, recent_self_broadcast: bool) -> bool {
    const SPI_SETDESKWALLPAPER: usize = 0x0014;
    if wparam == SPI_SETDESKWALLPAPER {
        return true;
    }
    match area {
        "ImmersiveColorSet" => !recent_self_broadcast,
        "WindowsThemeElement" | "Wallpaper" => true,
        _ => false,
    }
}

fn emit_system_accent_change() {
    if let Some(app_handle) = DISPLAY_MONITOR_HANDLE.get() {
        let color_hex = if let Some(c) = crate::commands::get_windows_accent_color() {
            c
        } else {
            let mut color = 0u32;
            let mut opaque = windows::core::BOOL(0);
            if unsafe { windows::Win32::Graphics::Dwm::DwmGetColorizationColor(&mut color, &mut opaque).is_ok() } {
                let r = ((color >> 16) & 0xff) as u8;
                let g = ((color >> 8) & 0xff) as u8;
                let b = (color & 0xff) as u8;
                format!("#{:02x}{:02x}{:02x}", r, g, b)
            } else {
                "#0078d4".to_string()
            }
        };
        let _ = app_handle.emit("system-accent-changed", color_hex);
    }
}

pub fn start_window_style_guard(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(crate::memory_saver::poll_interval(std::time::Duration::from_millis(300)));
        for (label, win) in app.webview_windows() {
            let is_notch = is_notch_label(&label);
            let is_dock = is_dock_label(&label);
            if !(is_notch || is_dock || label == "overlay") { continue; }
            let Ok(hwnd) = win.hwnd() else { continue };
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{
                    GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
                };
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as usize;
                let keyboard = crate::state::KEYBOARD_FOCUS_LABEL.lock().ok().is_some_and(|l| l.as_deref() == Some(label.as_str()));
                let want = if keyboard { WS_EX_TOOLWINDOW.0 as usize } else { (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) as usize };
                if ex & want != want {
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex | want) as isize);
                }

                let extra = crate::state::monitor_suffix(&label).is_some();
                let last = if label == "overlay" {
                    MH_LAST_OV_IGNORE.load(Ordering::Relaxed)
                } else if is_notch && !extra {
                    MH_LAST_MAIN_IGNORE.load(Ordering::Relaxed)
                } else if is_dock && !extra {
                    MH_LAST_DOCK_IGNORE.load(Ordering::Relaxed)
                } else if is_notch {
                    crate::state::get_i32(crate::state::notch_extra_last_ignore(), &label, -1)
                } else {
                    crate::state::get_i32(crate::state::dock_extra_last_ignore(), &label, -1)
                };
                // tao rewrites the extended style on its own flag changes. Restore what the hit test
                // last decided (see set_click_through), the cursor may not move again to trigger it.
                if last != -1 {
                    crate::utils::set_click_through(hwnd, last == 1);
                } else if ex & WS_EX_LAYERED.0 as usize == 0 {
                    crate::utils::set_click_through(hwnd, ex & WS_EX_TRANSPARENT.0 as usize != 0);
                }
            }
        }
    });
}

static WEBVIEWS_WATCHED_AT: OnceLock<Instant> = OnceLock::new();
static RECOVERING_FROM_WEBVIEW_FAILURE: AtomicBool = AtomicBool::new(false);

/// If a webview's browser or renderer process dies, the windows stay up with dead content and keep
/// swallowing clicks. Relaunch when that happens.
pub fn watch_webview_processes(app: &AppHandle) {
    let _ = WEBVIEWS_WATCHED_AT.set(Instant::now());
    for (_, window) in app.webview_windows() {
        watch_webview_process(app, &window);
    }
}

/// Also called for each on-demand window, those don't exist yet when the app starts.
pub fn watch_webview_process(app: &AppHandle, window: &tauri::WebviewWindow) {
    let handle = app.clone();
    let _ = window.with_webview(move |webview| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_PROCESS_FAILED_KIND,
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        };
        use webview2_com::ProcessFailedEventHandler;

        let Ok(core) = webview.controller().CoreWebView2() else {
            return;
        };
        let mut token = 0i64;
        let _ = core.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                args.ProcessFailedKind(&mut kind)?;
                crate::diagnostics::log(&format!("webview process failed: kind={}", kind.0));
                if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
                    || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                {
                    recover_from_webview_failure(&handle);
                }
                Ok(())
            })),
            &mut token,
        );
    });
}

fn recover_from_webview_failure(handle: &AppHandle) {
    // Quitting destroys the webviews too, and every window reports the same exit, so only act once.
    if SHUTTING_DOWN.load(Ordering::Relaxed)
        || RECOVERING_FROM_WEBVIEW_FAILURE.swap(true, Ordering::SeqCst)
    {
        return;
    }
    let started = WEBVIEWS_WATCHED_AT.get().map(Instant::elapsed);
    let handle = handle.clone();
    // Get out of the WebView2 callback before touching windows.
    tauri::async_runtime::spawn(async move {
        // A webview dying right after launch would die again after a relaunch, so give the taskbar back and exit.
        if started.is_some_and(|elapsed| elapsed < Duration::from_secs(30)) {
            crate::commands::restore_taskbar_and_exit(&handle);
        } else {
            crate::diagnostics::log("webview process failed, relaunching");
            // Give the leftover webview processes a moment to let go of the profile, relaunching
            // straight away left the new webviews without their images.
            SHUTTING_DOWN.store(true, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(1500)).await;
            crate::commands::relaunch_fern(&handle);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{cursor_watch_interval, dock_edge_hover_allowed, dock_hover_went_stale, edge_dwell_met, edge_dwell_waiting, notch_overlap_during_start, start_counts_as_shown, hides_own_window, hover_waits_on_expiry, is_helper_name, is_launchable_entry, win_number_index};
    use std::sync::atomic::AtomicI64;
    use std::time::Duration;

    #[test]
    fn start_leaves_the_notch_as_it_was() {
        assert!(notch_overlap_during_start(true, false, true), "hidden stays hidden");
        assert!(!notch_overlap_during_start(true, true, false), "visible stays visible");
        assert!(notch_overlap_during_start(false, true, false), "without Start the computed value counts");
        assert!(!notch_overlap_during_start(false, false, true));
    }

    #[test]
    fn start_counts_while_opening_but_not_forever() {
        let just_now = Duration::from_millis(100);
        let long_ago = Duration::from_millis(1000);
        assert!(start_counts_as_shown(true, true, long_ago), "on screen");
        assert!(start_counts_as_shown(true, false, just_now), "focused but still appearing");
        assert!(!start_counts_as_shown(true, false, long_ago), "focused but closed, the dock must come back");
        assert!(!start_counts_as_shown(false, true, just_now), "not Start");
    }

    #[test]
    fn hover_goes_stale_only_once_the_window_stopped_taking_input() {
        assert!(dock_hover_went_stale(true, true));
        assert!(!dock_hover_went_stale(false, true), "still taking input, the webview sees the leave itself");
        assert!(!dock_hover_went_stale(true, false));
    }

    #[test]
    fn a_hover_kept_only_by_its_expiry_needs_a_recheck() {
        assert!(hover_waits_on_expiry(false, true));
        assert!(!hover_waits_on_expiry(true, true), "the cursor is really there");
        assert!(!hover_waits_on_expiry(false, false));
    }

    #[test]
    fn cursor_watch_takes_over_after_the_hook_misses_a_move() {
        assert_eq!(cursor_watch_interval(None), Duration::from_millis(200));
        assert_eq!(cursor_watch_interval(Some(Duration::from_millis(999))), Duration::from_millis(32));
        assert_eq!(cursor_watch_interval(Some(Duration::from_millis(1000))), Duration::from_millis(200));
    }

    #[test]
    fn only_our_own_process_is_hidden_from_the_dock() {
        assert!(hides_own_window(10, 10, "fern"));
        assert!(hides_own_window(10, 10, "Fern Dock"));
        assert!(!hides_own_window(10, 10, "Settings"), "our Settings window shows in the dock");
        assert!(!hides_own_window(10, 10, "Fern Settings"), "Fern Settings shows in the dock");
        assert!(!hides_own_window(10, 10, "Ayarlar"), "Ayarlar shows in the dock");
        assert!(!hides_own_window(20, 10, "fern"), "another app's window with that title stays");
        assert!(!hides_own_window(20, 10, "Fern Dock"));
    }

    #[test]
    fn fullscreen_dock_does_not_reveal_on_edge_or_hover() {
        assert!(!dock_edge_hover_allowed(true, true, false));
        assert!(!dock_edge_hover_allowed(true, false, true));
        assert!(!dock_edge_hover_allowed(true, true, true));
    }

    #[test]
    fn ordinary_dock_preserves_edge_and_hover_reveal() {
        assert!(dock_edge_hover_allowed(false, true, false));
        assert!(dock_edge_hover_allowed(false, false, true));
        assert!(!dock_edge_hover_allowed(false, false, false));
    }

    #[test]
    fn win_number_maps_top_row_digits_only() {
        assert_eq!(win_number_index(0x31), Some(0));
        assert_eq!(win_number_index(0x35), Some(4));
        assert_eq!(win_number_index(0x39), Some(8));
        // 0, letters and numpad digits are not dock slots
        assert_eq!(win_number_index(0x30), None);
        assert_eq!(win_number_index(0x41), None);
        assert_eq!(win_number_index(0x61), None);
    }

    #[test]
    fn helper_shortcuts_are_not_launchable() {
        assert!(is_helper_name("Uninstall Brave"));
        assert!(is_helper_name("7-Zip Help"));
        assert!(is_helper_name("Readme"));
        assert!(is_helper_name("Get Help"));
        assert!(is_helper_name("Desktop Overlay Host"));
        assert!(is_helper_name("Debuggable Package Manager"));
        assert!(is_helper_name("Developer Command Prompt for VS 2022"));
        assert!(is_helper_name("Developer PowerShell for VS 2022"));
        // Whole-word matching: legit names containing helper words survive.
        assert!(!is_helper_name("Helper"));
        assert!(!is_helper_name("Photo Remover"));
        assert!(!is_helper_name("7-Zip File Manager"));
        assert!(!is_helper_name("Brave"));
        assert!(!is_helper_name("Blender 4.5"));
        assert!(!is_helper_name("Blockbench"));
        assert!(!is_helper_name("Denetim Masası"));
        assert!(!is_helper_name("Deskio"));
        assert!(!is_launchable_entry("Docs", "https://example.com"));
        assert!(is_launchable_entry("Brave", r"C:\Program Files\Brave\brave.exe"));
    }

    #[test]
    fn audio_frame_decoder_handles_ieee_float_and_pcm() {
        use super::decode_audio_frame;
        // Stereo 32-bit IEEE float (+0.5, -0.25) -> avg 0.125
        let mut float_frame = Vec::new();
        float_frame.extend_from_slice(&0.5f32.to_le_bytes());
        float_frame.extend_from_slice(&(-0.25f32).to_le_bytes());
        let val = decode_audio_frame(&float_frame, 2, 4, true);
        assert!((val - 0.125).abs() < 1e-6);

        // NaN float sample should clamp cleanly to 0.0
        let nan_frame = f32::NAN.to_le_bytes();
        assert_eq!(decode_audio_frame(&nan_frame, 1, 4, true), 0.0);

        // Mono 16-bit PCM half-scale negative (-16384 -> -0.5)
        let pcm16 = (-16384i16).to_le_bytes();
        assert!((decode_audio_frame(&pcm16, 1, 2, false) - (-0.5)).abs() < 1e-6);

        // Mono 32-bit PCM half-scale positive
        let pcm32 = (i32::MAX / 2).to_le_bytes();
        assert!((decode_audio_frame(&pcm32, 1, 4, false) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn primary_overlap_helpers_account_for_monitor_origin() {
        use super::{rect_overlaps_dock, rect_overlaps_notch};
        use crate::types::IntRect;
        use windows::Win32::Foundation::RECT;

        let dock_rect = IntRect { x: 600, y: 0, width: 720, height: 60 };
        // Specific monitor placed at x=1920..3840
        let fg_on_right = RECT { left: 2600, top: 800, right: 3100, bottom: 1060 };
        assert!(rect_overlaps_dock(1920, 1080, fg_on_right, dock_rect, 1.0));
        // Window only on primary monitor (0..1920) must not overlap dock on monitor at x=1920
        let fg_on_primary = RECT { left: 600, top: 800, right: 1300, bottom: 1060 };
        assert!(!rect_overlaps_dock(1920, 1080, fg_on_primary, dock_rect, 1.0));

        let notch_rect = IntRect { x: 760, y: 0, width: 400, height: 36 };
        let fg_top_right = RECT { left: 2700, top: 10, right: 2900, bottom: 500 };
        assert!(rect_overlaps_notch(1920, 0, fg_top_right, notch_rect, 1.0));
        assert!(!rect_overlaps_notch(1920, 0, fg_on_primary, notch_rect, 1.0));
    }

    #[test]
    fn setting_change_filter_suppresses_tray_echo_and_unrelated_areas() {
        use super::should_emit_setting_change;

        assert!(!should_emit_setting_change(0, "TraySettings", false));
        assert!(!should_emit_setting_change(0, "intl", false));
        assert!(!should_emit_setting_change(0, "", false));
        assert!(!should_emit_setting_change(0, "ImmersiveColorSet", true));
        assert!(should_emit_setting_change(0, "ImmersiveColorSet", false));
        assert!(should_emit_setting_change(0, "WindowsThemeElement", false));
        assert!(should_emit_setting_change(0x0014, "Wallpaper", true));
    }

    #[test]
    fn edge_area_allows_x_respects_narrow_mode_and_pad() {
        use super::edge_area_allows_x;

        assert!(edge_area_allows_x(false, Some((700, 1220)), 0, 60));
        assert!(edge_area_allows_x(false, Some((700, 1220)), 1900, 60));
        assert!(edge_area_allows_x(true, Some((700, 1220)), 640, 60));
        assert!(edge_area_allows_x(true, Some((700, 1220)), 1280, 60));
        assert!(!edge_area_allows_x(true, Some((700, 1220)), 639, 60));
        assert!(!edge_area_allows_x(true, Some((700, 1220)), 1281, 60));
        assert!(edge_area_allows_x(true, None, 0, 60));
    }

    #[test]
    fn smtc_artwork_retry_backs_off_after_eight_attempts() {
        use super::should_retry_artwork;
        for a in 0..8 {
            assert!(should_retry_artwork(a), "attempt {a} should retry");
        }
        assert!(should_retry_artwork(8));
        for a in 9..23 {
            assert!(!should_retry_artwork(a), "attempt {a} should skip");
        }
        assert!(should_retry_artwork(23));
    }

    #[test]
    fn notch_suppression_keeps_fixed_mode_interactive() {
        use super::{should_ignore_notch_cursor, should_suppress_notch_for_mode};
        assert!(!should_suppress_notch_for_mode("fixed"));
        assert!(should_suppress_notch_for_mode("smart"));
        assert!(should_suppress_notch_for_mode("peek"));

        assert!(should_ignore_notch_cursor(true, true));
        assert!(should_ignore_notch_cursor(true, false));
        assert!(should_ignore_notch_cursor(false, false));
        assert!(!should_ignore_notch_cursor(false, true));
    }

    #[test]
    fn names_a_process_from_the_process_list() {
        let own = std::env::current_exe().unwrap();
        let expected = own.file_name().unwrap().to_string_lossy().to_string();
        let found = unsafe { super::process_exe_name(std::process::id()) };
        assert_eq!(found.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn brightness_key_maps_0x216_and_0x217() {
        use crate::types::SystemCommand;
        assert!(matches!(super::brightness_key_command(0x216), Some(SystemCommand::BrightnessDown)));
        assert!(matches!(super::brightness_key_command(0x217), Some(SystemCommand::BrightnessUp)));
        assert!(super::brightness_key_command(0x215).is_none());
    }

    #[test]
    fn media_session_selection_prefers_current_and_fresh_over_stale_first() {
        use super::{choose_media_session_index, MediaSessionCandidate};

        let stale = MediaSessionCandidate {
            index: 0,
            source_id: "Brave".into(),
            title: "Stale Video".into(),
            is_playing: true,
            last_updated_ticks: 100,
        };
        let active = MediaSessionCandidate {
            index: 1,
            source_id: "Brave".into(),
            title: "Current Video".into(),
            is_playing: true,
            last_updated_ticks: 500,
        };
        let paused_spotify = MediaSessionCandidate {
            index: 2,
            source_id: "Spotify.exe".into(),
            title: "Paused Song".into(),
            is_playing: false,
            last_updated_ticks: 900,
        };
        let entries = vec![stale, active, paused_spotify];

        // When Windows GetCurrentSession points to index 1, choose it over stale index 0
        assert_eq!(
            choose_media_session_index(&entries, None, "Brave", "Stale Video", Some(("Brave", "Current Video"))),
            Some(1)
        );
        // Even with SELECTED_MEDIA_SOURCE="Brave", prefer current/fresh playing session
        assert_eq!(
            choose_media_session_index(&entries, Some("Brave"), "Brave", "Stale Video", Some(("Brave", "Current Video"))),
            Some(1)
        );
        // Without GetCurrentSession, fresher last_updated_ticks wins among playing sessions
        assert_eq!(
            choose_media_session_index(&entries, None, "Brave", "Stale Video", None),
            Some(1)
        );
        // When selected source is paused but has a playing entry vs paused entry, playing wins
        let mixed = vec![
            MediaSessionCandidate {
                index: 0,
                source_id: "Brave".into(),
                title: "Paused Stale".into(),
                is_playing: false,
                last_updated_ticks: 100,
            },
            MediaSessionCandidate {
                index: 1,
                source_id: "Brave".into(),
                title: "Playing Now".into(),
                is_playing: true,
                last_updated_ticks: 200,
            },
        ];
        assert_eq!(
            choose_media_session_index(&mixed, Some("Brave"), "", "", None),
            Some(1)
        );
    }

    #[test]
    fn edge_dwell_met_waits_until_elapsed_and_resets_outside() {
        let entered = AtomicI64::new(0);
        // First entry records timestamp and stays unconfirmed when dwell_ms > 0.
        assert!(!edge_dwell_met(&entered, true, 1000, 300));
        assert_eq!(entered.load(std::sync::atomic::Ordering::Relaxed), 1000);
        // Before dwell_ms elapses, still unconfirmed.
        assert!(!edge_dwell_met(&entered, true, 1299, 300));
        // At and after dwell_ms, confirmed.
        assert!(edge_dwell_met(&entered, true, 1300, 300));
        assert!(edge_dwell_met(&entered, true, 1500, 300));
        // Leaving resets the entry timestamp.
        assert!(!edge_dwell_met(&entered, false, 1600, 300));
        assert_eq!(entered.load(std::sync::atomic::Ordering::Relaxed), 0);
        // Zero dwell confirms immediately on first entry.
        assert!(edge_dwell_met(&entered, true, 2000, 0));
    }

    #[test]
    fn edge_dwell_waiting_only_while_inside_and_unconfirmed() {
        assert!(edge_dwell_waiting(true, false));
        assert!(!edge_dwell_waiting(true, true));
        assert!(!edge_dwell_waiting(false, false));
        assert!(!edge_dwell_waiting(false, true));
    }

    #[test]
    fn overlay_edge_y_span_centers_card_on_monitor() {
        use super::{edge_area_allows_x, overlay_edge_y_span};

        // 1080p at 1.0 scale -> centered 196px card spans y = 442..638; ±60px pad -> 382..698.
        let span = overlay_edge_y_span(0, 1080, 1.0);
        assert_eq!(span, (442, 638));
        // Full mode allows corners.
        assert!(edge_area_allows_x(false, Some(span), 0, 60));
        assert!(edge_area_allows_x(false, Some(span), 1079, 60));
        // Narrow mode allows near the centered card and ignores top/bottom corners.
        assert!(edge_area_allows_x(true, Some(span), 382, 60));
        assert!(edge_area_allows_x(true, Some(span), 698, 60));
        assert!(!edge_area_allows_x(true, Some(span), 381, 60));
        assert!(!edge_area_allows_x(true, Some(span), 699, 60));
        assert!(!edge_area_allows_x(true, Some(span), 0, 60));
        assert!(!edge_area_allows_x(true, Some(span), 1079, 60));
    }

    #[test]
    fn empty_title_console_windows_show_only_when_enabled() {
        use super::should_include_window_title;

        assert!(should_include_window_title(5, "Chrome_WidgetWin_1", false));
        assert!(!should_include_window_title(0, "Chrome_WidgetWin_1", true));
        assert!(!should_include_window_title(0, "ConsoleWindowClass", false));
        assert!(should_include_window_title(0, "ConsoleWindowClass", true));
    }

    #[test]
    fn fullscreen_monitor_stays_hidden_when_focus_moves_to_another_monitor() {
        use super::monitor_fullscreen;
        let covered = [(0, 0)];
        assert!(monitor_fullscreen((0, 0), Some((1920, 0)), false, &covered));
        assert!(!monitor_fullscreen((1920, 0), Some((1920, 0)), false, &covered));
    }

    #[test]
    fn focused_monitor_follows_the_foreground_window() {
        use super::monitor_fullscreen;
        assert!(!monitor_fullscreen((0, 0), Some((0, 0)), false, &[(0, 0)]));
        assert!(monitor_fullscreen((0, 0), Some((0, 0)), true, &[]));
        assert!(!monitor_fullscreen((0, 0), None, false, &[]));
    }

    #[test]
    fn resolve_focused_monitor_never_claims_monitor_for_unmeasured_window() {
        use super::resolve_focused_monitor;

        // Invalid HWND -> None (all monitors fall back to z-order scan).
        assert_eq!(resolve_focused_monitor(false, false, false, Some((0, 0)), Some((0, 0))), None);
        // Desktop focused -> None.
        assert_eq!(resolve_focused_monitor(true, true, true, Some((0, 0)), Some((0, 0))), None);
        // Known shell (Start/Taskbar) -> cursor monitor origin.
        assert_eq!(resolve_focused_monitor(true, false, true, Some((1920, 0)), None), Some((1920, 0)));
        // Transient/unmeasured window (fg_info is None) -> None (must NOT claim primary (0, 0)).
        assert_eq!(resolve_focused_monitor(true, false, false, Some((0, 0)), None), None);
        // Measured foreground window -> its monitor origin.
        assert_eq!(resolve_focused_monitor(true, false, false, Some((0, 0)), Some((1920, 0))), Some((1920, 0)));
    }

    #[test]
    fn window_over_the_panel_strip_counts_as_overlap_on_any_monitor() {
        use super::{rect_overlaps_dock, rect_overlaps_notch};
        use windows::Win32::Foundation::RECT;

        let dock = crate::types::IntRect { x: 880, y: 0, width: 800, height: 64 };
        let near_bottom = RECT { left: 2560 + 1000, top: 600, right: 2560 + 1600, bottom: 1420 };
        let above_dock = RECT { left: 2560 + 1000, top: 200, right: 2560 + 1600, bottom: 900 };
        let beside_dock = RECT { left: 2560 + 100, top: 600, right: 2560 + 600, bottom: 1420 };
        assert!(rect_overlaps_dock(2560, 1440, near_bottom, dock, 1.0));
        assert!(!rect_overlaps_dock(2560, 1440, above_dock, dock, 1.0));
        assert!(!rect_overlaps_dock(2560, 1440, beside_dock, dock, 1.0));

        let notch = crate::types::IntRect { x: 1080, y: 0, width: 400, height: 40 };
        let at_top = RECT { left: 2560 + 1200, top: 0, right: 2560 + 1800, bottom: 700 };
        let lower = RECT { left: 2560 + 1200, top: 300, right: 2560 + 1800, bottom: 900 };
        assert!(rect_overlaps_notch(2560, 0, at_top, notch, 1.0));
        assert!(!rect_overlaps_notch(2560, 0, lower, notch, 1.0));
    }

    #[test]
    fn css_rect_to_physical_bounds_rounds_outwards_at_fractional_dpi() {
        use super::css_rect_to_physical_bounds;

        // 1.25x scale: x=10 -> 12.5 (floor 12), right=111 -> 138.75 (ceil 139),
        // y=1 -> 1.25 (floor 1), bottom=46 -> 57.5 (ceil 58).
        let r125 = crate::types::IntRect { x: 10, y: 1, width: 101, height: 45 };
        assert_eq!(css_rect_to_physical_bounds(100, 200, r125, 1.25), (112, 201, 239, 258));

        // 1.5x scale: x=251 -> 376.5 (floor 376), right=852 -> 1278.0 (ceil 1278),
        // y=15 -> 22.5 (floor 22), bottom=76 -> 114.0 (ceil 114).
        // Truncating x and width separately would have clipped right to 1277 and bottom to 1013.
        let r150 = crate::types::IntRect { x: 251, y: 15, width: 601, height: 61 };
        assert_eq!(css_rect_to_physical_bounds(0, 900, r150, 1.5), (376, 922, 1278, 1014));
    }

    #[test]
    fn shortcut_dedup_preserves_distinct_apps_sharing_executable_name() {
        use super::is_duplicate_shortcut;
        use crate::types::AppInfo;

        let firefox = AppInfo {
            name: "Firefox".to_string(),
            path: r"C:\Program Files\Mozilla Firefox\firefox.exe".to_string(),
            icon: None,
            is_running: false,
            hwnd: None,
            executable: Some("firefox.exe".to_string()),
            all_hwnds: None,
            args: None,
        };

        // Distinct app sharing firefox.exe in another directory must NOT be deduplicated.
        assert!(!is_duplicate_shortcut(
            &firefox,
            None,
            "Tor Browser",
            r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Tor Browser.lnk",
            false,
            Some("firefox.exe"),
            Some(r"C:\Users\u\Desktop\Tor Browser\Browser\firefox.exe"),
        ));

        // Same resolved target path with a different shortcut display name IS a duplicate.
        assert!(is_duplicate_shortcut(
            &firefox,
            None,
            "Mozilla Firefox",
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Mozilla Firefox.lnk",
            false,
            Some("firefox.exe"),
            Some(r"C:\Program Files\Mozilla Firefox\firefox.exe"),
        ));

        // Same display name IS a duplicate even without a resolved target.
        assert!(is_duplicate_shortcut(
            &firefox,
            None,
            "Firefox",
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Firefox.lnk",
            false,
            Some("firefox.exe"),
            None,
        ));

        // Two .lnk shortcuts pointing to the same target are deduplicated via existing_resolved_target.
        let tor_lnk = AppInfo {
            name: "Tor Browser".to_string(),
            path: r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Tor Browser.lnk".to_string(),
            icon: None,
            is_running: false,
            hwnd: None,
            executable: Some("firefox.exe".to_string()),
            all_hwnds: None,
            args: None,
        };
        assert!(is_duplicate_shortcut(
            &tor_lnk,
            Some(r"C:\Users\u\Desktop\Tor Browser\Browser\firefox.exe"),
            "Start Tor Browser",
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Start Tor Browser.lnk",
            false,
            Some("firefox.exe"),
            Some(r"C:\Users\u\Desktop\Tor Browser\Browser\firefox.exe"),
        ));
    }

    #[test]
    fn capture_overlay_predicate_distinguishes_overlay_from_editor() {
        use super::is_capture_overlay_window;
        use windows::Win32::UI::WindowsAndMessaging::{WS_CAPTION, WS_EX_TOPMOST};

        // ScreenClippingHost.exe covering the monitor is always a capture overlay.
        assert!(is_capture_overlay_window("screenclippinghost.exe", WS_CAPTION.0, 0, true));
        assert!(!is_capture_overlay_window("screenclippinghost.exe", 0, WS_EX_TOPMOST.0, false));

        // Borderless or topmost SnippingTool.exe covering the monitor is a capture overlay.
        assert!(is_capture_overlay_window("snippingtool.exe", 0, 0, true));
        assert!(is_capture_overlay_window("snippingtool.exe", WS_CAPTION.0, WS_EX_TOPMOST.0, true));

        // Normal captioned maximized SnippingTool.exe / ScreenSketch.exe editor must NOT trigger.
        assert!(!is_capture_overlay_window("snippingtool.exe", WS_CAPTION.0, 0, true));
        assert!(!is_capture_overlay_window("screensketch.exe", WS_CAPTION.0, 0, true));
        assert!(is_capture_overlay_window("screensketch.exe", 0, 0, true));

        // Non-capture apps must never match, even when borderless and topmost.
        assert!(!is_capture_overlay_window("notepad.exe", 0, WS_EX_TOPMOST.0, true));
        assert!(!is_capture_overlay_window("chrome.exe", 0, WS_EX_TOPMOST.0, true));
    }

    #[test]
    fn cross_monitor_handoff_targets_only_other_monitors_on_active_edge() {
        use super::cross_monitor_handoff_targets;

        // Single monitor: never returns targets even on active edge.
        assert!(cross_monitor_handoff_targets("dock", true, &["dock"]).is_empty());
        assert!(cross_monitor_handoff_targets("main", true, &["main"]).is_empty());

        // Multi-monitor without active edge: never dismisses other monitors.
        assert!(cross_monitor_handoff_targets("dock", false, &["dock", "dock-mon2"]).is_empty());
        assert!(cross_monitor_handoff_targets("main-mon2", false, &["main", "main-mon2"]).is_empty());

        // Multi-monitor with active edge: dismisses every sibling of the same kind, never self.
        assert_eq!(
            cross_monitor_handoff_targets("dock", true, &["dock", "dock-mon2", "dock-mon3"]),
            vec!["dock-mon2", "dock-mon3"]
        );
        assert_eq!(
            cross_monitor_handoff_targets("main-mon2", true, &["main", "main-mon2", "main-mon3"]),
            vec!["main", "main-mon3"]
        );
    }

    #[test]
    fn notch_stale_hover_and_expiry_recheck_predicates() {
        use super::{dock_hover_went_stale, hover_waits_on_expiry};

        assert!(dock_hover_went_stale(true, true));
        assert!(!dock_hover_went_stale(false, true));
        assert!(!dock_hover_went_stale(true, false));

        assert!(hover_waits_on_expiry(false, true));
        assert!(!hover_waits_on_expiry(true, true));
        assert!(!hover_waits_on_expiry(false, false));
    }
}
