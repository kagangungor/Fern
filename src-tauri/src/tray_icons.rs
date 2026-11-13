use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationExpandCollapsePattern,
    IUIAutomationInvokePattern, IUIAutomationLegacyIAccessiblePattern, IUIAutomationTogglePattern,
    TreeScope_Descendants, UIA_ExpandCollapsePatternId, UIA_InvokePatternId,
    UIA_LegacyIAccessiblePatternId, UIA_TogglePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, FindWindowA, FindWindowExA, GetAncestor, GetClassNameA, GetCursorPos, GetForegroundWindow,
    GetShellWindow, GetWindowLongA, GetWindowRect, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    SendMessageA, SetForegroundWindow, SetLayeredWindowAttributes, SetWindowLongA, SetWindowPos, ShowWindow,
    GA_ROOT, GWL_EXSTYLE, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SW_HIDE, SW_SHOWNA, WM_CHANGEUISTATE, WS_EX_LAYERED,
};
use windows::core::{Interface, BOOL};

static AUTOMATION_LOCK: Mutex<()> = Mutex::new(());
static FLYOUT_OPEN: AtomicBool = AtomicBool::new(false);
static OPEN_COUNT: AtomicU64 = AtomicU64::new(0);
static PREVIOUS_FOREGROUND: AtomicIsize = AtomicIsize::new(0);
static OPENING: AtomicBool = AtomicBool::new(false);

fn automation() -> windows::core::Result<IUIAutomation> {
    unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
}

fn find_window(class: &std::ffi::CStr) -> windows::core::Result<HWND> {
    unsafe { FindWindowA(windows::core::PCSTR(class.as_ptr() as *const u8), windows::core::PCSTR::null()) }
}

fn foreground_description() -> String {
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        return "none".into();
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
    format!("{} (pid {pid})", class_name(fg))
}

fn flyout_owns_focus() -> bool {
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        return true;
    }
    let root = unsafe { GetAncestor(fg, GA_ROOT) };
    let root = if root.0.is_null() { fg } else { root };
    matches!(
        class_name(root).as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "TopLevelWindowForOverflowXamlIsland" | "NotifyIconOverflowWindow"
    )
}

fn find_overflow_window() -> Option<HWND> {
    find_window(c"TopLevelWindowForOverflowXamlIsland")
        .or_else(|_| find_window(c"NotifyIconOverflowWindow"))
        .ok()
}

fn invoke_element(el: &IUIAutomationElement) -> bool {
    unsafe {
        if let Ok(pattern) = el.GetCurrentPattern(UIA_InvokePatternId) {
            if let Ok(invoke_pattern) = pattern.cast::<IUIAutomationInvokePattern>() {
                if invoke_pattern.Invoke().is_ok() {
                    return true;
                }
            }
        }
        if let Ok(pattern) = el.GetCurrentPattern(UIA_TogglePatternId) {
            if let Ok(toggle_pattern) = pattern.cast::<IUIAutomationTogglePattern>() {
                if toggle_pattern.Toggle().is_ok() {
                    return true;
                }
            }
        }
        if let Ok(pattern) = el.GetCurrentPattern(UIA_ExpandCollapsePatternId) {
            if let Ok(expand_pattern) = pattern.cast::<IUIAutomationExpandCollapsePattern>() {
                if expand_pattern.Expand().is_ok() {
                    return true;
                }
            }
        }
        if let Ok(pattern) = el.GetCurrentPattern(UIA_LegacyIAccessiblePatternId) {
            if let Ok(legacy) = pattern.cast::<IUIAutomationLegacyIAccessiblePattern>() {
                if legacy.DoDefaultAction().is_ok() {
                    return true;
                }
            }
        }
        false
    }
}

fn find_show_hidden_icons_button(automation: &IUIAutomation, tray_hwnd: HWND) -> Option<IUIAutomationElement> {
    unsafe {
        let mut candidates = Vec::new();

        // 1. Windows 11 modern taskbar hosts the buttons in DesktopWindowContentBridge
        let bridge = FindWindowExA(
            Some(tray_hwnd),
            None,
            windows::core::PCSTR(c"Windows.UI.Composition.DesktopWindowContentBridge".as_ptr() as *const u8),
            windows::core::PCSTR::null(),
        ).unwrap_or_default();
        if !bridge.0.is_null() {
            candidates.push(bridge);
        }

        // 2. Windows 10 legacy taskbar hosts it in TrayNotifyWnd
        let notify = FindWindowExA(
            Some(tray_hwnd),
            None,
            windows::core::PCSTR(c"TrayNotifyWnd".as_ptr() as *const u8),
            windows::core::PCSTR::null(),
        ).unwrap_or_default();
        if !notify.0.is_null() {
            candidates.push(notify);
        }

        // 3. Fallback to tray_hwnd itself
        candidates.push(tray_hwnd);

        for hwnd in candidates {
            if let Ok(root) = automation.ElementFromHandle(hwnd) {
                if let Ok(condition) = automation.CreateTrueCondition() {
                    if let Ok(all) = root.FindAll(TreeScope_Descendants, &condition) {
                        for i in 0..all.Length().unwrap_or(0) {
                            if let Ok(el) = all.GetElement(i) {
                                let automation_id = el.CurrentAutomationId().map(|s| s.to_string()).unwrap_or_default();
                                if automation_id == "NotificationChevron" || automation_id == "OverflowButton" {
                                    return Some(el);
                                }
                                let name = el.CurrentName().map(|s| s.to_string()).unwrap_or_default();
                                let lower: String = name.chars().flat_map(|c| c.to_lowercase()).collect();
                                if lower.contains("hidden") || lower.contains("gizli") || lower.contains("chevron") || lower.contains("overflow") {
                                    return Some(el);
                                }
                                if automation_id == "SystemTrayIcon" && (lower.contains("simgeler") || lower.contains("icons") || lower.contains("bildirim") || lower.contains("notification")) {
                                    return Some(el);
                                }
                            }
                        }
                    }
                }
            }
        }
        None
    }
}

fn send_keys(keys: &[(windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY, bool)]) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP};
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|&(vk, up)| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() }, time: 0, dwExtraInfo: 0 },
            },
        })
        .collect();
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u8; 64];
    let len = unsafe { GetClassNameA(hwnd, &mut buf) }.max(0) as usize;
    String::from_utf8_lossy(&buf[..len]).into_owned()
}

fn taskbar_has_focus() -> bool {
    let fg = unsafe { GetForegroundWindow() };
    !fg.0.is_null() && matches!(class_name(fg).as_str(), "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

fn dismiss_taskbar_tooltips() {
    unsafe extern "system" fn enum_tooltips(hwnd: HWND, _: windows::Win32::Foundation::LPARAM) -> BOOL {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        if let Ok(tray) = find_window(c"Shell_TrayWnd") {
            let mut tray_pid = 0u32;
            unsafe { GetWindowThreadProcessId(tray, Some(&mut tray_pid)) };
            if pid != 0 && pid == tray_pid {
                let name = class_name(hwnd);
                if name == "tooltips_class32"
                    || name.contains("PopupWindowSiteBridge")
                    || name.contains("Xaml_WindowedPopupClass")
                {
                    if unsafe { IsWindowVisible(hwnd) }.as_bool() {
                        unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
                    }
                }
            }
        }
        BOOL(1)
    }
    unsafe {
        let _ = EnumWindows(Some(enum_tooltips), windows::Win32::Foundation::LPARAM(0));
    }
}

fn restore_foreground(target: HWND) -> bool {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    unsafe {
        if !IsWindow(Some(target)).as_bool() {
            return false;
        }
        if SetForegroundWindow(target).as_bool() {
            return true;
        }
        let ours = GetCurrentThreadId();
        let theirs = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let attached = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool();
        let restored = SetForegroundWindow(target).as_bool();
        if attached {
            let _ = AttachThreadInput(ours, theirs, false);
        }
        restored
    }
}

fn leave_taskbar_focus(flyout: HWND, open_count: u64) {
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        if !unsafe { IsWindowVisible(flyout) }.as_bool() {
            break;
        }
    }
    for _ in 0..20 {
        if OPEN_COUNT.load(Ordering::SeqCst) != open_count {
            return;
        }
        dismiss_taskbar_tooltips();
        if taskbar_has_focus() {
            let mut previous = HWND(PREVIOUS_FOREGROUND.load(Ordering::SeqCst) as *mut _);
            if previous.0.is_null() || !unsafe { IsWindow(Some(previous)) }.as_bool() {
                previous = unsafe { GetShellWindow() };
            }
            let restored = restore_foreground(previous);
            crate::diagnostics::log(&format!("tray: taskbar kept focus after closing, gave focus back to {}: {restored}", class_name(previous)));
            dismiss_taskbar_tooltips();
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn open_with_shortcut(anchor: Option<(i32, i32)>) -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_B, VK_LWIN, VK_RETURN};
    send_keys(&[(VK_LWIN, false), (VK_B, false), (VK_B, true), (VK_LWIN, true)]);

    let mut focused = false;
    for _ in 0..60 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        if taskbar_has_focus() {
            focused = true;
            break;
        }
    }
    if !focused {
        crate::diagnostics::log("tray: taskbar never took focus, not sending Return");
        return false;
    }

    let (stop_tx, stop_rx) = std::sync::mpsc::channel();
    let placer = std::thread::spawn(move || {
        for _ in 0..200 {
            if stop_rx.try_recv().is_ok() {
                break;
            }
            if let Some(flyout) = find_overflow_window() {
                if unsafe { IsWindowVisible(flyout) }.as_bool() {
                    place_flyout(flyout, anchor);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });

    send_keys(&[(VK_RETURN, false), (VK_RETURN, true)]);

    let mut opened = false;
    for _ in 0..100 {
        if let Some(flyout) = find_overflow_window() {
            if unsafe { IsWindowVisible(flyout) }.as_bool() {
                place_flyout(flyout, anchor);
                opened = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let _ = stop_tx.send(());
    let _ = placer.join();
    opened
}

fn place_flyout(flyout: HWND, anchor: Option<(i32, i32)>) {
    unsafe {
        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        let (ax, ay) = anchor.unwrap_or((cursor.x, cursor.y - 4));
        let point = POINT { x: ax, y: ay };
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST), &mut mi);
        let work = mi.rcWork;

        let mut rect = RECT::default();
        let _ = GetWindowRect(flyout, &mut rect);
        if rect.right - rect.left <= 50 {
            for _ in 0..10 {
                std::thread::sleep(std::time::Duration::from_millis(1));
                let _ = GetWindowRect(flyout, &mut rect);
                if rect.right - rect.left > 50 { break; }
            }
        }
        let (w, h) = if rect.right - rect.left > 50 {
            (rect.right - rect.left, rect.bottom - rect.top)
        } else {
            (280, 220)
        };

        let x = (ax - w / 2).clamp(work.left + 8, (work.right - w - 8).max(work.left + 8));
        let below = ay + 10;
        let y = if below + h <= work.bottom - 8 { below } else { (ay - h - 10).max(work.top + 8) };
        let _ = SetWindowPos(flyout, Some(HWND_TOPMOST), x, y, 0, 0, SWP_NOSIZE | SWP_SHOWWINDOW);
        let _ = SetForegroundWindow(flyout);
        // Suppress any keyboard focus cue on the first item
        let _ = SendMessageA(flyout, WM_CHANGEUISTATE, windows::Win32::Foundation::WPARAM(0x00010001), windows::Win32::Foundation::LPARAM(0));
    }
}

pub fn warm_up() {
    let _com = crate::utils::ComGuard::mta();
    let Ok(_guard) = AUTOMATION_LOCK.try_lock() else { return };
    let (Ok(tray), Ok(auto)) = (find_window(c"Shell_TrayWnd"), automation()) else { return };
    let _ = find_show_hidden_icons_button(&auto, tray);
}

struct OpeningFlag;

impl Drop for OpeningFlag {
    fn drop(&mut self) {
        OPENING.store(false, Ordering::SeqCst);
    }
}

pub fn open_native_tray(app: Option<AppHandle>, anchor: Option<(i32, i32)>) {
    let _com = crate::utils::ComGuard::mta();

    crate::diagnostics::log(&format!("tray: open requested, anchor={anchor:?}, flyout_open={}", FLYOUT_OPEN.load(Ordering::SeqCst)));
    if let Some(flyout) = find_overflow_window().filter(|&w| unsafe { IsWindowVisible(w) }.as_bool()) {
        use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
        send_keys(&[(VK_ESCAPE, false), (VK_ESCAPE, true)]);
        dismiss_taskbar_tooltips();
        leave_taskbar_focus(flyout, OPEN_COUNT.load(Ordering::SeqCst));
        FLYOUT_OPEN.store(false, Ordering::SeqCst);
        if let Some(ref a) = app {
            let _ = a.emit("tray-flyout-state", false);
        }
        return;
    }
    if OPENING.swap(true, Ordering::SeqCst) {
        crate::diagnostics::log("tray: another open is still running");
        return;
    }
    let _opening = OpeningFlag;
    OPEN_COUNT.fetch_add(1, Ordering::SeqCst);
    let previous = unsafe { GetForegroundWindow() };
    let usable = !previous.0.is_null() && !taskbar_has_focus() && find_overflow_window() != Some(previous);
    PREVIOUS_FOREGROUND.store(if usable { previous.0 as isize } else { 0 }, Ordering::SeqCst);

    let waiting = std::time::Instant::now();
    let _guard = loop {
        if let Ok(guard) = AUTOMATION_LOCK.try_lock() {
            break guard;
        }
        if waiting.elapsed().as_millis() > 2000 {
            crate::diagnostics::log("tray: another tray action is still running");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    if waiting.elapsed().as_millis() >= 20 {
        crate::diagnostics::log(&format!("tray: waited {}ms for another tray action to finish", waiting.elapsed().as_millis()));
    }

    let Ok(tray) = find_window(c"Shell_TrayWnd") else {
        crate::diagnostics::log("tray: Shell_TrayWnd not found");
        return;
    };

    let hidden = !unsafe { IsWindowVisible(tray) }.as_bool();
    let original_ex = unsafe { GetWindowLongA(tray, GWL_EXSTYLE) };
    if hidden {
        unsafe {
            SetWindowLongA(tray, GWL_EXSTYLE, original_ex | WS_EX_LAYERED.0 as i32);
            let _ = SetLayeredWindowAttributes(tray, windows::Win32::Foundation::COLORREF(0), 0, LWA_ALPHA);
            let _ = ShowWindow(tray, SW_SHOWNA);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let mut opened = false;
    if let Ok(auto) = automation() {
        let button = find_show_hidden_icons_button(&auto, tray);
        if button.is_none() {
            crate::diagnostics::log("tray: hidden icons button not found in tray or children");
        }
        if let Some(button) = button {
            crate::diagnostics::log("tray: invoking hidden icons button via UIA");
            let (stop_tx, stop_rx) = std::sync::mpsc::channel();
            let placer = std::thread::spawn(move || {
                for _ in 0..200 {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                    if let Some(flyout) = find_overflow_window() {
                        if unsafe { IsWindowVisible(flyout) }.as_bool() {
                            place_flyout(flyout, anchor);
                            break;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            });

            if invoke_element(&button) {
                for _ in 0..60 {
                    if let Some(flyout) = find_overflow_window() {
                        if unsafe { IsWindowVisible(flyout) }.as_bool() {
                            place_flyout(flyout, anchor);
                            opened = true;
                            break;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
            let _ = stop_tx.send(());
            let _ = placer.join();
        }
    }

    if !opened {
        opened = open_with_shortcut(anchor);
        crate::diagnostics::log(&format!("tray: open_with_shortcut opened={opened}"));
    }

    // Release UIA/automation lock before waiting for the user to close the flyout
    drop(_guard);

    if opened {
        FLYOUT_OPEN.store(true, Ordering::SeqCst);
        if let Some(ref a) = app {
            let _ = a.emit("tray-flyout-state", true);
        }
        crate::diagnostics::log("tray: flyout opened and placed");
        std::thread::sleep(std::time::Duration::from_millis(150));
        let mut last_fg = String::new();
        for _ in 0..2400 {
            let fg = foreground_description();
            if fg != last_fg {
                crate::diagnostics::log(&format!("tray: flyout open, foreground is now {fg}"));
                last_fg = fg;
            }
            match find_overflow_window().filter(|&w| unsafe { IsWindowVisible(w) }.as_bool()) {
                Some(flyout) => unsafe {
                    if flyout_owns_focus() {
                        let _ = SetWindowPos(flyout, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                    }
                },
                None => {
                    crate::diagnostics::log("tray: flyout closed");
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        FLYOUT_OPEN.store(false, Ordering::SeqCst);
        if let Some(ref a) = app {
            let _ = a.emit("tray-flyout-state", false);
        }
        dismiss_taskbar_tooltips();
        if let Some(flyout) = find_overflow_window() {
            leave_taskbar_focus(flyout, OPEN_COUNT.load(Ordering::SeqCst));
        }
    }

    if hidden {
        unsafe {
            let _ = ShowWindow(tray, SW_HIDE);
            SetWindowLongA(tray, GWL_EXSTYLE, original_ex);
        }
    }
}

fn all_taskbars() -> Vec<HWND> {
    let mut taskbars = Vec::new();
    if let Ok(primary) = find_window(c"Shell_TrayWnd") {
        taskbars.push(primary);
    }
    taskbars.extend(crate::utils::secondary_taskbars());
    taskbars
}

pub fn schedule_taskbar_layout_refresh() {
    static SCHEDULED: AtomicBool = AtomicBool::new(false);
    struct ScheduledReset;
    impl Drop for ScheduledReset {
        fn drop(&mut self) {
            SCHEDULED.store(false, Ordering::SeqCst);
        }
    }
    if SCHEDULED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let _reset = ScheduledReset;
        std::thread::sleep(std::time::Duration::from_millis(2500));
        if FLYOUT_OPEN.load(Ordering::SeqCst) {
            return;
        }
        let Ok(_guard) = AUTOMATION_LOCK.try_lock() else { return };
        if !crate::state::NATIVE_TASKBAR_HIDDEN.swap(false, Ordering::SeqCst) {
            return;
        }

        let taskbars = all_taskbars();
        let originals: Vec<(HWND, i32)> = taskbars
            .iter()
            .map(|&hwnd| (hwnd, unsafe { GetWindowLongA(hwnd, GWL_EXSTYLE) }))
            .collect();
        for &(hwnd, ex) in &originals {
            unsafe {
                SetWindowLongA(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as i32);
                let _ = SetLayeredWindowAttributes(hwnd, windows::Win32::Foundation::COLORREF(0), 0, LWA_ALPHA);
                let _ = ShowWindow(hwnd, SW_SHOWNA);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        for &(hwnd, ex) in &originals {
            unsafe {
                let _ = ShowWindow(hwnd, SW_HIDE);
                SetWindowLongA(hwnd, GWL_EXSTYLE, ex);
            }
        }
        crate::utils::set_taskbar_visibility(false, false);
        crate::state::NATIVE_TASKBAR_HIDDEN.store(true, Ordering::SeqCst);
    });
}
