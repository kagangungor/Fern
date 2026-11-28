//! DWM Direct-Flip Protection for fullscreen games and apps.
//!
//! Transparent `WS_EX_TOPMOST` windows (notch, dock, overlay) force Windows DWM out of
//! Independent Flip (Direct-Flip) and back into desktop compositing even when their CSS is
//! invisible and click-through, adding frame latency and breaking VRR/G-Sync.
//!
//! When enabled (default ON), this module natively hides eligible windows via
//! `ShowWindow(SW_HIDE)` while a real fullscreen window covers their monitor, and restores them
//! with `ShowWindow(SW_SHOWNOACTIVATE)` + `re_assert_topmost` the moment fullscreen ends (or
//! when the user turns the protection off).
//!
//! Safety invariants:
//! - Only Smart and Peek panels are hidden; Fixed panels stay visible by design.
//! - Only windows that were actually visible before hiding are tracked and restored, so a dock
//!   or notch disabled by the user is never unexpectedly shown.
//! - Active keyboard focus (e.g. Notch AI Chat), open dock menus, active dock drags, and the
//!   startup splash screen are never interrupted.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use tauri::{AppHandle, Manager, WebviewWindow};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    IsWindowVisible, ShowWindow, SW_HIDE, SW_SHOWNOACTIVATE,
};

pub const SETTING_KEY: &str = "fern-direct-flip-protection";

fn hidden_windows() -> &'static Mutex<HashSet<String>> {
    static CELL: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Default ON unless explicitly set to `"false"`.
pub fn is_enabled(app: &AppHandle) -> bool {
    crate::utils::get_setting_str(app, SETTING_KEY).as_deref() != Some("false")
}

/// Returns true if this window is currently natively hidden by Direct-Flip protection.
pub fn is_hidden(label: &str) -> bool {
    hidden_windows()
        .lock()
        .ok()
        .is_some_and(|set| set.contains(label))
}

/// Removes a window label from the hidden set (e.g. when a window is closed or explicitly shown).
pub fn clear_hidden(label: &str) {
    if let Ok(mut set) = hidden_windows().lock() {
        set.remove(label);
    }
}

/// Pure predicate deciding whether a notch or dock window should be natively hidden.
pub(crate) fn should_hide_panel(
    enabled: bool,
    monitor_is_fullscreen: bool,
    mode: &str,
    has_keyboard_focus: bool,
    busy: bool,
) -> bool {
    enabled && monitor_is_fullscreen && mode != "fixed" && !has_keyboard_focus && !busy
}

/// Pure predicate deciding whether the HUD overlay window should be natively hidden.
pub(crate) fn should_hide_overlay(
    enabled: bool,
    primary_monitor_is_fullscreen: bool,
    in_splash: bool,
) -> bool {
    enabled && primary_monitor_is_fullscreen && !in_splash
}

/// Applies the desired native hidden state for a single window.
///
/// Only hides windows that are currently visible (`IsWindowVisible == true`) so disabled panels
/// are never touched. Only restores windows that this module previously hid.
pub fn sync_window(win: &WebviewWindow, label: &str, should_hide: bool) {
    let already_hidden = is_hidden(label);
    if should_hide == already_hidden {
        return;
    }

    let Ok(hwnd) = win.hwnd() else { return };
    if should_hide {
        let visible_now = unsafe { IsWindowVisible(hwnd).as_bool() };
        if !visible_now {
            // Window is already hidden by something else (e.g. dock/notch disabled); do not claim it.
            return;
        }
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        if let Ok(mut set) = hidden_windows().lock() {
            set.insert(label.to_string());
        }
        crate::diagnostics::log(&format!(
            "direct_flip: native SW_HIDE on '{label}'"
        ));
    } else {
        // Restore without stealing focus from the user's active window / game.
        if let Ok(mut set) = hidden_windows().lock() {
            set.remove(label);
        }
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        crate::utils::re_assert_topmost(hwnd);
        crate::diagnostics::log(&format!(
            "direct_flip: native SW_SHOWNOACTIVATE on '{label}'"
        ));
    }
}

/// Restores all windows currently hidden by Direct-Flip protection (e.g. when the user disables
/// the setting or during cleanup).
pub fn restore_all(app: &AppHandle) {
    let labels: Vec<String> = hidden_windows()
        .lock()
        .ok()
        .map(|mut set| set.drain().collect())
        .unwrap_or_default();

    for label in labels {
        if let Some(win) = app.get_webview_window(&label) {
            if let Ok(hwnd) = win.hwnd() {
                restore_hwnd(hwnd, &label);
            }
        }
    }
}

fn restore_hwnd(hwnd: HWND, label: &str) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }
    crate::utils::re_assert_topmost(hwnd);
    crate::diagnostics::log(&format!(
        "direct_flip: restored '{label}' via SW_SHOWNOACTIVATE"
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_smart_and_peek_in_fullscreen_when_enabled() {
        assert!(should_hide_panel(true, true, "smart", false, false));
        assert!(should_hide_panel(true, true, "peek", false, false));
    }

    #[test]
    fn never_hides_fixed_mode_panels() {
        assert!(!should_hide_panel(true, true, "fixed", false, false));
    }

    #[test]
    fn never_hides_when_disabled_or_not_fullscreen() {
        assert!(!should_hide_panel(false, true, "smart", false, false));
        assert!(!should_hide_panel(true, false, "smart", false, false));
    }

    #[test]
    fn respects_keyboard_focus_and_busy_states() {
        assert!(!should_hide_panel(true, true, "smart", true, false));
        assert!(!should_hide_panel(true, true, "smart", false, true));
    }

    #[test]
    fn overlay_hides_in_fullscreen_except_during_splash() {
        assert!(should_hide_overlay(true, true, false));
        assert!(!should_hide_overlay(true, true, true));
        assert!(!should_hide_overlay(true, false, false));
        assert!(!should_hide_overlay(false, true, false));
    }
}
