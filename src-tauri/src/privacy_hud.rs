use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, IMMDeviceEnumerator, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
    REG_VALUE_TYPE,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PrivacyStatus {
    pub mic_active: bool,
    pub cam_active: bool,
    pub active_apps: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MicMuteStatus {
    pub is_muted: bool,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn get_default_capture_mute() -> Result<bool, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eCapture, eConsole)
            .map_err(|e| e.to_string())?;
        let endpoint: IAudioEndpointVolume =
            device.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        let is_muted = endpoint
            .GetMute()
            .map(|m| m.as_bool())
            .map_err(|e| e.to_string())?;
        Ok(is_muted)
    }
}

pub fn set_default_capture_mute(muted: bool) -> Result<bool, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let mut success = false;
        for role in [eConsole, eCommunications] {
            if let Ok(device) = enumerator.GetDefaultAudioEndpoint(eCapture, role) {
                if let Ok(endpoint) = device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    let _ = endpoint.SetMute(muted, std::ptr::null());
                    success = true;
                }
            }
        }
        if !success {
            return Err("No active capture device found".to_string());
        }
        Ok(muted)
    }
}

pub fn toggle_default_capture_mute() -> Result<bool, String> {
    let current = get_default_capture_mute().unwrap_or(false);
    let target = !current;
    set_default_capture_mute(target)?;
    Ok(target)
}

pub fn toggle_microphone_mute_internal(app: &AppHandle) -> Result<bool, String> {
    let new_mute = toggle_default_capture_mute()?;
    let _ = app.emit("mic-mute-changed", MicMuteStatus { is_muted: new_mute });
    Ok(new_mute)
}

fn check_consent_store_path(subpath: &str, is_active: &mut bool, active_apps: &mut Vec<String>) {
    unsafe {
        let subpath_w = wide(subpath);
        let mut root_key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subpath_w.as_ptr()),
            Some(0),
            KEY_READ,
            &mut root_key,
        )
        .is_err()
        {
            return;
        }

        let mut index = 0u32;
        let mut key_name_buf = [0u16; 512];
        loop {
            let mut name_len = key_name_buf.len() as u32;
            if RegEnumKeyExW(
                root_key,
                index,
                Some(PWSTR(key_name_buf.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                None,
                None,
            )
            .is_err()
            {
                break;
            }
            index += 1;

            let child_name = String::from_utf16_lossy(&key_name_buf[..name_len as usize]);
            if child_name.eq_ignore_ascii_case("NonPackaged") {
                continue;
            }

            let child_path = format!(r"{}\{}", subpath, child_name);
            let child_path_w = wide(&child_path);
            let mut child_key = HKEY::default();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(child_path_w.as_ptr()),
                Some(0),
                KEY_READ,
                &mut child_key,
            )
            .is_ok()
            {
                let stop_w = wide("LastUsedTimeStop");
                let mut stop_val: u64 = 0;
                let mut stop_len = std::mem::size_of::<u64>() as u32;
                let mut val_type = REG_VALUE_TYPE(0);

                let stop_res = RegQueryValueExW(
                    child_key,
                    PCWSTR(stop_w.as_ptr()),
                    None,
                    Some(&mut val_type),
                    Some(&mut stop_val as *mut _ as *mut u8),
                    Some(&mut stop_len),
                );

                let start_w = wide("LastUsedTimeStart");
                let mut start_val: u64 = 0;
                let mut start_len = std::mem::size_of::<u64>() as u32;

                let start_res = RegQueryValueExW(
                    child_key,
                    PCWSTR(start_w.as_ptr()),
                    None,
                    Some(&mut val_type),
                    Some(&mut start_val as *mut _ as *mut u8),
                    Some(&mut start_len),
                );

                let _ = RegCloseKey(child_key);

                if stop_res.is_ok() && start_res.is_ok() {
                    // Windows ConsentStore:
                    // When actively recording: LastUsedTimeStop is 0 or Start is greater than Stop
                    if (stop_val == 0 && start_val > 0) || (start_val > stop_val) {
                        *is_active = true;
                        let display_name = child_name
                            .split('#')
                            .last()
                            .unwrap_or(&child_name)
                            .trim_end_matches(".exe")
                            .to_string();
                        if !active_apps.contains(&display_name) {
                            active_apps.push(display_name);
                        }
                    }
                }
            }
        }
        let _ = RegCloseKey(root_key);
    }
}

fn is_capability_active(capability: &str, active_apps: &mut Vec<String>) -> bool {
    let base_path = format!(
        r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{}",
        capability
    );
    let mut is_active = false;
    check_consent_store_path(&base_path, &mut is_active, active_apps);
    let non_packaged_path = format!(r"{}\NonPackaged", base_path);
    check_consent_store_path(&non_packaged_path, &mut is_active, active_apps);
    is_active
}

pub fn check_privacy_status() -> PrivacyStatus {
    let mut active_apps = Vec::new();
    let mic_active = is_capability_active("microphone", &mut active_apps);
    let cam_active = is_capability_active("webcam", &mut active_apps);
    PrivacyStatus {
        mic_active,
        cam_active,
        active_apps,
    }
}

pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        let mut last_privacy = PrivacyStatus::default();
        let mut last_mute = get_default_capture_mute().unwrap_or(false);

        // Initial broadcast
        let _ = app.emit("mic-mute-changed", MicMuteStatus { is_muted: last_mute });
        let _ = app.emit("privacy-status-changed", &last_privacy);

        loop {
            std::thread::sleep(std::time::Duration::from_millis(750));

            let current_privacy = check_privacy_status();
            if current_privacy != last_privacy {
                last_privacy = current_privacy.clone();
                let _ = app.emit("privacy-status-changed", &current_privacy);
            }

            if let Ok(current_mute) = get_default_capture_mute() {
                if current_mute != last_mute {
                    last_mute = current_mute;
                    let _ = app.emit("mic-mute-changed", MicMuteStatus { is_muted: current_mute });
                }
            }
        }
    });
}

#[tauri::command]
pub async fn get_microphone_mute() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(get_default_capture_mute)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_microphone_mute(app: AppHandle, muted: bool) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_default_capture_mute(muted)?;
        let _ = app.emit("mic-mute-changed", MicMuteStatus { is_muted: muted });
        Ok(muted)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn toggle_microphone_mute(app: AppHandle) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || toggle_microphone_mute_internal(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_privacy_status() -> Result<PrivacyStatus, String> {
    tauri::async_runtime::spawn_blocking(check_privacy_status)
        .await
        .map_err(|e| e.to_string())
}
