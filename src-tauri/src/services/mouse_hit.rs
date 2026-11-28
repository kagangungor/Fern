use std::sync::atomic::{AtomicBool, AtomicI64, AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM, CloseHandle};
use windows::core::BOOL;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW, WH_MOUSE_LL, MSLLHOOKSTRUCT, WM_MOUSEMOVE, WM_LBUTTONDOWN, SetWindowsHookExW, CallNextHookEx};
use windows::Win32::Graphics::Dwm::DwmGetWindowAttribute;
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_NAME_WIN32};
use crate::types::*;
use crate::state::*;
use crate::utils::*;
use super::appbar::{notch_target_top, position_notch_window};

pub(super) static MH_LAST_MAIN_IGNORE: AtomicI32 = AtomicI32::new(-1);
pub(super) static MH_LAST_DOCK_IGNORE: AtomicI32 = AtomicI32::new(-1);
pub(super) static MH_LAST_OV_IGNORE: AtomicI32 = AtomicI32::new(-1);
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
pub(super) static CAPTURE_UI_ACTIVE: AtomicBool = AtomicBool::new(false);
pub(super) static CAPTURE_RECHECK: AtomicBool = AtomicBool::new(true);
pub(super) static CAPTURE_LAST_SCAN_MS: AtomicI64 = AtomicI64::new(0);

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

pub(super) fn is_capture_ui_present() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    let mut found = false;
    unsafe {
        let _ = EnumWindows(Some(capture_ui_enum_proc), LPARAM(&mut found as *mut bool as isize));
    }
    found
}

pub(super) fn apply_capture_ui_state(app: &AppHandle, active: bool) {
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
                if crate::direct_flip::is_hidden("overlay") {
                    // Natively hidden for DWM Direct-Flip; avoid touching Z-order or ex-style.
                } else if fg_fs {
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

pub(super) fn monitor_rect_for_hwnd(hwnd: HWND) -> Option<(i32, i32, i32, i32)> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicI64;
    use std::time::Duration;

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
