use std::sync::atomic::{AtomicBool, AtomicU8, AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW, CallNextHookEx};
use crate::types::*;
use crate::state::*;
use crate::utils::*;
use super::mouse_hit::{CAPTURE_RECHECK, now_ms};

static KEYBOARD_HOOK_APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

/// Whether a HUD overlay is enabled. When it's off, its keys and the native flyout stay with Windows.
pub(super) fn overlay_enabled(app: &AppHandle, key: &str) -> bool {
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
pub(super) static START_TAP_MS: AtomicI64 = AtomicI64::new(0);
pub(super) const START_TAP_GRACE_MS: i64 = 700;
/// Whether Start/Search is currently detected on the primary dock monitor.
pub(super) static CURRENT_START_ON_DOCK: AtomicBool = AtomicBool::new(false);
/// Visibility/overlap loop thread, unparked on foreground or Win tap changes.
pub(super) static VISIBILITY_THREAD: OnceLock<std::thread::Thread> = OnceLock::new();
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn brightness_key_maps_0x216_and_0x217() {
        use crate::types::SystemCommand;
        assert!(matches!(super::brightness_key_command(0x216), Some(SystemCommand::BrightnessDown)));
        assert!(matches!(super::brightness_key_command(0x217), Some(SystemCommand::BrightnessUp)));
        assert!(super::brightness_key_command(0x215).is_none());
    }
}
