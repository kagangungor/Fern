use std::sync::atomic::{AtomicU32, Ordering};
use tauri::{AppHandle, Emitter};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
    VIRTUAL_KEY, VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F10, VK_F11, VK_F12, VK_F13, VK_F14,
    VK_F15, VK_F16, VK_F17, VK_F18, VK_F19, VK_F2, VK_F20, VK_F21, VK_F22, VK_F23, VK_F24, VK_F3, VK_F4, VK_F5,
    VK_F6, VK_F7, VK_F8, VK_F9, VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_NEXT,
    VK_OEM_1, VK_OEM_2, VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD,
    VK_OEM_PLUS, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_APP, WM_HOTKEY, WM_USER,
};

const NOTCH_HOTKEY_ID: i32 = 1;
const MIC_MUTE_HOTKEY_ID: i32 = 2;
const RELOAD: u32 = WM_APP + 1;

static THREAD_ID: AtomicU32 = AtomicU32::new(0);

fn key_for(name: &str) -> Option<VIRTUAL_KEY> {
    let lower = name.to_lowercase();
    match lower.as_str() {
        "space" => Some(VK_SPACE),
        "enter" | "return" => Some(VK_RETURN),
        "tab" => Some(VK_TAB),
        "esc" | "escape" => Some(VK_ESCAPE),
        "backspace" => Some(VK_BACK),
        "delete" | "del" => Some(VK_DELETE),
        "insert" | "ins" => Some(VK_INSERT),
        "home" => Some(VK_HOME),
        "end" => Some(VK_END),
        "pageup" | "pgup" => Some(VK_PRIOR),
        "pagedown" | "pgdn" => Some(VK_NEXT),
        "up" => Some(VK_UP),
        "down" => Some(VK_DOWN),
        "left" => Some(VK_LEFT),
        "right" => Some(VK_RIGHT),
        "f1" => Some(VK_F1),
        "f2" => Some(VK_F2),
        "f3" => Some(VK_F3),
        "f4" => Some(VK_F4),
        "f5" => Some(VK_F5),
        "f6" => Some(VK_F6),
        "f7" => Some(VK_F7),
        "f8" => Some(VK_F8),
        "f9" => Some(VK_F9),
        "f10" => Some(VK_F10),
        "f11" => Some(VK_F11),
        "f12" => Some(VK_F12),
        "f13" => Some(VK_F13),
        "f14" => Some(VK_F14),
        "f15" => Some(VK_F15),
        "f16" => Some(VK_F16),
        "f17" => Some(VK_F17),
        "f18" => Some(VK_F18),
        "f19" => Some(VK_F19),
        "f20" => Some(VK_F20),
        "f21" => Some(VK_F21),
        "f22" => Some(VK_F22),
        "f23" => Some(VK_F23),
        "f24" => Some(VK_F24),
        "`" | "~" => Some(VK_OEM_3),
        "-" | "_" => Some(VK_OEM_MINUS),
        "=" | "+" => Some(VK_OEM_PLUS),
        "[" | "{" => Some(VK_OEM_4),
        "]" | "}" => Some(VK_OEM_6),
        "\\" | "|" => Some(VK_OEM_5),
        ";" | ":" => Some(VK_OEM_1),
        "'" | "\"" => Some(VK_OEM_7),
        "," | "<" => Some(VK_OEM_COMMA),
        "." | ">" => Some(VK_OEM_PERIOD),
        "/" | "?" => Some(VK_OEM_2),
        s if s.len() == 1 => {
            let b = s.as_bytes()[0];
            if b.is_ascii_alphanumeric() {
                Some(VIRTUAL_KEY(b.to_ascii_uppercase() as u16))
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn parse_shortcut(spec: &str) -> Option<(Vec<VIRTUAL_KEY>, VIRTUAL_KEY)> {
    let mut modifiers = Vec::new();
    let mut key = None;
    for part in spec.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers.push(VK_LCONTROL),
            "alt" => modifiers.push(VK_LMENU),
            "shift" => modifiers.push(VK_LSHIFT),
            "win" | "windows" | "meta" | "super" => modifiers.push(VK_LWIN),
            _ if key.is_none() => key = Some(key_for(part)),
            _ => return None,
        }
    }
    if key.is_none() && spec.trim_end().ends_with("++") {
        key = Some(Some(VK_OEM_PLUS));
    }
    let Some(Some(key)) = key else { return None };
    Some((modifiers, key))
}

fn apply(app: &AppHandle, reg_notch: &mut bool, reg_mic: &mut bool) {
    unsafe {
        if *reg_notch {
            let _ = UnregisterHotKey(None, NOTCH_HOTKEY_ID);
            *reg_notch = false;
        }
        if *reg_mic {
            let _ = UnregisterHotKey(None, MIC_MUTE_HOTKEY_ID);
            *reg_mic = false;
        }

        // Notch Hotkey
        if let Some(spec) = crate::utils::get_setting_str(app, "fern-notch-hotkey").filter(|s| !s.trim().is_empty()) {
            if let Some((modifiers, key)) = parse_shortcut(&spec) {
                let mut mods = MOD_NOREPEAT;
                for m in modifiers {
                    mods |= match m {
                        VK_LCONTROL => MOD_CONTROL,
                        VK_LMENU => MOD_ALT,
                        VK_LSHIFT => MOD_SHIFT,
                        VK_LWIN => MOD_WIN,
                        _ => HOT_KEY_MODIFIERS(0),
                    };
                }
                match RegisterHotKey(None, NOTCH_HOTKEY_ID, mods, key.0 as u32) {
                    Ok(()) => *reg_notch = true,
                    Err(e) => crate::diagnostics::log(&format!("hotkey: could not register notch \"{spec}\": {e}")),
                }
            } else {
                crate::diagnostics::log(&format!("hotkey: could not read notch \"{spec}\""));
            }
        }

        // Universal Microphone Mute Hotkey (defaults to Ctrl+F1)
        let mic_spec = crate::utils::get_setting_str(app, "fern-mic-mute-hotkey")
            .unwrap_or_else(|| "Ctrl+F1".to_string());
        if !mic_spec.trim().is_empty() {
            if let Some((modifiers, key)) = parse_shortcut(&mic_spec) {
                let mut mods = MOD_NOREPEAT;
                for m in modifiers {
                    mods |= match m {
                        VK_LCONTROL => MOD_CONTROL,
                        VK_LMENU => MOD_ALT,
                        VK_LSHIFT => MOD_SHIFT,
                        VK_LWIN => MOD_WIN,
                        _ => HOT_KEY_MODIFIERS(0),
                    };
                }
                match RegisterHotKey(None, MIC_MUTE_HOTKEY_ID, mods, key.0 as u32) {
                    Ok(()) => *reg_mic = true,
                    Err(e) => crate::diagnostics::log(&format!("hotkey: could not register mic mute \"{mic_spec}\": {e}")),
                }
            } else {
                crate::diagnostics::log(&format!("hotkey: could not read mic mute \"{mic_spec}\""));
            }
        }
    }
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || unsafe {
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
        THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
        let mut reg_notch = false;
        let mut reg_mic = false;
        apply(&app, &mut reg_notch, &mut reg_mic);
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            match msg.message {
                WM_HOTKEY => {
                    let id = msg.wParam.0 as i32;
                    if id == NOTCH_HOTKEY_ID {
                        let _ = app.emit("notch-hotkey-toggle", ());
                    } else if id == MIC_MUTE_HOTKEY_ID {
                        let _ = crate::privacy_hud::toggle_microphone_mute_internal(&app);
                    }
                }
                RELOAD => apply(&app, &mut reg_notch, &mut reg_mic),
                _ => {}
            }
        }
    });
}

pub fn reload() {
    let id = THREAD_ID.load(Ordering::SeqCst);
    if id != 0 {
        unsafe {
            let _ = PostThreadMessageW(id, RELOAD, WPARAM(0), LPARAM(0));
        }
    }
}
