use std::sync::atomic::Ordering;
use tauri::AppHandle;
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::EnumWindows;

use crate::state::*;

#[tauri::command]
pub fn select_media_session(id: Option<String>) {
    if let Ok(mut selected) = crate::services::SELECTED_MEDIA_SOURCE.lock() {
        *selected = id.filter(|s| !s.is_empty());
    }
    crate::services::MEDIA_REFRESH.store(true, Ordering::Relaxed);
}

#[tauri::command]
pub fn media_play_pause() { if let Some(sender) = COMMAND_SENDER.get() { let _ = sender.send(crate::types::SystemCommand::MediaPlayPause); } }

#[tauri::command]
pub fn media_next() { if let Some(sender) = COMMAND_SENDER.get() { let _ = sender.send(crate::types::SystemCommand::MediaNext); } }

#[tauri::command]
pub fn media_previous() { if let Some(sender) = COMMAND_SENDER.get() { let _ = sender.send(crate::types::SystemCommand::MediaPrevious); } }

#[tauri::command]
pub fn media_seek(position_ms: f64) { if let Some(sender) = COMMAND_SENDER.get() { let _ = sender.send(crate::types::SystemCommand::MediaSeek(position_ms as i64)); } }

#[derive(serde::Serialize)]
pub struct AppMediaSession {
    pub app_id: String,
    pub title: String,
    pub artist: String,
    pub is_playing: bool,
}

#[tauri::command]
pub fn get_app_media_sessions() -> Vec<AppMediaSession> {
    use windows::Media::Control::{GlobalSystemMediaTransportControlsSessionManager, GlobalSystemMediaTransportControlsSessionPlaybackStatus};

    let Ok(mgr) = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.get()) else { return Vec::new() };
    let Ok(sessions) = mgr.GetSessions() else { return Vec::new() };

    sessions
        .into_iter()
        .filter_map(|session| {
            let app_id = session.SourceAppUserModelId().ok()?.to_string();
            let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
            let is_playing = session.GetPlaybackInfo().ok().and_then(|p| p.PlaybackStatus().ok())
                == Some(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing);
            Some(AppMediaSession {
                app_id,
                title: props.Title().ok()?.to_string(),
                artist: props.Artist().map(|a| a.to_string()).unwrap_or_default(),
                is_playing,
            })
        })
        .collect()
}

#[tauri::command]
pub fn app_media_control(app_id: String, action: String) -> Result<(), String> {
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;

    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
        .and_then(|op| op.get())
        .map_err(|e| e.to_string())?;
    let session = mgr
        .GetSessions()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|s| s.SourceAppUserModelId().map(|id| id.to_string() == app_id).unwrap_or(false))
        .ok_or("No media session for that app")?;

    match action.as_str() {
        "play_pause" => session.TryTogglePlayPauseAsync(),
        "next" => session.TrySkipNextAsync(),
        "previous" => session.TrySkipPreviousAsync(),
        _ => return Err("Unknown media action".into()),
    }
    .map(|_| ())
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_media_source_app() {
    unsafe {
        use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager;
        use windows::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_RESTORE, IsIconic, IsWindowVisible, FindWindowW, EnumWindows, GetWindowThreadProcessId};
        use windows::Win32::Foundation::{HWND, LPARAM};
        use windows::Win32::System::Threading::OpenProcess;

        let mgr = match GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.get()) {
            Ok(m) => m,
            Err(_) => return,
        };
        let session = match mgr.GetCurrentSession() {
            Ok(s) => s,
            Err(_) => return,
        };
        let app_id = match session.SourceAppUserModelId() {
            Ok(id) => id.to_string(),
            Err(_) => return,
        };
        if app_id.is_empty() { return; }

        // First try: FindWindow with the AppUserModelId directly (works for UWP)
        let h_app_id = windows::core::HSTRING::from(&app_id);
        if let Ok(hwnd) = FindWindowW(None, windows::core::PCWSTR(h_app_id.as_ptr())) {
            if !hwnd.is_invalid() {
                if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(hwnd);
                return;
            }
        }

        // Second try: find window by process name extracted from app_id
        let process_name = app_id.split('.').next().unwrap_or(&app_id).to_lowercase();

        struct EnumData { process_name: String, found_hwnd: Option<HWND> }

        unsafe extern "system" fn enum_callback(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
            let data = &mut *(lparam.0 as *mut EnumData);
            if !IsWindowVisible(hwnd).as_bool() {
                return windows::core::BOOL(1);
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 { return windows::core::BOOL(1); }

            if let Ok(proc) = OpenProcess(
                windows::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                pid,
            ) {
                let mut buf = [0u16; 260];
                let mut size = buf.len() as u32;
                let ok = windows::Win32::System::Threading::QueryFullProcessImageNameW(
                    proc,
                    windows::Win32::System::Threading::PROCESS_NAME_FORMAT(0),
                    windows::core::PWSTR(buf.as_mut_ptr()),
                    &mut size,
                );
                let _ = windows::Win32::Foundation::CloseHandle(proc);
                if ok.is_ok() {
                    let path = String::from_utf16_lossy(&buf[..size as usize]);
                    let file_name = path.rsplit('\\').next().unwrap_or("").to_lowercase().replace(".exe", "");
                    if file_name == data.process_name {
                        data.found_hwnd = Some(hwnd);
                        return windows::core::BOOL(0);
                    }
                }
            }
            windows::core::BOOL(1)
        }

        let mut data = EnumData { process_name, found_hwnd: None };
        let callback: unsafe extern "system" fn(HWND, LPARAM) -> windows::core::BOOL = enum_callback;
        let _ = EnumWindows(Some(callback), LPARAM(&mut data as *mut EnumData as isize));

        if let Some(hwnd) = data.found_hwnd {
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[tauri::command]
pub fn get_media_info() -> Option<crate::types::MediaInfo> {
    crate::state::LAST_MEDIA.lock().ok().and_then(|last| last.clone())
}

#[tauri::command]
pub async fn get_lyrics(
    app: AppHandle,
    title: String,
    artist: String,
    duration_ms: Option<u64>,
) -> Result<Option<crate::lyrics::LyricsPayload>, String> {
    crate::lyrics::fetch_lyrics(&app, &title, &artist, duration_ms).await
}
