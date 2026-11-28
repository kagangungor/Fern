use std::collections::HashMap;
use std::sync::atomic::Ordering;
use windows::core::Interface;

use crate::state::*;
use crate::types::{AudioSessionInfo, VolumeChangeEvent};

#[tauri::command]
pub fn set_volume(volume: f32) { if let Some(sender) = COMMAND_SENDER.get() { let _ = sender.send(crate::types::SystemCommand::SetVolume(volume)); } }

/// Full path of a process, None if we can't open it.
unsafe fn process_image_path(pid: u32) -> Option<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };

    let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
    let mut buf = [0u16; 1024];
    let mut size = buf.len() as u32;
    let res = QueryFullProcessImageNameW(
        handle,
        PROCESS_NAME_WIN32,
        windows::core::PWSTR(buf.as_mut_ptr()),
        &mut size,
    );
    let _ = CloseHandle(handle);
    res.ok()?;
    Some(String::from_utf16_lossy(&buf[..size as usize]))
}

/// Friendly exe name from version info (FileDescription), else the file stem.
/// Cached because the mixer polls.
unsafe fn friendly_process_name(path: &str) -> String {
    let cache = PROCESS_NAME_CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(name) = guard.get(path) {
            return name.clone();
        }
    }
    let name = friendly_process_name_uncached(path);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(path.to_string(), name.clone());
    }
    name
}

unsafe fn friendly_process_name_uncached(path: &str) -> String {
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let filename = windows::core::PCWSTR(wide.as_ptr());

    'description: {
        let size = GetFileVersionInfoSizeW(filename, None);
        if size == 0 {
            break 'description;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(filename, None, size, data.as_mut_ptr() as *mut _).is_err() {
            break 'description;
        }

        // FileDescription is under the first language/codepage pair in the translation table.
        let mut trans_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut trans_len = 0u32;
        if !VerQueryValueW(
            data.as_ptr() as *const _,
            windows::core::w!("\\VarFileInfo\\Translation"),
            &mut trans_ptr,
            &mut trans_len,
        )
        .as_bool()
            || trans_len < 4
            || trans_ptr.is_null()
        {
            break 'description;
        }
        // Need all four bytes of the (LANGID, codepage) pair, read bytewise to avoid alignment issues.
        let translation = std::slice::from_raw_parts(trans_ptr as *const u8, 4);
        let lang = u16::from_le_bytes([translation[0], translation[1]]);
        let codepage = u16::from_le_bytes([translation[2], translation[3]]);

        let query = format!("\\StringFileInfo\\{lang:04x}{codepage:04x}\\FileDescription");
        let query_wide: Vec<u16> = query.encode_utf16().chain(std::iter::once(0)).collect();
        let mut value_ptr: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut value_len = 0u32;
        if !VerQueryValueW(
            data.as_ptr() as *const _,
            windows::core::PCWSTR(query_wide.as_ptr()),
            &mut value_ptr,
            &mut value_len,
        )
        .as_bool()
            || value_len == 0
            || value_ptr.is_null()
        {
            break 'description;
        }

        let chars = std::slice::from_raw_parts(value_ptr as *const u16, value_len as usize);
        let description = String::from_utf16_lossy(chars)
            .trim_end_matches('\0')
            .trim()
            .to_string();
        if !description.is_empty() {
            return description;
        }
    }

    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|stem| {
            let mut chars = stem.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => stem.to_string(),
            }
        })
        .unwrap_or_else(|| "Unknown app".to_string())
}

/// Runs f for every audio session on every active output (sets up and tears down COM itself).
unsafe fn for_each_audio_session<F: FnMut(&windows::Win32::Media::Audio::IAudioSessionControl2)>(
    mut f: F,
) -> Result<(), String> {
    use windows::Win32::Media::Audio::{
        eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
        DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    let com_initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
    let result = (|| -> Result<(), String> {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(
            &windows::Win32::Media::Audio::MMDeviceEnumerator,
            None,
            CLSCTX_ALL,
        )
        .map_err(|e| e.to_string())?;
        // Per-app output routing can put an app on a non-default device, so scan all active outputs.
        let devices = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .map_err(|e| e.to_string())?;
        let device_count = devices.GetCount().map_err(|e| e.to_string())?;
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
                let Ok(control) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                f(&control);
            }
        }
        Ok(())
    })();
    if com_initialized {
        CoUninitialize();
    }
    result
}

/// Apps with an audio session, one per process, playing ones first.
#[tauri::command]
pub async fn get_audio_sessions() -> Result<Vec<AudioSessionInfo>, String> {
    tauri::async_runtime::spawn_blocking(|| unsafe {
        use std::collections::HashSet;
        use windows::Win32::Media::Audio::{
            AudioSessionStateActive, AudioSessionStateExpired, ISimpleAudioVolume,
        };

        let own_pid = std::process::id();
        let mut seen: HashSet<u32> = HashSet::new();
        let mut found: Vec<(AudioSessionInfo, bool)> = Vec::new();

        for_each_audio_session(|control| {
            // Only S_OK is the system sounds session.
            if control.IsSystemSoundsSession() == windows::Win32::Foundation::S_OK {
                return;
            }
            let Ok(pid) = control.GetProcessId() else {
                return;
            };
            if pid == 0 || pid == own_pid || !seen.insert(pid) {
                return;
            }
            let state = control.GetState().unwrap_or_default();
            if state == AudioSessionStateExpired {
                seen.remove(&pid);
                return;
            }
            let Ok(volume_ctl) = control.cast::<ISimpleAudioVolume>() else {
                return;
            };
            let process_path = process_image_path(pid);
            let name = match process_path.as_deref() {
                Some(path) => friendly_process_name(path),
                None => format!("Process {pid}"),
            };
            found.push((
                AudioSessionInfo {
                    pid,
                    name,
                    process_path,
                    volume: volume_ctl.GetMasterVolume().unwrap_or(1.0),
                    is_muted: volume_ctl.GetMute().map(|m| m.as_bool()).unwrap_or(false),
                },
                state == AudioSessionStateActive,
            ));
        })?;

        found.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| a.0.name.to_lowercase().cmp(&b.0.name.to_lowercase()))
        });
        Ok(found.into_iter().map(|(info, _)| info).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_app_volume(pid: u32, volume: f32) -> Result<(), String> {
    let volume = volume.clamp(0.0, 1.0);
    tauri::async_runtime::spawn_blocking(move || unsafe {
        use windows::Win32::Media::Audio::ISimpleAudioVolume;

        for_each_audio_session(|control| {
            if control.GetProcessId().ok() != Some(pid) {
                return;
            }
            if let Ok(volume_ctl) = control.cast::<ISimpleAudioVolume>() {
                let _ = volume_ctl.SetMasterVolume(volume, std::ptr::null());
                if volume > 0.0 {
                    let _ = volume_ctl.SetMute(false, std::ptr::null());
                }
            }
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_app_mute(pid: u32, muted: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || unsafe {
        use windows::Win32::Media::Audio::ISimpleAudioVolume;

        for_each_audio_session(|control| {
            if control.GetProcessId().ok() != Some(pid) {
                return;
            }
            if let Ok(volume_ctl) = control.cast::<ISimpleAudioVolume>() {
                let _ = volume_ctl.SetMute(muted, std::ptr::null());
            }
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Mixer panel bounds in overlay CSS px, so the mouse hook keeps the card clickable while open.
#[tauri::command]
pub fn set_volume_mixer_rect(x: f64, y: f64, width: f64, height: f64) {
    if let Ok(mut rect) = VOLUME_MIXER_RECT.lock() {
        *rect = Some((x, y, width, height));
    }
}

#[tauri::command]
pub fn clear_volume_mixer_rect() {
    if let Ok(mut rect) = VOLUME_MIXER_RECT.lock() {
        *rect = None;
    }
}

#[tauri::command]
pub fn get_volume() -> f32 {
    crate::state::CURRENT_VOLUME.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0
}

/// Live volume and mute of the default output. HUDs seed from this because the worker's
/// first volume-change fires before the overlay can listen.
#[tauri::command]
pub async fn get_volume_state() -> Result<VolumeChangeEvent, String> {
    tauri::async_runtime::spawn_blocking(|| unsafe {
        use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
        use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
        };

        let com_initialized = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let result = (|| -> Result<VolumeChangeEvent, String> {
            let enumerator: IMMDeviceEnumerator = CoCreateInstance(
                &windows::Win32::Media::Audio::MMDeviceEnumerator,
                None,
                CLSCTX_ALL,
            )
            .map_err(|e| e.to_string())?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .map_err(|e| e.to_string())?;
            let endpoint: IAudioEndpointVolume = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| e.to_string())?;
            let volume = endpoint
                .GetMasterVolumeLevelScalar()
                .map_err(|e| e.to_string())?;
            let is_muted = endpoint.GetMute().map(|m| m.as_bool()).unwrap_or(false);
            CURRENT_VOLUME.store((volume * 100.0) as u32, Ordering::Relaxed);
            Ok(VolumeChangeEvent { volume, is_muted })
        })();
        if com_initialized {
            CoUninitialize();
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

// Per-App Audio Scroll Mixer
#[tauri::command]
pub fn get_target_app_volume(target: String, pids: Option<Vec<u32>>) -> Result<Option<crate::app_audio::AppVolumeState>, String> {
    crate::app_audio::get_app_volume(&target, pids.as_deref())
}

#[tauri::command]
pub fn set_target_app_volume(target: String, volume: f32, pids: Option<Vec<u32>>) -> Result<crate::app_audio::AppVolumeState, String> {
    crate::app_audio::set_app_volume(&target, pids.as_deref(), volume)
}

#[tauri::command]
pub fn adjust_target_app_volume(target: String, delta: f32, pids: Option<Vec<u32>>) -> Result<crate::app_audio::AppVolumeState, String> {
    crate::app_audio::adjust_app_volume(&target, pids.as_deref(), delta)
}

#[tauri::command]
pub fn toggle_target_app_mute(target: String, pids: Option<Vec<u32>>) -> Result<crate::app_audio::AppVolumeState, String> {
    crate::app_audio::toggle_app_mute(&target, pids.as_deref())
}
