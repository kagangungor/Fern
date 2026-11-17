use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, AudioSessionStateActive, IAudioSessionControl2,
    IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, REG_VALUE_TYPE,
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
    let _com = crate::utils::ComGuard::mta();
    unsafe {
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
    let _com = crate::utils::ComGuard::mta();
    unsafe {
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

fn is_consent_timestamps_active(start: u64, stop: Option<u64>) -> bool {
    if start == 0 {
        return false;
    }
    match stop {
        None => true,
        Some(s) => s == 0 || start > s,
    }
}

fn check_consent_store_path(
    root: HKEY,
    subpath: &str,
    is_active: &mut bool,
    active_apps: &mut Vec<String>,
) {
    unsafe {
        let subpath_w = wide(subpath);
        let mut root_key = HKEY::default();
        if RegOpenKeyExW(
            root,
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
                root,
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

                if start_res.is_ok() {
                    let stop_opt = if stop_res.is_ok() { Some(stop_val) } else { None };
                    if is_consent_timestamps_active(start_val, stop_opt) {
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
    let non_packaged_path = format!(r"{}\NonPackaged", base_path);
    let mut is_active = false;
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        check_consent_store_path(root, &base_path, &mut is_active, active_apps);
        check_consent_store_path(root, &non_packaged_path, &mut is_active, active_apps);
    }
    is_active
}

fn is_capture_audio_active(active_apps: &mut Vec<String>) -> bool {
    let _com = crate::utils::ComGuard::mta();
    let own_pid = std::process::id();
    let mut found = false;
    unsafe {
        let Ok(enumerator) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else {
            return false;
        };
        let Ok(collection) = enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) else {
            return false;
        };
        let dev_count = collection.GetCount().unwrap_or(0);
        for d in 0..dev_count {
            let Ok(device) = collection.Item(d) else { continue };
            let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
            let Ok(sessions) = manager.GetSessionEnumerator() else { continue };
            let count = sessions.GetCount().unwrap_or(0);
            for s in 0..count {
                let Ok(session) = sessions.GetSession(s) else { continue };
                let Ok(state) = session.GetState() else { continue };
                if state != AudioSessionStateActive {
                    continue;
                }
                let pid = session
                    .cast::<IAudioSessionControl2>()
                    .ok()
                    .and_then(|c| c.GetProcessId().ok())
                    .unwrap_or(0);
                if pid == own_pid {
                    continue;
                }
                found = true;
                if let Ok(ctrl2) = session.cast::<IAudioSessionControl2>() {
                    if let Ok(name_pw) = ctrl2.GetDisplayName() {
                        let name = name_pw.to_string().unwrap_or_default();
                        windows::Win32::System::Com::CoTaskMemFree(Some(name_pw.as_ptr() as *const _));
                        let clean = name.trim().to_string();
                        if !clean.is_empty() && !clean.starts_with('@') && !active_apps.contains(&clean) {
                            active_apps.push(clean);
                        }
                    }
                }
            }
        }
    }
    found
}

pub fn check_privacy_status() -> PrivacyStatus {
    let mut active_apps = Vec::new();
    let mic_active =
        is_capability_active("microphone", &mut active_apps) || is_capture_audio_active(&mut active_apps);
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

#[cfg(test)]
mod tests {
    use super::is_consent_timestamps_active;

    #[test]
    fn consent_timestamps_handle_missing_or_zero_stop() {
        assert!(!is_consent_timestamps_active(0, None));
        assert!(!is_consent_timestamps_active(0, Some(0)));
        assert!(is_consent_timestamps_active(100, None));
        assert!(is_consent_timestamps_active(100, Some(0)));
        assert!(is_consent_timestamps_active(200, Some(100)));
        assert!(!is_consent_timestamps_active(100, Some(200)));
        assert!(!is_consent_timestamps_active(100, Some(100)));
    }
}
