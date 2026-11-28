use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::EnumWindows;

use crate::services::enum_windows_proc;
use crate::state::*;
use crate::types::AppInfo;
use crate::utils::*;

/// Coalesces duplicate events, has to stay below the double-click interval.
const START_TOGGLE_DEBOUNCE_MS: i64 = 80;

/// How long Win is held so the shell registers the tap.
const START_WIN_KEY_HOLD_MS: u64 = 40;

fn send_key_tap(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY, hold_ms: u64) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, KEYBDINPUT, KEYEVENTF_KEYUP};
    let down = [INPUT {
        r#type: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: Default::default(), time: 0, dwExtraInfo: 0 },
        },
    }];
    let up = [INPUT {
        r#type: windows::Win32::UI::Input::KeyboardAndMouse::INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: KEYEVENTF_KEYUP, time: 0, dwExtraInfo: 0 },
        },
    }];
    unsafe {
        SendInput(&down, std::mem::size_of::<INPUT>() as i32);
        if hold_ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(hold_ms));
        }
        SendInput(&up, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Start is hosted by StartMenuExperienceHost, or SearchHost on 24H2+. Search by SearchHost, or
/// SearchApp/SearchUI on older builds. Window titles are localized, so the executable decides.
fn is_start_menu_exe(path: &str) -> bool {
    let file = path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
    matches!(file.as_str(), "startmenuexperiencehost.exe" | "searchhost.exe" | "searchapp.exe" | "searchui.exe" | "shellexperiencehost.exe")
}

/// True while the native Start menu or Search has focus.
pub fn is_start_menu_open() -> bool {
    is_start_menu_window(unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() })
}

pub(crate) fn is_start_menu_window(window: HWND) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, GetClassNameW, GetWindowTextW, GetWindowThreadProcessId, GA_ROOT};

    unsafe {
        if window.0.is_null() {
            return false;
        }
        let root = GetAncestor(window, GA_ROOT);
        let target = if !root.is_invalid() && !root.0.is_null() { root } else { window };

        let mut pid = 0u32;
        GetWindowThreadProcessId(target, Some(&mut pid));
        if pid != 0 {
            if let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                let mut buf = [0u16; 260];
                let mut len = buf.len() as u32;
                let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
                let _ = CloseHandle(process);
                if ok.is_ok() && is_start_menu_exe(&String::from_utf16_lossy(&buf[..len as usize])) {
                    return true;
                }
            }
        }

        let mut class = [0u16; 128];
        let class_len = GetClassNameW(target, &mut class).max(0) as usize;
        let class_str = String::from_utf16_lossy(&class[..class_len]);

        let mut text = [0u16; 128];
        let text_len = GetWindowTextW(target, &mut text).max(0) as usize;
        let title = String::from_utf16_lossy(&text[..text_len]);

        (class_str == "Windows.UI.Core.CoreWindow" || class_str == "SimpleWindow" || class_str == "XamlExplorerHostIslandWindow")
            && (title == "Start" || title == "Search" || title.is_empty())
    }
}

#[cfg(test)]
mod close_window_tests {
    use super::process_outranks_us;

    #[test]
    fn our_own_process_does_not_outrank_us() {
        assert!(!process_outranks_us(std::process::id()));
    }

    #[test]
    fn a_process_that_cannot_be_opened_is_not_treated_as_elevated() {
        assert!(!process_outranks_us(0));
    }
}

#[cfg(test)]
mod start_menu_tests {
    use super::is_start_menu_exe;

    #[test]
    fn recognizes_start_and_search_hosts_by_executable() {
        assert!(is_start_menu_exe(r"C:\Windows\SystemApps\Microsoft.Windows.StartMenuExperienceHost_cw5n1h2txyewy\StartMenuExperienceHost.exe"));
        assert!(is_start_menu_exe(r"C:\Windows\SystemApps\MicrosoftWindows.Client.CBS_cw5n1h2txyewy\SearchHost.exe"));
        assert!(is_start_menu_exe(r"C:\Windows\SystemApps\Microsoft.Windows.Search_cw5n1h2txyewy\SearchApp.exe"));
        assert!(!is_start_menu_exe(r"C:\Windows\explorer.exe"));
        assert!(!is_start_menu_exe(r"C:\Windows\System32\Taskmgr.exe"));
    }
}

/// Toggles the native Start menu. If it's open, Escape closes it, otherwise tap Win.
/// Tapping Win blindly replayed the open animation on a quick second click.
pub(super) fn toggle_start_menu() {
    let now = get_now_ms();
    if now - LAST_START_TOGGLE_MS.load(Ordering::Relaxed) < START_TOGGLE_DEBOUNCE_MS {
        return;
    }
    LAST_START_TOGGLE_MS.store(now, Ordering::Relaxed);
    let open_at_press = crate::state::START_OPEN_AT_PRESS.swap(false, Ordering::Relaxed);

    tauri::async_runtime::spawn_blocking(move || {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_LWIN};
        if is_start_menu_open() {
            send_key_tap(VK_ESCAPE, 0);
        } else if !open_at_press && !ask_explorer_to_open_start() {
            send_key_tap(VK_LWIN, START_WIN_KEY_HOLD_MS);
        }
    });
}

/// SendInput is silently dropped while an elevated app owns the foreground (Task Manager), so the
/// Start button asks Explorer directly instead.
fn ask_explorer_to_open_start() -> bool {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, SC_TASKLIST, WM_SYSCOMMAND};
    unsafe {
        let Ok(tray) = FindWindowW(windows::core::w!("Shell_TrayWnd"), None) else {
            return false;
        };
        PostMessageW(Some(tray), WM_SYSCOMMAND, WPARAM(SC_TASKLIST as usize), LPARAM(0)).is_ok()
    }
}

pub fn send_win_chord(key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_LWIN};
    let input = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, time: 0, dwExtraInfo: 0 },
        },
    };
    let inputs = [input(VK_LWIN, false), input(key, false), input(key, true), input(VK_LWIN, true)];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

#[tauri::command]
pub async fn get_active_windows() -> Vec<AppInfo> {
    tauri::async_runtime::spawn_blocking(move || {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
        // COM is needed here, each window's AppUserModelID comes from the shell property store.
        let com_initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).is_ok() };

        let mut apps: Vec<AppInfo> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(enum_windows_proc), LPARAM(&mut apps as *mut Vec<AppInfo> as isize));
        }

        let mut grouped: HashMap<String, AppInfo> = HashMap::new();
        
        for app in apps {
            let path = app.path.to_lowercase();
            let name = app.name.to_lowercase();
            
            // For host processes (Edge, Chrome, Brave, ApplicationFrameHost), use path + name 
            // so that different PWAs/UWP apps are separate dock items.
            let key = if path.contains("msedge.exe") || path.contains("chrome.exe") || path.contains("brave.exe") || path.contains("applicationframehost.exe") {
                format!("{}:{}", path, name)
            } else if let Some(ref exe) = app.executable {
                format!("{}:{}", path, exe.to_lowercase())
            } else {
                path.clone()
            };

            if let Some(existing) = grouped.get_mut(&key) {
                let items_to_add = app.all_hwnds.unwrap_or_else(|| vec![(app.hwnd.unwrap_or(0), app.name.clone())]);
                if let Some(ref mut hwnds) = existing.all_hwnds {
                    hwnds.extend(items_to_add);
                } else {
                    let mut list = vec![(existing.hwnd.unwrap_or(0), existing.name.clone())];
                    list.extend(items_to_add);
                    existing.all_hwnds = Some(list);
                }
            } else {
                let mut new_app = app.clone();
                if new_app.all_hwnds.is_none() {
                    new_app.all_hwnds = Some(vec![(app.hwnd.unwrap_or(0), app.name.clone())]);
                }
                grouped.insert(key, new_app);
            }
        }

        let focus_guard = if let Some(map) = crate::state::FOCUS_TIMESTAMPS.get() {
            map.lock().ok()
        } else {
            None
        };

        let mut result_apps: Vec<AppInfo> = grouped.into_values().collect();

        for app in &mut result_apps {
            if let Some(ref mut hwnds) = app.all_hwnds {
                hwnds.sort_by(|a, b| {
                    let ts_a = focus_guard.as_ref()
                        .and_then(|g| g.get(&a.0))
                        .copied()
                        .unwrap_or(0);
                    let ts_b = focus_guard.as_ref()
                        .and_then(|g| g.get(&b.0))
                        .copied()
                        .unwrap_or(0);
                    ts_b.cmp(&ts_a)
                });
            }
        }

        if com_initialized {
            unsafe { CoUninitialize(); }
        }

        result_apps
    }).await.unwrap_or_default()
}

unsafe fn force_set_foreground(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

    let _ = SetForegroundWindow(hwnd);

    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, IsIconic, SwitchToThisWindow};
    if GetForegroundWindow() != hwnd || IsIconic(hwnd).as_bool() {
        SwitchToThisWindow(hwnd, true);
    }
}

static PREVIOUS_FOREGROUND: std::sync::Mutex<Option<(isize, isize)>> = std::sync::Mutex::new(None);

static LAST_FOCUS_CLICK: std::sync::Mutex<Option<(isize, i64)>> = std::sync::Mutex::new(None);

fn is_repeat_click(hwnd: isize) -> bool {
    let now = crate::utils::get_now_ms();
    let Ok(mut guard) = LAST_FOCUS_CLICK.lock() else { return false };
    let repeat = matches!(*guard, Some((last_hwnd, at)) if last_hwnd == hwnd && now - at < 350);
    if !repeat {
        *guard = Some((hwnd, now));
    }
    repeat
}

unsafe fn send_behind(hwnd: HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible,
        GWL_EXSTYLE, GW_HWNDNEXT, WS_EX_TOOLWINDOW,
    };
    let my_pid = std::process::id();
    let usable = |h: HWND| {
        if h == hwnd || !IsWindow(Some(h)).as_bool() || !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        pid != my_pid
    };

    let previous = PREVIOUS_FOREGROUND.lock().ok().and_then(|g| *g).filter(|(target, _)| *target == hwnd.0 as isize);
    if let Some((_, prev)) = previous {
        let prev = HWND(prev as *mut _);
        if usable(prev) {
            force_set_foreground(prev);
            crate::diagnostics::log(&format!("focus: sent {:?} behind by activating the previous window {:?}", hwnd.0, prev.0));
            return;
        }
    }

    let mut next = GetWindow(hwnd, GW_HWNDNEXT);
    while let Ok(candidate) = next {
        let is_tool = (GetWindowLongW(candidate, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0) != 0;
        if usable(candidate) && !is_tool && GetWindowTextLengthW(candidate) > 0 {
            force_set_foreground(candidate);
            crate::diagnostics::log(&format!("focus: sent {:?} behind by activating the next window {:?}", hwnd.0, candidate.0));
            return;
        }
        next = GetWindow(candidate, GW_HWNDNEXT);
    }
    crate::diagnostics::log(&format!("focus: found no window to activate behind {:?}", hwnd.0));
}

fn workspace_offset(
    monitor: &windows::Win32::Graphics::Gdi::MONITORINFO,
    tool_window: bool,
) -> (i32, i32) {
    if tool_window {
        (0, 0)
    } else {
        (
            monitor.rcWork.left - monitor.rcMonitor.left,
            monitor.rcWork.top - monitor.rcMonitor.top,
        )
    }
}

fn shift_rect(
    rect: windows::Win32::Foundation::RECT,
    (dx, dy): (i32, i32),
) -> windows::Win32::Foundation::RECT {
    windows::Win32::Foundation::RECT {
        left: rect.left + dx,
        top: rect.top + dy,
        right: rect.right + dx,
        bottom: rect.bottom + dy,
    }
}

pub(super) fn on_screen_restore_rect(
    normal: windows::Win32::Foundation::RECT,
    work: windows::Win32::Foundation::RECT,
) -> windows::Win32::Foundation::RECT {
    let work_w = work.right - work.left;
    let work_h = work.bottom - work.top;
    let fit = |size: i32, available: i32| {
        if size >= available / 3 {
            size.min(available)
        } else {
            available * 2 / 3
        }
    };
    let w = fit(normal.right - normal.left, work_w);
    let h = fit(normal.bottom - normal.top, work_h);
    let left = work.left + (work_w - w) / 2;
    let top = work.top + (work_h - h) / 2;
    windows::Win32::Foundation::RECT {
        left,
        top,
        right: left + w,
        bottom: top + h,
    }
}

fn restore_rect_on_monitor(
    on_screen: windows::Win32::Foundation::RECT,
    target: &windows::Win32::Graphics::Gdi::MONITORINFO,
    tool_window: bool,
) -> windows::Win32::Foundation::RECT {
    let (dx, dy) = workspace_offset(target, tool_window);
    shift_rect(on_screen_restore_rect(on_screen, target.rcWork), (-dx, -dy))
}

unsafe fn bring_restore_position_on_screen(hwnd: HWND) {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        MONITOR_DEFAULTTONULL, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetWindowLongW, GetWindowPlacement, SetWindowPlacement, GWL_EXSTYLE,
        WINDOWPLACEMENT, WS_EX_TOOLWINDOW,
    };

    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if GetWindowPlacement(hwnd, &mut placement).is_err() {
        return;
    }

    let monitor_info = |monitor| {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        GetMonitorInfoW(monitor, &mut info)
            .as_bool()
            .then_some(info)
    };
    let tool_window = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0;
    let normal = placement.rcNormalPosition;
    let Some(source) = monitor_info(MonitorFromRect(&normal, MONITOR_DEFAULTTONEAREST)) else {
        return;
    };
    let on_screen = shift_rect(normal, workspace_offset(&source, tool_window));
    if !MonitorFromRect(&on_screen, MONITOR_DEFAULTTONULL).is_invalid() {
        return;
    }

    let mut cursor = POINT::default();
    let _ = GetCursorPos(&mut cursor);
    let Some(target) = monitor_info(MonitorFromPoint(cursor, MONITOR_DEFAULTTOPRIMARY)) else {
        return;
    };
    placement.rcNormalPosition = restore_rect_on_monitor(on_screen, &target, tool_window);
    let _ = SetWindowPlacement(hwnd, &placement);
}

fn last_focused_window() -> Option<HWND> {
    let map = crate::state::FOCUS_TIMESTAMPS.get()?.lock().ok()?;
    let (&last, _) = map.iter().max_by_key(|(_, &at)| at)?;
    let last = HWND(last as *mut _);
    unsafe { windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(last)) }
        .as_bool()
        .then_some(last)
}

#[tauri::command]
pub async fn focus_window(app: AppHandle, hwnd: isize) {
    if is_repeat_click(hwnd) {
        return;
    }
    let dock_hwnds: Vec<isize> = app
        .webview_windows()
        .iter()
        .filter(|(label, _)| crate::state::is_dock_label(label))
        .filter_map(|(_, window)| window.hwnd().ok().map(|h| h.0 as isize))
        .collect();
    tauri::async_runtime::spawn_blocking(move || unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GA_ROOTOWNER, GetAncestor, GetForegroundWindow, GetWindowThreadProcessId,
            IsIconic, IsWindowVisible, ShowWindow, SW_MINIMIZE, SW_RESTORE, SW_SHOW,
        };
        let hwnd = HWND(hwnd as *mut _);
        let my_pid = std::process::id();

        let foreground_before = GetForegroundWindow();
        if !foreground_before.is_invalid() && foreground_before != hwnd {
            let mut before_pid = 0u32;
            GetWindowThreadProcessId(foreground_before, Some(&mut before_pid));
            if before_pid != my_pid {
                if let Ok(mut guard) = PREVIOUS_FOREGROUND.lock() {
                    *guard = Some((hwnd.0 as isize, foreground_before.0 as isize));
                }
            }
        }

        if !IsWindowVisible(hwnd).as_bool() {
            crate::diagnostics::log(&format!("focus: showing hidden {:?} with SW_RESTORE", hwnd.0));
            let _ = ShowWindow(hwnd, SW_SHOW);
            bring_restore_position_on_screen(hwnd);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            force_set_foreground(hwnd);
        } else if IsIconic(hwnd).as_bool() {
            crate::diagnostics::log(&format!("focus: restoring iconic {:?} with SW_RESTORE", hwnd.0));
            bring_restore_position_on_screen(hwnd);
            let _ = ShowWindow(hwnd, SW_RESTORE);
            force_set_foreground(hwnd);
        } else {
            // Only minimize when this exact window (or an owned popup/dialog via GA_ROOTOWNER)
            // is in front, never when another window from the same process is active.
            let mut fg = GetForegroundWindow();
            if dock_hwnds.contains(&(fg.0 as isize)) {
                if let Some(last) = last_focused_window() {
                    fg = last;
                }
            }
            let should_minimize = !fg.is_invalid()
                && (fg == hwnd || GetAncestor(fg, GA_ROOTOWNER) == hwnd);

            if should_minimize {
                // Only grab a ~2ms raw pixel copy if THUMBNAIL_CACHE doesn't already have a recent
                // entry, then minimize IMMEDIATELY and resize/PNG-encode on a background thread so
                // clicking the dock icon feels instantaneous.
                let hwnd_key = hwnd.0 as isize;
                let skip_gdi_snapshot = crate::utils::is_chrome_window(hwnd)
                    && crate::live_preview::is_composition_enabled();
                let has_fresh_cache = skip_gdi_snapshot
                    || crate::state::THUMBNAIL_CACHE
                        .get()
                        .and_then(|c| c.lock().ok())
                        .and_then(|g| g.get(&hwnd_key).map(|(_, ts)| *ts))
                        .is_some_and(|ts| crate::utils::get_now_ms().saturating_sub(ts) < 5_000);
                let raw_snapshot = if has_fresh_cache {
                    None
                } else {
                    crate::utils::capture_hwnd_raw_bgra_in_front(hwnd)
                };
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
                if let Some((pixels, w, h)) = raw_snapshot {
                    std::thread::spawn(move || {
                        if let Some(img) = crate::utils::encode_thumbnail(pixels, w, h, 320, 200) {
                            let cache = crate::state::THUMBNAIL_CACHE.get_or_init(|| {
                                std::sync::Mutex::new(std::collections::HashMap::new())
                            });
                            if let Ok(mut guard) = cache.lock() {
                                guard.insert(hwnd_key, (img, crate::utils::get_now_ms()));
                            }
                        }
                    });
                }
                if !IsIconic(hwnd).as_bool() {
                    use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SC_MINIMIZE, SMTO_ABORTIFHUNG, WM_SYSCOMMAND};
                    let _ = SendMessageTimeoutW(
                        hwnd,
                        WM_SYSCOMMAND,
                        windows::Win32::Foundation::WPARAM(SC_MINIMIZE as usize),
                        windows::Win32::Foundation::LPARAM(0),
                        SMTO_ABORTIFHUNG,
                        300,
                        None,
                    );
                }
                let minimized = IsIconic(hwnd).as_bool();
                crate::diagnostics::log(&format!("focus: minimize of {:?} worked: {minimized}", hwnd.0));
                if !minimized {
                    send_behind(hwnd);
                }
            } else {
                crate::diagnostics::log(&format!("focus: bringing {:?} to foreground", hwnd.0));
                force_set_foreground(hwnd);
            }
        }
    }).await.unwrap_or_default();
}

/// Brings a window to the foreground and restores it if minimized.
/// Unlike `focus_window`, this NEVER minimizes the window, making it safe for hover-to-activate / drag-and-drop.
#[tauri::command]
pub async fn bring_hwnd_to_front(hwnd: isize) {
    tauri::async_runtime::spawn_blocking(move || unsafe {
        use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        use windows::Win32::UI::WindowsAndMessaging::{
            BringWindowToTop, GetForegroundWindow, GetWindowLongW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, SetWindowPos,
            ShowWindow, GWL_EXSTYLE, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE, SW_RESTORE, SW_SHOW, WS_EX_TOPMOST,
        };
        let hwnd = windows::Win32::Foundation::HWND(hwnd as *mut _);
        if !IsWindowVisible(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_SHOW);
            bring_restore_position_on_screen(hwnd);
            let _ = ShowWindow(hwnd, SW_RESTORE);
        } else if IsIconic(hwnd).as_bool() {
            bring_restore_position_on_screen(hwnd);
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }

        let mine = GetCurrentThreadId();
        let front = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let attached = front != 0 && front != mine && AttachThreadInput(mine, front, true).as_bool();

        force_set_foreground(hwnd);
        let _ = BringWindowToTop(hwnd);
        if (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) == 0 {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
        }

        if attached {
            let _ = AttachThreadInput(mine, front, false);
        }
    })
    .await
    .unwrap_or_default();
}

/// Focuses one window of a multi-window item. hwnds are most recent first: if none is
/// in front take the first, otherwise go to the next one so clicks cycle through all of them.
#[tauri::command]
pub async fn focus_app_windows(hwnds: Vec<isize>) {
    if let Some(&first) = hwnds.first() {
        if is_repeat_click(first) {
            return;
        }
    }
    tauri::async_runtime::spawn_blocking(move || unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, IsIconic, IsWindowVisible, SetForegroundWindow, ShowWindow,
            SW_RESTORE, SW_SHOW,
        };
        let Some(&most_recent) = hwnds.first() else {
            return;
        };
        let foreground = GetForegroundWindow().0 as isize;
        let mut ring = hwnds.clone();
        ring.sort_unstable();
        let target = match ring.iter().position(|&h| h == foreground) {
            Some(i) => ring[(i + 1) % ring.len()],
            None => most_recent,
        };

        let hwnd = HWND(target as *mut _);
        if !IsWindowVisible(hwnd).as_bool() {
            crate::diagnostics::log(&format!("focus_app_windows: showing hidden {:?} with SW_SHOW", hwnd.0));
            let _ = ShowWindow(hwnd, SW_SHOW);
        }
        if IsIconic(hwnd).as_bool() {
            crate::diagnostics::log(&format!("focus_app_windows: restoring iconic {:?} with SW_RESTORE", hwnd.0));
            bring_restore_position_on_screen(hwnd);
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let fg_ok = SetForegroundWindow(hwnd).as_bool();
        crate::diagnostics::log(&format!("focus_app_windows: SetForegroundWindow({:?}) -> {fg_ok}", hwnd.0));
    })
    .await
    .unwrap_or_default();
}

fn terminate_process(pid: u32) -> Result<(), bool> {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED};
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    match unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) } {
        Ok(handle) => {
            let terminated = unsafe { TerminateProcess(handle, 1).is_ok() };
            unsafe { let _ = CloseHandle(handle); }
            if terminated { Ok(()) } else { Err(false) }
        }
        Err(e) => Err(e.code() == windows::core::HRESULT::from_win32(ERROR_ACCESS_DENIED.0)),
    }
}

fn kill_elevated(pid: u32) -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let verb = crate::uninstall_registry::wide("runas");
    let file = crate::uninstall_registry::wide("taskkill");
    let args = crate::uninstall_registry::wide(&format!("/F /T /PID {pid}"));
    let result = unsafe {
        ShellExecuteW(None, PCWSTR(verb.as_ptr()), PCWSTR(file.as_ptr()), PCWSTR(args.as_ptr()), None, SW_HIDE)
    };
    (result.0 as isize) > 32
}

#[tauri::command]
pub async fn end_task(hwnd: isize, all_hwnds: Option<Vec<isize>>) {
    use std::os::windows::process::CommandExt;
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    struct HostScan { host_pid: u32, found: u32 }
    unsafe extern "system" fn child_proc(child: HWND, lparam: LPARAM) -> windows::core::BOOL {
        let scan = &mut *(lparam.0 as *mut HostScan);
        let mut pid = 0u32;
        GetWindowThreadProcessId(child, Some(&mut pid));
        if pid != 0 && pid != scan.host_pid {
            scan.found = pid;
            return windows::core::BOOL(0);
        }
        windows::core::BOOL(1)
    }

    unsafe fn image_name(pid: u32) -> Option<String> {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let res = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = windows::Win32::Foundation::CloseHandle(handle);
        res.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }

    unsafe fn app_pid(hwnd: HWND) -> Option<u32> {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 { return None; }
        let name = image_name(pid)?.to_lowercase();
        if name.ends_with("\\explorer.exe") { return None; }
        if name.ends_with("\\applicationframehost.exe") {
            let mut scan = HostScan { host_pid: pid, found: 0 };
            let _ = EnumChildWindows(Some(hwnd), Some(child_proc), LPARAM(&mut scan as *mut HostScan as isize));
            return if scan.found != 0 { Some(scan.found) } else { None };
        }
        Some(pid)
    }

    tauri::async_runtime::spawn_blocking(move || unsafe {
        let mut handles = vec![hwnd];
        handles.extend(all_hwnds.unwrap_or_default());

        let own_pid = std::process::id();
        let mut by_pid: std::collections::BTreeMap<u32, Vec<isize>> = std::collections::BTreeMap::new();
        let mut politely = Vec::new();
        for h in handles {
            match app_pid(HWND(h as *mut _)) {
                Some(pid) if pid != own_pid => by_pid.entry(pid).or_default().push(h),
                Some(_) => {}
                None => politely.push(h),
            }
        }

        for (pid, hwnds) in by_pid {
            let killed = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid.to_string()])
                .creation_flags(0x0800_0000)
                .output()
                .is_ok_and(|o| o.status.success());
            if killed {
                crate::diagnostics::log(&format!("end task: taskkill ended pid {pid}"));
                continue;
            }
            match terminate_process(pid) {
                Ok(()) => {
                    crate::diagnostics::log(&format!("end task: terminated pid {pid} directly"));
                    continue;
                }
                Err(denied) => {
                    crate::diagnostics::log(&format!("end task: could not terminate pid {pid}, access denied: {denied}"));
                    if denied {
                        let asked = kill_elevated(pid);
                        crate::diagnostics::log(&format!("end task: elevation prompt for pid {pid} started: {asked}"));
                    }
                }
            }
            politely.extend(hwnds);
        }
        for h in politely {
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_CLOSE, windows::Win32::Foundation::WPARAM(0), windows::Win32::Foundation::LPARAM(0));
        }
    }).await.unwrap_or_default();
}

/// Whether a process runs with more rights than ours. Windows lets anyone open an elevated process
/// for basic queries but refuses to show its token, so that refusal is the answer.
fn process_outranks_us(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, HANDLE};
    use windows::Win32::Security::TOKEN_QUERY;
    use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return false;
        };
        let mut token = HANDLE::default();
        let result = OpenProcessToken(process, TOKEN_QUERY, &mut token);
        let _ = CloseHandle(process);
        match result {
            Ok(()) => {
                let _ = CloseHandle(token);
                false
            }
            Err(e) => e.code() == windows::core::HRESULT::from_win32(ERROR_ACCESS_DENIED.0),
        }
    }
}

/// Asks the window to close. Returns false if it is still open because Windows wouldn't let us ask:
/// an elevated window (Task Manager) doesn't take WM_CLOSE from us. It's then brought to the front so
/// the user can close it, rather than killing it and losing whatever it holds.
#[tauri::command]
pub async fn close_window(hwnd: isize) -> bool {
    tauri::async_runtime::spawn_blocking(move || unsafe {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow, PostMessageW, WM_CLOSE};
        #[link(name = "user32")]
        extern "system" {
            fn EndTask(hwnd: HWND, shutdown: windows::core::BOOL, force: windows::core::BOOL) -> windows::core::BOOL;
        }

        let hwnd = HWND(hwnd as *mut _);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let posted = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok();
        if posted && !process_outranks_us(pid) {
            return true;
        }

        // Without force this only tries the same polite close, but through the system.
        let _ = EndTask(hwnd, false.into(), false.into());
        std::thread::sleep(std::time::Duration::from_millis(400));
        if !IsWindow(Some(hwnd)).as_bool() {
            crate::diagnostics::log(&format!("close window: pid {pid} closed through EndTask"));
            return true;
        }
        crate::diagnostics::log(&format!("close window: pid {pid} refused, bringing it to the front"));
        force_set_foreground(hwnd);
        false
    }).await.unwrap_or(true)
}

const ELECTRON_THUMBAR_BUTTON_ID_BASE: u16 = 40001;

const THBN_CLICKED: usize = 0x1800;

const MAX_THUMBAR_BUTTONS: u16 = 7;

pub(crate) fn thumbar_command_wparam(button_index: u16) -> Option<usize> {
    if button_index >= MAX_THUMBAR_BUTTONS {
        return None;
    }
    Some((THBN_CLICKED << 16) | (ELECTRON_THUMBAR_BUTTON_ID_BASE + button_index) as usize)
}

/// Triggers an Electron window's registered thumbnail toolbar button without focusing it.
#[tauri::command]
pub fn trigger_thumbar_button(hwnd: isize, button_index: u16) -> Result<(), String> {
    let wparam = thumbar_command_wparam(button_index)
        .ok_or_else(|| format!("Invalid thumbar button index: {button_index}"))?;
    unsafe {
        use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{IsWindow, PostMessageW, WM_COMMAND};
        let target = HWND(hwnd as *mut _);
        if !IsWindow(Some(target)).as_bool() {
            return Err("Target window is no longer valid".to_string());
        }
        PostMessageW(Some(target), WM_COMMAND, WPARAM(wparam), LPARAM(0))
            .map_err(|e| format!("Failed to post thumbar command: {e}"))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn capture_window_thumbnail(hwnd: isize, max_width: u32, max_height: u32) -> Result<Option<(String, i64)>, String> {
    tauri::async_runtime::spawn_blocking(move || unsafe {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{IsWindow, IsIconic};

        let hwnd = HWND(hwnd as *mut _);
        if !IsWindow(Some(hwnd)).as_bool() { return None; }

        let hwnd_key = hwnd.0 as isize;
        let is_minimized = IsIconic(hwnd).as_bool();

        let mut focus_time = 0i64;
        if let Some(map) = crate::state::FOCUS_TIMESTAMPS.get() {
            if let Ok(guard) = map.lock() {
                focus_time = guard.get(&hwnd_key).copied().unwrap_or(0);
            }
        }

        let cached_entry = || {
            crate::state::THUMBNAIL_CACHE.get()
                .and_then(|cache| cache.lock().ok())
                .and_then(|g| g.get(&hwnd_key).cloned())
        };

        // Chromium/Electron windows (Chrome, Brave, Helium, Spotify, Discord, Antigravity, etc.)
        // skip PrintWindow and fall back to screen BitBlt, which bakes the topmost dock and preview
        // tooltip into the thumbnail (and poisons THUMBNAIL_CACHE when minimized). Use the native
        // DWM live thumbnail host for both active and minimized states whenever DWM is enabled.
        if crate::utils::is_chrome_window(hwnd) && crate::live_preview::is_composition_enabled() {
            return None;
        }

        if is_minimized {
            // Minimized windows can't be captured by PrintWindow or WGC without flashing on screen.
            // Return the cached PNG immediately; if empty (cold start), the frontend uses the DWM
            // thumbnail host as a zero-flash fallback until the window is next restored/minimized.
            return cached_entry().map(|(img, _)| (img, focus_time));
        }

        if let Some((img, captured_at)) = cached_entry() {
            if crate::utils::get_now_ms().saturating_sub(captured_at) < 2000 {
                return Some((img, focus_time));
            }
        }

        let result = crate::utils::capture_hwnd_to_base64(hwnd, max_width, max_height);

        // An elevated window behind others can't be captured, the last time it was in front is
        // the best there is.
        if result.is_none() {
            return cached_entry().map(|(img, _)| (img, focus_time));
        }

        if let Some(ref img) = result {
            if let Some(cache) = crate::state::THUMBNAIL_CACHE.get() {
                if let Ok(mut guard) = cache.lock() {
                    guard.insert(hwnd_key, (img.clone(), crate::utils::get_now_ms()));
                    // Prune dead windows if cache size exceeds 15
                    // Minimized windows (IsWindow == true) are kept intact
                    if guard.len() > 15 {
                        guard.retain(|&k, _| {
                            IsWindow(Some(HWND(k as *mut _))).as_bool()
                        });
                    }
                }
            }
        }

        result.map(|img| (img, focus_time))
    }).await.map_err(|e| e.to_string())
}

// Live Previews
#[tauri::command]
pub async fn set_live_thumbnails(
    window: tauri::Window,
    targets: Vec<crate::live_preview::LiveThumbnailTarget>,
    client_area_only: Option<bool>,
    session_id: Option<u64>,
) -> Result<Vec<isize>, String> {
    let dest_hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    let client_only = client_area_only.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        crate::live_preview::set_live_thumbnails(dest_hwnd, targets, client_only, session_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn clear_live_thumbnails(session_id: Option<u64>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::live_preview::clear_all_thumbnails(session_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_live_thumbnails_opacity(
    opacity: u8,
    session_id: Option<u64>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::live_preview::set_opacity(opacity, session_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn is_dwm_composition_enabled() -> Result<bool, String> {
    Ok(crate::live_preview::is_composition_enabled())
}

#[tauri::command]
pub async fn close_app(hwnd: isize, all_hwnds: Option<Vec<isize>>) {
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    tauri::async_runtime::spawn_blocking(move || unsafe {
        let mut handles = vec![hwnd];
        handles.extend(all_hwnds.unwrap_or_default());
        for h in handles {
            let win = HWND(h as *mut _);
            if windows::Win32::UI::WindowsAndMessaging::IsWindow(Some(win)).as_bool() {
                let _ = PostMessageW(Some(win), WM_CLOSE, windows::Win32::Foundation::WPARAM(0), windows::Win32::Foundation::LPARAM(0));
            }
        }
    }).await.unwrap_or(());
}
