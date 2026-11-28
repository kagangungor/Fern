use std::path::PathBuf;
use std::sync::OnceLock;
use tauri::Manager;

pub static ORIGINAL_TRAY_RECT: std::sync::Mutex<Option<windows::Win32::Foundation::RECT>> = std::sync::Mutex::new(None);
static ORIGINAL_SEC_TRAY_RECTS: std::sync::Mutex<Vec<(isize, windows::Win32::Foundation::RECT)>> = std::sync::Mutex::new(Vec::new());
static ORIGINAL_TASKBAR_STATE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);
static TASKBAR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

static TASKBAR_MARKER: OnceLock<PathBuf> = OnceLock::new();
static TASKBAR_WATCHDOG_STARTED: std::sync::Mutex<bool> = std::sync::Mutex::new(false);

/// Point the crash-recovery marker at this user's app config dir (called from setup).
pub fn init_taskbar_marker(app: &tauri::AppHandle) {
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = TASKBAR_MARKER.set(dir.join("taskbar_hidden.flag"));
    }
}

/// Spawns a separate watchdog process that restores Explorer's taskbar if Fern
/// is force-killed or crashes without running exit handlers.
pub fn start_taskbar_watchdog() -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut started = TASKBAR_WATCHDOG_STARTED
        .lock()
        .map_err(|_| std::io::Error::other("taskbar watchdog lock poisoned"))?;
    if *started {
        return Ok(());
    }
    let marker = TASKBAR_MARKER.get().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "taskbar marker path unavailable",
        )
    })?;
    if let Some(dir) = marker.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::process::Command::new(std::env::current_exe()?)
        .arg("--taskbar-watchdog")
        .arg(std::process::id().to_string())
        .arg(marker)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    *started = true;
    Ok(())
}

pub fn run_taskbar_watchdog(parent_pid: u32, marker: PathBuf) {
    use windows::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
    };
    // Only treat a missing PID as an already-exited parent. Access denied or
    // another wait failure must not restore the taskbar over a live Fern.
    unsafe {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, parent_pid) {
            Ok(parent) => {
                let result = WaitForSingleObject(parent, INFINITE);
                let _ = CloseHandle(parent);
                if result != WAIT_OBJECT_0 {
                    return;
                }
            }
            Err(_) if GetLastError() == ERROR_INVALID_PARAMETER => {}
            Err(_) => {
                return;
            }
        }
    }
    let _ = TASKBAR_MARKER.set(marker.clone());
    // Explorer can be restarting at the same time. Retry briefly, but never
    // act on a marker owned by a replacement Fern process.
    for _ in 0..120 {
        if std::fs::read_to_string(&marker)
            .ok()
            .and_then(|contents| parse_marker_pid(&contents))
            != Some(parent_pid)
        {
            return;
        }
        restore_taskbar_after_crash();
        if !marker.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

pub fn taskbar_marker_exists() -> bool {
    TASKBAR_MARKER.get().is_some_and(|p| p.exists())
}

pub fn needs_taskbar_restore() -> bool {
    crate::state::NATIVE_TASKBAR_HIDDEN.load(std::sync::atomic::Ordering::SeqCst)
        || ORIGINAL_TASKBAR_STATE.load(std::sync::atomic::Ordering::SeqCst) != -1
        || taskbar_marker_exists()
}

fn resolve_restore_taskbar_state(orig: i32, always_on_top: bool) -> isize {
    if orig != -1 {
        orig as isize
    } else if always_on_top {
        2
    } else {
        1
    }
}

fn write_taskbar_marker() {
    if let Some(p) = TASKBAR_MARKER.get() {
        let original = ORIGINAL_TASKBAR_STATE.load(std::sync::atomic::Ordering::SeqCst);
        let primary = ORIGINAL_TRAY_RECT.lock().ok().and_then(|g| *g);
        let secondaries = ORIGINAL_SEC_TRAY_RECTS.lock().ok().map(|g| g.clone()).unwrap_or_default();
        let contents = marker_contents(original, std::process::id(), primary, &secondaries);
        let existing = std::fs::read_to_string(p).ok();
        if existing.as_deref() != Some(contents.as_str()) {
            use std::io::Write;
            let _ = std::fs::File::create(p).and_then(|mut file| {
                file.write_all(contents.as_bytes())?;
                file.sync_all()
            });
        }
    }
}

fn rect_is_on_screen(r: &windows::Win32::Foundation::RECT) -> bool {
    r.left > -5000 && r.top > -5000
}

unsafe fn monitor_rects() -> Vec<(windows::Win32::Foundation::RECT, bool)> {
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};

    unsafe extern "system" fn collect(hmon: HMONITOR, _: HDC, _: *mut RECT, lparam: LPARAM) -> windows::core::BOOL {
        let list = &mut *(lparam.0 as *mut Vec<(RECT, bool)>);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            list.push((mi.rcMonitor, mi.dwFlags & 1 != 0));
        }
        windows::core::BOOL(1)
    }
    let mut monitors: Vec<(RECT, bool)> = Vec::new();
    let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut monitors as *mut _ as isize));
    monitors
}

fn taskbar_monitor(monitors: &[(windows::Win32::Foundation::RECT, bool)], primary: bool, taskbar: &windows::Win32::Foundation::RECT) -> Option<windows::Win32::Foundation::RECT> {
    if primary {
        monitors.iter().find(|(_, is_primary)| *is_primary).map(|(r, _)| *r)
    } else {
        let width = taskbar.right - taskbar.left;
        monitors.iter().find(|(r, is_primary)| !*is_primary && r.left == taskbar.left && r.right == taskbar.right)
            .or_else(|| monitors.iter().find(|(r, is_primary)| !*is_primary && r.right - r.left == width))
            .or_else(|| monitors.iter().find(|(_, is_primary)| !*is_primary))
            .map(|(r, _)| *r)
    }
}

unsafe fn park_below_monitor(hwnd: windows::Win32::Foundation::HWND, primary: bool) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER};
    let mut cur = RECT::default();
    let _ = GetWindowRect(hwnd, &mut cur);
    let (x, y) = match taskbar_monitor(&monitor_rects(), primary, &cur) {
        Some(m) => (m.left, m.bottom),
        None => (-10000, -10000),
    };
    let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
}

unsafe fn restore_tray_position(hwnd: windows::Win32::Foundation::HWND, primary: bool, saved: Option<windows::Win32::Foundation::RECT>) {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowRect, SetWindowPos, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER};

    if let Some(r) = saved {
        if rect_is_on_screen(&r) {
            let _ = SetWindowPos(hwnd, None, r.left, r.top, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            return;
        }
    }

    let mut cur = RECT::default();
    let _ = GetWindowRect(hwnd, &mut cur);
    let h = cur.bottom - cur.top;
    let monitors = monitor_rects();
    let inside_a_monitor = monitors.iter().any(|(m, _)| cur.left >= m.left && cur.top >= m.top && cur.right <= m.right && cur.bottom <= m.bottom);
    if inside_a_monitor { return; }

    if let Some(mon) = taskbar_monitor(&monitors, primary, &cur) {
        let _ = SetWindowPos(hwnd, None, mon.left, mon.bottom - h, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// The marker stores the user's own taskbar state from before we forced auto-hide,
/// so a crash can't lose it. Old markers just have "1".
fn marker_contents(
    original_state: i32,
    pid: u32,
    primary: Option<windows::Win32::Foundation::RECT>,
    secondaries: &[(isize, windows::Win32::Foundation::RECT)],
) -> String {
    let mut contents = format!("abstate={original_state}\npid={pid}\n");
    if let Some(rect) = primary {
        contents.push_str(&format!(
            "primary={},{},{},{}\n",
            rect.left, rect.top, rect.right, rect.bottom
        ));
    }
    let mut sorted = secondaries.to_vec();
    sorted.sort_by_key(|(hwnd, _)| *hwnd);
    for (hwnd, rect) in sorted {
        contents.push_str(&format!(
            "secondary={hwnd},{},{},{},{}\n",
            rect.left, rect.top, rect.right, rect.bottom
        ));
    }
    contents
}

fn parse_marker_state(contents: &str) -> Option<i32> {
    contents
        .lines()
        .next()?
        .strip_prefix("abstate=")?
        .parse()
        .ok()
}

fn parse_marker_pid(contents: &str) -> Option<u32> {
    contents
        .lines()
        .find_map(|line| line.strip_prefix("pid=")?.parse().ok())
}

fn parse_rect<'a>(mut parts: impl Iterator<Item = &'a str>) -> Option<windows::Win32::Foundation::RECT> {
    let mut next = || parts.next()?.parse::<i32>().ok();
    let rect = windows::Win32::Foundation::RECT {
        left: next()?,
        top: next()?,
        right: next()?,
        bottom: next()?,
    };
    parts.next().is_none().then_some(rect)
}

fn parse_marker_rect(contents: &str, key: &str) -> Option<windows::Win32::Foundation::RECT> {
    parse_rect(
        contents
            .lines()
            .find_map(|line| line.strip_prefix(key))?
            .split(','),
    )
}

fn parse_marker_secondaries(contents: &str) -> Vec<(isize, windows::Win32::Foundation::RECT)> {
    contents
        .lines()
        .filter_map(|line| {
            let mut parts = line.strip_prefix("secondary=")?.split(',');
            let hwnd = parts.next()?.parse::<isize>().ok()?;
            Some((hwnd, parse_rect(parts)?))
        })
        .collect()
}

/// Restores the taskbar after a session died with it hidden, using the state saved in the marker.
pub fn restore_taskbar_after_crash() {
    let saved = TASKBAR_MARKER
        .get()
        .and_then(|p| std::fs::read_to_string(p).ok());
    if let Some(contents) = saved {
        if let Some(state) = parse_marker_state(&contents) {
            ORIGINAL_TASKBAR_STATE.store(state, std::sync::atomic::Ordering::Relaxed);
        }
        if let Some(rect) = parse_marker_rect(&contents, "primary=") {
            if let Ok(mut guard) = ORIGINAL_TRAY_RECT.lock() {
                if guard.is_none() {
                    *guard = Some(rect);
                }
            }
        }
        let secs = parse_marker_secondaries(&contents);
        if !secs.is_empty() {
            if let Ok(mut guard) = ORIGINAL_SEC_TRAY_RECTS.lock() {
                if guard.is_empty() {
                    *guard = secs;
                }
            }
        }
    }
    set_taskbar_visibility(true, true);
}

pub fn secondary_taskbars() -> Vec<windows::Win32::Foundation::HWND> {
    use windows::Win32::UI::WindowsAndMessaging::FindWindowExA;
    let mut taskbars = Vec::new();
    let mut previous = None;
    loop {
        let next = unsafe {
            FindWindowExA(None, previous, windows::core::PCSTR(c"Shell_SecondaryTrayWnd".as_ptr() as *const u8), windows::core::PCSTR::null())
        };
        match next {
            Ok(hwnd) if !hwnd.0.is_null() => {
                taskbars.push(hwnd);
                previous = Some(hwnd);
            }
            _ => break,
        }
    }
    taskbars
}

pub fn set_taskbar_visibility(visible: bool, always_on_top: bool) {
    use std::sync::atomic::Ordering;

    let _guard = TASKBAR_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    // Startup retries and display handlers can try to hide the taskbar after shutdown restored it.
    if !visible && crate::state::SHUTTING_DOWN.load(Ordering::SeqCst) {
        return;
    }

    if visible {
        crate::state::NATIVE_TASKBAR_HIDDEN.store(false, Ordering::SeqCst);
    }

    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::{FindWindowA, ShowWindow, SW_HIDE, SW_SHOW, GetWindowRect, IsWindowVisible};
        use windows::Win32::UI::Shell::{SHAppBarMessage, APPBARDATA, ABM_SETSTATE, ABM_GETSTATE};

        let tray_class = windows::core::PCSTR(c"Shell_TrayWnd".as_ptr() as *const u8);
        let tray_hwnd_opt = FindWindowA(tray_class, windows::core::PCSTR::null())
            .ok()
            .filter(|h| !h.0.is_null());

        if !visible {
            if start_taskbar_watchdog().is_err() {
                // Never hide the taskbar without the watchdog guarding recovery.
                return;
            }

            // Capture original taskbar state only when hiding, before mutating anything.
            if ORIGINAL_TASKBAR_STATE.load(Ordering::SeqCst) == -1 {
                let mut get_abd = APPBARDATA {
                    cbSize: std::mem::size_of::<APPBARDATA>() as u32,
                    hWnd: tray_hwnd_opt.unwrap_or_default(),
                    ..Default::default()
                };
                let original_state = SHAppBarMessage(ABM_GETSTATE, &mut get_abd);
                ORIGINAL_TASKBAR_STATE.store(original_state as i32, Ordering::SeqCst);
            }

            // Capture primary and secondary tray rects before writing the crash-recovery marker.
            if let Some(tray_hwnd) = tray_hwnd_opt {
                let has_rect = ORIGINAL_TRAY_RECT.lock().map(|g| g.is_some()).unwrap_or(false);
                if !has_rect {
                    let mut rect = windows::Win32::Foundation::RECT::default();
                    let _ = GetWindowRect(tray_hwnd, &mut rect);
                    if rect_is_on_screen(&rect) && IsWindowVisible(tray_hwnd).as_bool() {
                        if let Ok(mut guard) = ORIGINAL_TRAY_RECT.lock() {
                            *guard = Some(rect);
                        }
                    }
                }
            }

            let sec_taskbars = secondary_taskbars();
            for &secondary_tray_hwnd in &sec_taskbars {
                let key = secondary_tray_hwnd.0 as isize;
                let has_sec_rect = ORIGINAL_SEC_TRAY_RECTS
                    .lock()
                    .map(|g| g.iter().any(|(k, _)| *k == key))
                    .unwrap_or(false);
                if !has_sec_rect {
                    let mut rect = windows::Win32::Foundation::RECT::default();
                    let _ = GetWindowRect(secondary_tray_hwnd, &mut rect);
                    if rect_is_on_screen(&rect) && IsWindowVisible(secondary_tray_hwnd).as_bool() {
                        if let Ok(mut guard) = ORIGINAL_SEC_TRAY_RECTS.lock() {
                            guard.push((key, rect));
                        }
                    }
                }
            }

            // Persist the marker before hiding the taskbar so no crash window exists.
            write_taskbar_marker();

            // 1. Set the taskbar state to Auto-hide
            let mut abd = APPBARDATA {
                cbSize: std::mem::size_of::<APPBARDATA>() as u32,
                hWnd: tray_hwnd_opt.unwrap_or_default(),
                lParam: windows::Win32::Foundation::LPARAM(1),
                ..Default::default()
            };
            SHAppBarMessage(ABM_SETSTATE, &mut abd);

            // 2. Hide primary taskbar
            if let Some(tray_hwnd) = tray_hwnd_opt {
                let _ = ShowWindow(tray_hwnd, SW_HIDE);
                // Move it far off-screen to prevent any "thin line" artifacts or flashes
                park_below_monitor(tray_hwnd, true);
            }

            // 3. Hide secondary taskbars (multi-monitor)
            for secondary_tray_hwnd in sec_taskbars {
                let _ = ShowWindow(secondary_tray_hwnd, SW_HIDE);
                park_below_monitor(secondary_tray_hwnd, false);
            }
            return;
        }

        let state_val = resolve_restore_taskbar_state(
            ORIGINAL_TASKBAR_STATE.load(Ordering::SeqCst),
            always_on_top,
        );

        // 1. Restore the taskbar state (original or fallback)
        let mut abd = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            hWnd: tray_hwnd_opt.unwrap_or_default(),
            lParam: windows::Win32::Foundation::LPARAM(state_val),
            ..Default::default()
        };
        SHAppBarMessage(ABM_SETSTATE, &mut abd);

        // 2. Restore visibility of the primary taskbar
        if let Some(tray_hwnd) = tray_hwnd_opt {
            use windows::Win32::UI::WindowsAndMessaging::{GetWindowLongA, SetWindowLongA, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_TRANSPARENT, SetLayeredWindowAttributes, LWA_ALPHA};
            // Revert any lingering WS_EX_LAYERED / WS_EX_TRANSPARENT left by
            // open_system_tray if the user quit before the tray thread cleaned up.
            let ex = GetWindowLongA(tray_hwnd, GWL_EXSTYLE);
            let cleaned = ex & !(WS_EX_LAYERED.0 as i32) & !(WS_EX_TRANSPARENT.0 as i32);
            if cleaned != ex {
                let _ = SetWindowLongA(tray_hwnd, GWL_EXSTYLE, cleaned);
                let _ = SetLayeredWindowAttributes(tray_hwnd, windows::Win32::Foundation::COLORREF(0), 255, LWA_ALPHA);
            }
            let saved = ORIGINAL_TRAY_RECT.lock().ok().and_then(|g| *g);
            restore_tray_position(tray_hwnd, true, saved);
            let _ = ShowWindow(tray_hwnd, SW_SHOW);

            {
                use windows::Win32::Graphics::Gdi::{RedrawWindow, RDW_INVALIDATE, RDW_ALLCHILDREN, RDW_UPDATENOW, RDW_FRAME, RDW_ERASE};
                let _ = RedrawWindow(Some(tray_hwnd), None, None, RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW | RDW_FRAME | RDW_ERASE);

                use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutA, HWND_BROADCAST, WM_SETTINGCHANGE, SMTO_ABORTIFHUNG};
                use windows::Win32::Foundation::{WPARAM, LPARAM};
                let param = c"TraySettings";
                let mut result: usize = 0;
                let _ = SendMessageTimeoutA(HWND_BROADCAST, WM_SETTINGCHANGE, WPARAM(0), LPARAM(param.as_ptr() as isize), SMTO_ABORTIFHUNG, 100, Some(&mut result));
            }
        }

        // 3. Restore visibility of secondary taskbars (multi-monitor)
        for secondary_tray_hwnd in secondary_taskbars() {
            let key = secondary_tray_hwnd.0 as isize;
            let saved = ORIGINAL_SEC_TRAY_RECTS.lock().ok().and_then(|g| g.iter().find(|(k, _)| *k == key).map(|(_, r)| *r));
            restore_tray_position(secondary_tray_hwnd, false, saved);
            let _ = ShowWindow(secondary_tray_hwnd, SW_SHOW);
            {
                use windows::Win32::Graphics::Gdi::{RedrawWindow, RDW_INVALIDATE, RDW_ALLCHILDREN, RDW_UPDATENOW, RDW_FRAME, RDW_ERASE};
                let _ = RedrawWindow(Some(secondary_tray_hwnd), None, None, RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW | RDW_FRAME | RDW_ERASE);
            }
        }

        // Only remove the marker and reset captured state after Shell_TrayWnd was actually restored.
        if tray_hwnd_opt.is_some() {
            if let Some(p) = TASKBAR_MARKER.get() {
                let _ = std::fs::remove_file(p);
            }
            ORIGINAL_TASKBAR_STATE.store(-1, Ordering::SeqCst);
            if let Ok(mut guard) = ORIGINAL_TRAY_RECT.lock() {
                *guard = None;
            }
            if let Ok(mut guard) = ORIGINAL_SEC_TRAY_RECTS.lock() {
                guard.clear();
            }
        }

        {
            use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, WM_SETTINGCHANGE, SMTO_ABORTIFHUNG};
            let setting: Vec<u16> = "TraySettings".encode_utf16().chain(std::iter::once(0)).collect();
            let _ = SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(setting.as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                1000,
                None,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn each_secondary_taskbar_keeps_its_own_monitor() {
        use super::taskbar_monitor;
        use windows::Win32::Foundation::RECT;

        let rect = |left, right, top, bottom| RECT { left, top, right, bottom };
        let primary = rect(0, 1920, 0, 1080);
        let left = rect(-1920, 0, 0, 1080);
        let right = rect(1920, 3840, 0, 1080);
        let monitors = [(primary, true), (left, false), (right, false)];

        let on_right = rect(1920, 3840, 1032, 1080);
        assert_eq!(taskbar_monitor(&monitors, false, &on_right), Some(right));
        let on_left = rect(-1920, 0, 1032, 1080);
        assert_eq!(taskbar_monitor(&monitors, false, &on_left), Some(left));

        let stranded = rect(-10000, -8080, -10000, -9952);
        assert_eq!(taskbar_monitor(&monitors, false, &stranded), Some(left));
        assert_eq!(taskbar_monitor(&monitors, true, &on_right), Some(primary));
    }

    #[test]
    fn roundtrips_taskbar_marker_with_pid_and_rects() {
        use super::{
            marker_contents, parse_marker_pid, parse_marker_rect, parse_marker_secondaries,
            parse_marker_state,
        };
        use windows::Win32::Foundation::RECT;
        let primary = RECT { left: 0, top: 1032, right: 1920, bottom: 1080 };
        let sec = RECT { left: 1920, top: 1032, right: 3840, bottom: 1080 };
        let raw = marker_contents(2, 4242, Some(primary), &[(999, sec)]);
        assert_eq!(parse_marker_state(&raw), Some(2));
        assert_eq!(parse_marker_pid(&raw), Some(4242));
        assert_eq!(parse_marker_rect(&raw, "primary="), Some(primary));
        assert_eq!(parse_marker_secondaries(&raw), vec![(999, sec)]);
    }

    #[test]
    fn resolve_restore_taskbar_state_respects_original_and_fallbacks() {
        use super::resolve_restore_taskbar_state;

        assert_eq!(resolve_restore_taskbar_state(0, true), 0);
        assert_eq!(resolve_restore_taskbar_state(0, false), 0);
        assert_eq!(resolve_restore_taskbar_state(1, true), 1);
        assert_eq!(resolve_restore_taskbar_state(2, false), 2);
        assert_eq!(resolve_restore_taskbar_state(-1, true), 2);
        assert_eq!(resolve_restore_taskbar_state(-1, false), 1);
    }
}
