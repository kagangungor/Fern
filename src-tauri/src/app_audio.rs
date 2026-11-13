use std::path::Path;
use serde::{Deserialize, Serialize};
use windows::core::Interface;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Media::Audio::{
    eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
    ISimpleAudioVolume, DEVICE_STATE_ACTIVE, MMDeviceEnumerator,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CLSCTX_ALL,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppVolumeState {
    pub volume: f32, // 0.0 to 1.0
    pub muted: bool,
    pub found: bool,
}

/// Helper to get the lowercase executable name (e.g. "spotify.exe") for a given PID.
fn get_exe_name_for_pid(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let res = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);

        if res.is_ok() && len > 0 {
            let full_path = String::from_utf16_lossy(&buf[..len as usize]);
            Path::new(&full_path)
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_lowercase())
        } else {
            None
        }
    }
}

fn stem_prefix_match(longer: &str, shorter: &str) -> bool {
    if !longer.starts_with(shorter) {
        return false;
    }
    let suffix = &longer[shorter.len()..];
    if suffix.is_empty() {
        return true;
    }
    // Allow version/arch numeric suffixes like obs -> obs64 (len >= 3) or longer app prefixes (len >= 5).
    (shorter.len() >= 3 && suffix.chars().all(|c| c.is_ascii_digit())) || shorter.len() >= 5
}

/// Normalized check to see if an audio session matches the target application.
fn is_session_match(
    session_pid: u32,
    session_exe: Option<&str>,
    target_clean: &str,
    extra_pids: Option<&[u32]>,
) -> bool {
    if let Some(pids) = extra_pids {
        if pids.contains(&session_pid) {
            return true;
        }
    }

    if let Some(exe) = session_exe {
        let exe_clean = exe.trim().to_lowercase();
        if exe_clean.is_empty() || target_clean.is_empty() {
            return false;
        }
        if exe_clean == target_clean {
            return true;
        }
        let exe_stem = exe_clean.strip_suffix(".exe").unwrap_or(&exe_clean);
        let target_stem = target_clean.strip_suffix(".exe").unwrap_or(target_clean);
        if exe_stem == target_stem {
            return true;
        }
        if exe_stem.len() >= 3
            && target_stem
                .split(|c: char| !c.is_alphanumeric())
                .any(|tok| tok == exe_stem)
        {
            return true;
        }
        if target_stem.len() >= 3
            && exe_stem
                .split(|c: char| !c.is_alphanumeric())
                .any(|tok| tok == target_stem)
        {
            return true;
        }
        if stem_prefix_match(exe_stem, target_stem) || stem_prefix_match(target_stem, exe_stem) {
            return true;
        }
    }

    false
}

/// Iterate through all active audio endpoints and execute a closure for matching sessions.
fn for_matching_sessions<F>(
    target: &str,
    extra_pids: Option<&[u32]>,
    mut op: F,
) -> Result<bool, String>
where
    F: FnMut(&ISimpleAudioVolume, u32) -> Result<(), String>,
{
    let target_clean = Path::new(target)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(target)
        .trim()
        .to_lowercase();

    unsafe {
        let _com = crate::utils::ComGuard::mta();
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| e.to_string())?;

            let devices = enumerator
                .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
                .map_err(|e| e.to_string())?;

            let device_count = devices.GetCount().map_err(|e| e.to_string())?;
            let mut matched_any = false;

            for i in 0..device_count {
                let Ok(device) = devices.Item(i) else {
                    continue;
                };
                let Ok(manager) = device.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else {
                    continue;
                };
                let Ok(sessions) = manager.GetSessionEnumerator() else {
                    continue;
                };
                let Ok(count) = sessions.GetCount() else {
                    continue;
                };

                for j in 0..count {
                    let Ok(control) = sessions.GetSession(j) else {
                        continue;
                    };
                    let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                        continue;
                    };

                    let pid = control2.GetProcessId().unwrap_or(0);
                    let exe_name = get_exe_name_for_pid(pid);

                    if is_session_match(pid, exe_name.as_deref(), &target_clean, extra_pids) {
                        if let Ok(simple_vol) = control2.cast::<ISimpleAudioVolume>() {
                            if op(&simple_vol, pid).is_ok() {
                                matched_any = true;
                            }
                        }
                    }
                }
            }

            Ok(matched_any)
    }
}

/// Get the current volume and mute state of a target application.
pub fn get_app_volume(
    target: &str,
    extra_pids: Option<&[u32]>,
) -> Result<Option<AppVolumeState>, String> {
    let mut current_vol = 1.0f32;
    let mut is_muted = false;
    let mut found = false;

    let matched = for_matching_sessions(target, extra_pids, |simple_vol, _pid| {
        unsafe {
            if let Ok(vol) = simple_vol.GetMasterVolume() {
                current_vol = vol;
                found = true;
            }
            if let Ok(muted) = simple_vol.GetMute() {
                is_muted = muted.as_bool();
            }
        }
        Ok(())
    })?;

    if matched || found {
        Ok(Some(AppVolumeState {
            volume: current_vol,
            muted: is_muted,
            found: true,
        }))
    } else {
        Ok(None)
    }
}

/// Set the volume for a target application, adjusting all matching sessions.
pub fn set_app_volume(
    target: &str,
    extra_pids: Option<&[u32]>,
    volume: f32,
) -> Result<AppVolumeState, String> {
    let clamped_vol = volume.clamp(0.0, 1.0);
    let mut was_muted = false;

    let matched = for_matching_sessions(target, extra_pids, |simple_vol, _pid| {
        unsafe {
            let _ = simple_vol.SetMasterVolume(clamped_vol, std::ptr::null());
            if clamped_vol > 0.0 {
                // Auto unmute if volume is raised above 0
                if let Ok(muted) = simple_vol.GetMute() {
                    if muted.as_bool() {
                        let _ = simple_vol.SetMute(false, std::ptr::null());
                    }
                }
            } else if clamped_vol == 0.0 {
                was_muted = true;
            }
        }
        Ok(())
    })?;

    Ok(AppVolumeState {
        volume: clamped_vol,
        muted: if clamped_vol == 0.0 { true } else { was_muted },
        found: matched,
    })
}

/// Adjust application volume by a continuous delta (+/-).
pub fn adjust_app_volume(
    target: &str,
    extra_pids: Option<&[u32]>,
    delta: f32,
) -> Result<AppVolumeState, String> {
    let current_vol = get_app_volume(target, extra_pids)?
        .map(|s| s.volume)
        .unwrap_or(1.0);

    let next_vol = (current_vol + delta).clamp(0.0, 1.0);
    set_app_volume(target, extra_pids, next_vol)
}

/// Toggle mute state for all sessions belonging to the target application.
pub fn toggle_app_mute(
    target: &str,
    extra_pids: Option<&[u32]>,
) -> Result<AppVolumeState, String> {
    let current_state = get_app_volume(target, extra_pids)?;
    let new_mute = !current_state.as_ref().map(|s| s.muted).unwrap_or(false);
    let current_vol = current_state.as_ref().map(|s| s.volume).unwrap_or(1.0);

    let matched = for_matching_sessions(target, extra_pids, |simple_vol, _pid| {
        unsafe {
            let _ = simple_vol.SetMute(new_mute, std::ptr::null());
        }
        Ok(())
    })?;

    Ok(AppVolumeState {
        volume: current_vol,
        muted: new_mute,
        found: matched,
    })
}

#[cfg(test)]
mod tests {
    use super::is_session_match;

    #[test]
    fn session_matching_handles_stems_tokens_and_rejects_short_substrings() {
        assert!(is_session_match(10, Some("Spotify.exe"), "spotify.exe", None));
        assert!(is_session_match(10, Some("Spotify.exe"), "spotify", None));
        assert!(is_session_match(10, Some("chrome.exe"), "google chrome", None));
        assert!(is_session_match(10, Some("obs64.exe"), "obs", None));
        assert!(is_session_match(42, Some("other.exe"), "unrelated.exe", Some(&[42])));

        assert!(!is_session_match(10, Some("whatsapp.exe"), "app", None));
        assert!(!is_session_match(10, Some("knowledge.exe"), "edge.exe", None));
        assert!(!is_session_match(10, Some("spotify.exe"), "", None));
    }
}
