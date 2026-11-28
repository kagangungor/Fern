use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::core::BOOL;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use wmi::{COMLibrary, WMIConnection};
use crate::types::*;
use crate::state::*;
use crate::utils::*;
use super::hooks::{CURRENT_START_ON_DOCK, START_TAP_GRACE_MS, START_TAP_MS, VISIBILITY_THREAD, overlay_enabled};
use super::mouse_hit::{CAPTURE_LAST_SCAN_MS, CAPTURE_RECHECK, CAPTURE_UI_ACTIVE, apply_capture_ui_state, is_capture_ui_present, monitor_rect_for_hwnd, now_ms};

pub static SELECTED_MEDIA_SOURCE: Mutex<Option<String>> = Mutex::new(None);
static ACTIVE_MEDIA_SOURCE: Mutex<String> = Mutex::new(String::new());
static ACTIVE_MEDIA_TITLE: Mutex<String> = Mutex::new(String::new());
pub static MEDIA_REFRESH: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
struct MediaSessionCandidate {
    index: u32,
    source_id: String,
    title: String,
    is_playing: bool,
    last_updated_ticks: i64,
}

fn choose_media_session_index(
    entries: &[MediaSessionCandidate],
    selected_source: Option<&str>,
    prev_active_source: &str,
    prev_active_title: &str,
    current_session: Option<(&str, &str)>,
) -> Option<u32> {
    if entries.is_empty() {
        return None;
    }

    let is_current = |e: &MediaSessionCandidate| {
        current_session.is_some_and(|(cid, ctitle)| e.source_id == cid && e.title == ctitle)
    };
    let is_prev_exact = |e: &MediaSessionCandidate| {
        !prev_active_source.is_empty()
            && e.source_id == prev_active_source
            && e.title == prev_active_title
    };

    if let Some(sel) = selected_source {
        if let Some(chosen) = entries
            .iter()
            .filter(|e| e.source_id == sel)
            .max_by_key(|e| {
                (
                    e.is_playing,
                    is_current(e),
                    e.last_updated_ticks,
                    is_prev_exact(e),
                    std::cmp::Reverse(e.index),
                )
            })
        {
            return Some(chosen.index);
        }
    }

    if let Some(chosen) = entries
        .iter()
        .filter(|e| e.is_playing)
        .max_by_key(|e| {
            (
                is_current(e),
                e.last_updated_ticks,
                is_prev_exact(e),
                std::cmp::Reverse(e.index),
            )
        })
    {
        return Some(chosen.index);
    }

    entries
        .iter()
        .max_by_key(|e| {
            (
                is_prev_exact(e),
                !prev_active_source.is_empty() && e.source_id == prev_active_source,
                is_current(e),
                e.last_updated_ticks,
                std::cmp::Reverse(e.index),
            )
        })
        .map(|e| e.index)
}

fn media_app_name(source_id: &str) -> String {
    let id = source_id.rsplit('!').next().unwrap_or(source_id);
    let id = id.trim_end_matches(".exe").trim_end_matches(".EXE");
    let name = id.rsplit('.').next().unwrap_or(id);
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn media_control_session(
    mgr: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager,
) -> windows::core::Result<windows::Media::Control::GlobalSystemMediaTransportControlsSession> {
    let active = ACTIVE_MEDIA_SOURCE.lock().map(|g| g.clone()).unwrap_or_default();
    let active_title = ACTIVE_MEDIA_TITLE.lock().map(|g| g.clone()).unwrap_or_default();
    if !active.is_empty() {
        if let Ok(sessions) = mgr.GetSessions() {
            let mut source_fallback = None;
            for i in 0..sessions.Size().unwrap_or(0) {
                if let Ok(session) = sessions.GetAt(i) {
                    if session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default() == active {
                        if !active_title.is_empty() {
                            let title = session
                                .TryGetMediaPropertiesAsync()
                                .and_then(|op| op.get())
                                .and_then(|p| p.Title())
                                .map(|t| t.to_string())
                                .unwrap_or_default();
                            if title == active_title {
                                return Ok(session);
                            }
                        }
                        if source_fallback.is_none() {
                            source_fallback = Some(session);
                        }
                    }
                }
            }
            if let Some(session) = source_fallback {
                return Ok(session);
            }
        }
    }
    mgr.GetCurrentSession()
}

pub fn setup_system_worker(app_handle: AppHandle) -> Sender<SystemCommand> {
    let (tx, rx) = channel::<SystemCommand>();
    let handle_system = app_handle.clone();
    std::thread::spawn(move || {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED, CoCreateInstance, CLSCTX_ALL, CoTaskMemFree};
        use windows::Win32::Media::Audio::{IMMDeviceEnumerator, eRender, eConsole};
        use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
        use windows::Media::Control::{GlobalSystemMediaTransportControlsSessionManager, GlobalSystemMediaTransportControlsSessionPlaybackStatus};
        use base64::{Engine as _, engine::general_purpose};
        use windows::Storage::Streams::DataReader;

        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator = CoCreateInstance::<_, IMMDeviceEnumerator>(&windows::Win32::Media::Audio::MMDeviceEnumerator, None, CLSCTX_ALL).ok();
            let mut device = enumerator.as_ref().and_then(|e| e.GetDefaultAudioEndpoint(eRender, eConsole).ok());
            let mut audio_endpoint_volume: Option<IAudioEndpointVolume> = device.as_ref().and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok());
            
            let mut current_device_id = String::new();
            if let Some(ref d) = device {
                if let Ok(id) = d.GetId() {
                    current_device_id = windows::core::PCWSTR::from_raw(id.0).to_string().unwrap_or_default();
                    CoTaskMemFree(Some(id.0 as *const _));
                }
            }

            let mut manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.get()).ok();
            let mut last_processed_media = std::time::Instant::now();
            let mut media_burst_until: Option<std::time::Instant> = None;
            let mut last_device_check = std::time::Instant::now();
            #[allow(clippy::type_complexity)]
            let mut last_emitted_info: Option<(String, String, bool, bool, Option<String>, i64, i64)> = None;
            let mut art_attempts: (String, u32) = (String::new(), 0);
            let mut last_volume: f32 = -1.0;
            let mut last_sessions_json = String::new();
            let mut last_muted: bool = false;

            // Only hide the native flyout when our overlay replaces it.
            let hide_osd = |overlay_key: &str| {
                if !overlay_enabled(&handle_system, overlay_key) { return; }
                use windows::Win32::UI::WindowsAndMessaging::{FindWindowA, ShowWindow, SW_HIDE};
                let class1 = windows::core::PCSTR(c"NativeHWNDHost".as_ptr() as *const u8);
                if let Ok(hwnd1) = FindWindowA(class1, windows::core::PCSTR::null()) { let _ = ShowWindow(hwnd1, SW_HIDE); }
            };

            loop {
                // Check for device change
                if last_device_check.elapsed().as_secs() >= 2 {
                    last_device_check = std::time::Instant::now();
                    if let Some(ref enum_ref) = enumerator {
                        if let Ok(new_device) = enum_ref.GetDefaultAudioEndpoint(eRender, eConsole) {
                            if let Ok(id) = new_device.GetId() {
                                let new_id = windows::core::PCWSTR::from_raw(id.0).to_string().unwrap_or_default();
                                CoTaskMemFree(Some(id.0 as *const _));
                                if new_id != current_device_id || audio_endpoint_volume.is_none() {
                                    current_device_id = new_id;
                                    device = Some(new_device);
                                    audio_endpoint_volume = device.as_ref().and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok());
                                    // Reset last_volume to force an update event
                                    last_volume = -1.0;
                                }
                            }
                        }
                    }
                }

                if manager.is_none() { manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.get()).ok(); }
                while let Ok(cmd) = rx.try_recv() {
                    // Only the volume commands need the audio endpoint, the rest must work with no output device.
                    {
                        match cmd {
                            SystemCommand::VolumeMute => { if let Some(aev) = audio_endpoint_volume.as_ref() { if let Ok(muted) = aev.GetMute() { let _ = aev.SetMute(!muted.as_bool(), std::ptr::null()); hide_osd("fern-volume-overlay-enabled"); } } }
                            SystemCommand::VolumeUp => {
                                if let Some(aev) = audio_endpoint_volume.as_ref() {
                                    if let (Ok(vol), Ok(muted)) = (aev.GetMasterVolumeLevelScalar(), aev.GetMute()) {
                                        let _ = aev.SetMasterVolumeLevelScalar((vol + 0.05).min(1.0), std::ptr::null());
                                        if muted.as_bool() { let _ = aev.SetMute(false, std::ptr::null()); }
                                        hide_osd("fern-volume-overlay-enabled");
                                    }
                                }
                            }
                            SystemCommand::VolumeDown => { if let Some(aev) = audio_endpoint_volume.as_ref() { if let Ok(vol) = aev.GetMasterVolumeLevelScalar() { let _ = aev.SetMasterVolumeLevelScalar((vol - 0.05).max(0.0), std::ptr::null()); hide_osd("fern-volume-overlay-enabled"); } } }
                            SystemCommand::SetVolume(volume) => {
                                if let Some(aev) = audio_endpoint_volume.as_ref() {
                                    let _ = aev.SetMasterVolumeLevelScalar(volume.clamp(0.0, 1.0), std::ptr::null());
                                    if volume > 0.0 { let _ = aev.SetMute(false, std::ptr::null()); }
                                    hide_osd("fern-volume-overlay-enabled");
                                }
                            }
                            SystemCommand::BrightnessUp => {
                                let new_val = (CURRENT_BRIGHTNESS.load(Ordering::Relaxed) + 10).min(100);
                                CURRENT_BRIGHTNESS.store(new_val, Ordering::Relaxed);
                                LAST_BRIGHTNESS_CHANGE.store(get_now_ms(), Ordering::Relaxed);
                                let _ = handle_system.emit("brightness-change", BrightnessChangeEvent { brightness: new_val });
                                if let Some(tx) = BRIGHTNESS_SENDER.get() { let _ = tx.send(new_val); }
                                hide_osd("fern-brightness-overlay-enabled");
                            }
                            SystemCommand::BrightnessDown => {
                                let current = CURRENT_BRIGHTNESS.load(Ordering::Relaxed);
                                let new_val = current.saturating_sub(10);
                                CURRENT_BRIGHTNESS.store(new_val, Ordering::Relaxed);
                                LAST_BRIGHTNESS_CHANGE.store(get_now_ms(), Ordering::Relaxed);
                                let _ = handle_system.emit("brightness-change", BrightnessChangeEvent { brightness: new_val });
                                if let Some(tx) = BRIGHTNESS_SENDER.get() { let _ = tx.send(new_val); }
                                hide_osd("fern-brightness-overlay-enabled");
                            }
                            SystemCommand::MediaPlayPause => { if let Some(ref mgr) = manager { if let Ok(session) = media_control_session(mgr) { let _ = session.TryTogglePlayPauseAsync(); MEDIA_REFRESH.store(true, Ordering::Relaxed); media_burst_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(1500)); } } }
                            SystemCommand::MediaNext => { if let Some(ref mgr) = manager { if let Ok(session) = media_control_session(mgr) { let _ = session.TrySkipNextAsync(); MEDIA_REFRESH.store(true, Ordering::Relaxed); media_burst_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(1500)); } } }
                            SystemCommand::MediaPrevious => { if let Some(ref mgr) = manager { if let Ok(session) = media_control_session(mgr) { let _ = session.TrySkipPreviousAsync(); MEDIA_REFRESH.store(true, Ordering::Relaxed); media_burst_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(1500)); } } }
                            SystemCommand::MediaSeek(position_ms) => {
                                if let Some(ref mgr) = manager {
                                    if let Ok(session) = media_control_session(mgr) {
                                        let ticks = position_ms * 10_000;
                                        let _ = session.TryChangePlaybackPositionAsync(ticks);
                                        MEDIA_REFRESH.store(true, Ordering::Relaxed);
                                        media_burst_until = Some(std::time::Instant::now() + std::time::Duration::from_millis(1500));
                                    }
                                }
                            }
                            SystemCommand::ToggleVisibility(visible) => {
                                let _ = handle_system.emit("visibility-change", visible);
                                if let Some(w) = handle_system.get_webview_window("bottom-corners") { if visible { let _ = w.show(); } else { let _ = w.hide(); } }
                            }
                        }
                    }
                }
                if let Some(ref aev) = audio_endpoint_volume {
                    if let (Ok(vol), Ok(muted)) = (aev.GetMasterVolumeLevelScalar(), aev.GetMute()) {
                        let is_muted: bool = muted.into();
                        if (vol - last_volume).abs() > 0.001 || is_muted != last_muted {
                            last_volume = vol; last_muted = is_muted;
                            crate::state::CURRENT_VOLUME.store((vol * 100.0) as u32, Ordering::Relaxed);
                            let _ = handle_system.emit("volume-change", VolumeChangeEvent { volume: vol, is_muted });
                            hide_osd("fern-volume-overlay-enabled");
                        }
                    }
                }
                let media_poll_ms = if media_burst_until.is_some_and(|t| t > std::time::Instant::now()) { 100 } else { 400 };
                if last_processed_media.elapsed().as_millis() >= media_poll_ms || MEDIA_REFRESH.swap(false, Ordering::Relaxed) {
                    last_processed_media = std::time::Instant::now();
                    let mut best_info: Option<MediaInfo> = None;
                    if let Some(ref mgr) = manager {
                        if let Ok(sessions) = mgr.GetSessions() {
                            let current_session = mgr.GetCurrentSession().ok().and_then(|s| {
                                let cid = s.SourceAppUserModelId().map(|v| v.to_string()).unwrap_or_default();
                                let ctitle = s
                                    .TryGetMediaPropertiesAsync()
                                    .and_then(|op| op.get())
                                    .and_then(|p| p.Title())
                                    .map(|t| t.to_string())
                                    .unwrap_or_default();
                                if cid.is_empty() || ctitle.is_empty() { None } else { Some((cid, ctitle)) }
                            });
                            let mut entries: Vec<MediaSessionCandidate> = Vec::new();
                            for i in 0..sessions.Size().unwrap_or(0) {
                                if let Ok(session) = sessions.GetAt(i) {
                                    let title = session
                                        .TryGetMediaPropertiesAsync()
                                        .and_then(|op| op.get())
                                        .and_then(|p| p.Title())
                                        .map(|t| t.to_string())
                                        .unwrap_or_default();
                                    if title.is_empty() { continue; }
                                    let id = session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default();
                                    let playing = session.GetPlaybackInfo().ok().and_then(|p| p.PlaybackStatus().ok()) == Some(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing);
                                    let last_updated_ticks = session
                                        .GetTimelineProperties()
                                        .ok()
                                        .and_then(|tl| tl.LastUpdatedTime().ok())
                                        .map(|dt| dt.UniversalTime)
                                        .unwrap_or(0);
                                    entries.push(MediaSessionCandidate {
                                        index: i,
                                        source_id: id,
                                        title,
                                        is_playing: playing,
                                        last_updated_ticks,
                                    });
                                }
                            }
                            let selected = SELECTED_MEDIA_SOURCE.lock().ok().and_then(|g| g.clone());
                            let prev_active = ACTIVE_MEDIA_SOURCE.lock().ok().map(|g| g.clone()).unwrap_or_default();
                            let prev_title = ACTIVE_MEDIA_TITLE.lock().ok().map(|g| g.clone()).unwrap_or_default();
                            let chosen_i = choose_media_session_index(
                                &entries,
                                selected.as_deref(),
                                &prev_active,
                                &prev_title,
                                current_session.as_ref().map(|(id, t)| (id.as_str(), t.as_str())),
                            );
                            let chosen_entry = entries.iter().find(|e| Some(e.index) == chosen_i);
                            let active_id = chosen_entry.map(|e| e.source_id.clone()).unwrap_or_default();
                            let active_title = chosen_entry.map(|e| e.title.clone()).unwrap_or_default();
                            if let Ok(mut g) = ACTIVE_MEDIA_SOURCE.lock() { *g = active_id.clone(); }
                            if let Ok(mut g) = ACTIVE_MEDIA_TITLE.lock() { *g = active_title; }
                            let list: Vec<serde_json::Value> = entries
                                .iter()
                                .map(|e| serde_json::json!({ "id": e.source_id, "name": media_app_name(&e.source_id), "title": e.title, "is_playing": e.is_playing, "active": Some(e.index) == chosen_i }))
                                .collect();
                            let list_json = serde_json::to_string(&list).unwrap_or_default();
                            if list_json != last_sessions_json {
                                let _ = handle_system.emit("media-sessions", list);
                                last_sessions_json = list_json;
                            }
                            for i in 0..sessions.Size().unwrap_or(0) {
                                if Some(i) != chosen_i { continue; }
                                if let Ok(session) = sessions.GetAt(i) {
                                    let is_playing = session.GetPlaybackInfo().ok().and_then(|p| p.PlaybackStatus().ok()) == Some(GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing);
                                    if let Ok(props) = session.TryGetMediaPropertiesAsync().and_then(|op| op.get()) {
                                        let title = props.Title().unwrap_or_default().to_string();
                                        if !title.is_empty() {
                                            let artist = props.Artist().unwrap_or_default().to_string();
                                            let mut artwork = None;
                                            let mut got_art_from_cache = false;
                                            let track_key = format!("{}|{}", title, artist);
                                            if art_attempts.0 != track_key {
                                                art_attempts = (track_key, 0);
                                            }
                                            if let Some((ref last_title, ref last_artist, _, _, ref last_art, _, _)) = last_emitted_info {
                                                // A track whose artwork failed to load is retried every poll at first, then every 15th poll, the player may not have it ready yet
                                                let retry_due = should_retry_artwork(art_attempts.1);
                                                if last_title == &title && last_artist == &artist && (last_art.is_some() || !retry_due) {
                                                    artwork = last_art.as_ref().map(|art| vec![art.clone()]);
                                                    got_art_from_cache = true;
                                                    if last_art.is_none() {
                                                        art_attempts.1 += 1;
                                                    }
                                                }
                                            }

                                            if !got_art_from_cache {
                                                artwork = (|| -> Option<Vec<String>> {
                                                    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
                                                    let content_type = stream.ContentType().ok()?.to_string().split(',').next().unwrap_or("image/jpeg").trim().to_string();
                                                    let reader = DataReader::CreateDataReader(&stream).ok()?;
                                                    let mut all_bytes = Vec::new();
                                                    let chunk_size = 65536u32;
                                                    loop {
                                                        let loaded = reader.LoadAsync(chunk_size).ok()?.get().ok()?;
                                                        if loaded == 0 { break; }
                                                        let mut chunk = vec![0u8; loaded as usize];
                                                        reader.ReadBytes(&mut chunk).ok()?;
                                                        all_bytes.extend_from_slice(&chunk);
                                                    }
                                                    if all_bytes.is_empty() { return None; }
                                                    Some(vec![format!("data:{};base64,{}", content_type, general_purpose::STANDARD.encode(all_bytes))])
                                                })();
                                                if artwork.is_none() {
                                                    art_attempts.1 += 1;
                                                }
                                            }

                                            // Extract timeline properties for progress bar
                                            let (position_ms, duration_ms, seek_enabled) = {
                                                let defaults = (0i64, 0i64, false);
                                                match session.GetTimelineProperties() {
                                                    Ok(timeline) => {
                                                        let start = timeline.StartTime().map(|t| t.Duration / 10_000).unwrap_or(0);
                                                        let end = timeline.EndTime().map(|t| t.Duration / 10_000).unwrap_or(0);
                                                        let pos = timeline.Position().map(|t| t.Duration / 10_000).unwrap_or(0);
                                                        let dur = (end - start).max(0);
                                                        // If duration is 0 but position > 0, some players don't report start/end
                                                        // but still track position — use position as fallback duration indicator
                                                        let effective_dur = if dur > 0 { dur } else { 0 };
                                                        // Check if seeking is supported
                                                        let seek = session.GetPlaybackInfo().ok()
                                                            .and_then(|pi| pi.Controls().ok())
                                                            .map(|c| c.IsPlaybackPositionEnabled().unwrap_or(false))
                                                            .unwrap_or(false);
                                                        (pos, effective_dur, seek)
                                                    }
                                                    Err(_) => defaults,
                                                }
                                            };

                                            let now_ms = std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_millis() as u64)
                                                .unwrap_or(0);
                                            let info = MediaInfo { title, artist, is_playing, has_media: true, artwork, position_ms, duration_ms, seek_enabled, position_updated_at: now_ms, source_id: active_id.clone() };
                                            if is_playing { best_info = Some(info); break; } else if best_info.is_none() { best_info = Some(info); }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let current = best_info.unwrap_or(MediaInfo { title: "".into(), artist: "".into(), is_playing: false, has_media: false, artwork: None, position_ms: 0, duration_ms: 0, seek_enabled: false, position_updated_at: 0, source_id: String::new() });
                    let art_str = current.artwork.as_ref().and_then(|a| a.first()).cloned();
                    if last_emitted_info.as_ref().is_none_or(|(t, a, p, h, art, pos, dur)| {
                        t != &current.title || a != &current.artist || p != &current.is_playing || h != &current.has_media || art != &art_str || dur != &current.duration_ms ||
                        // For position: emit if >1s difference (avoid spamming on every poll)
                        (*pos - current.position_ms).abs() > 1000
                    }) {
                        if let Ok(mut guard) = crate::state::LAST_MEDIA.lock() {
                            *guard = Some(current.clone());
                        }
                        let _ = handle_system.emit("media-update", current.clone());
                        ANY_MEDIA_PLAYING.store(current.is_playing, Ordering::Relaxed);
                        last_emitted_info = Some((current.title, current.artist, current.is_playing, current.has_media, art_str, current.position_ms, current.duration_ms));
                    }
                }
                std::thread::sleep(crate::memory_saver::poll_interval(std::time::Duration::from_millis(64)));
            }
        }
    });

    let handle_brightness = app_handle.clone();
    std::thread::spawn(move || {
        let com_lib = match COMLibrary::new() { Ok(lib) => lib, Err(_) => return };
        let wmi_con = match WMIConnection::with_namespace_path("root\\WMI", com_lib) { Ok(con) => con, Err(_) => return };
        let first = wmi_con.query::<WmiMonitorBrightness>();
        let supported = first.as_ref().map(|r| !r.is_empty()).unwrap_or(false);
        let poll = if supported { 1500 } else { 30_000 };
        let saved_brightness = if supported { None } else {
            handle_brightness.path().app_config_dir().ok()
                .and_then(|d| std::fs::read_to_string(d.join("last_brightness.txt")).ok())
                .and_then(|t| t.trim().parse::<u32>().ok())
                .map(|v| v.min(100))
        };
        let mut last_brightness = match first.ok().and_then(|res| res.first().map(|b| b.current_brightness as u32)) { Some(v) => v, None => saved_brightness.unwrap_or(50) };
        CURRENT_BRIGHTNESS.store(last_brightness, Ordering::Relaxed);
        loop {
            if get_now_ms() - LAST_BRIGHTNESS_CHANGE.load(Ordering::Relaxed) < 2000 { std::thread::sleep(std::time::Duration::from_millis(500)); continue; }
            if let Ok(results) = wmi_con.query::<WmiMonitorBrightness>() {
                if let Some(b) = results.first() {
                    let brightness = b.current_brightness as u32;
                    if brightness != last_brightness { last_brightness = brightness; CURRENT_BRIGHTNESS.store(brightness, Ordering::Relaxed); let _ = handle_brightness.emit("brightness-change", BrightnessChangeEvent { brightness }); }
                }
            }
            std::thread::sleep(crate::memory_saver::poll_interval(std::time::Duration::from_millis(poll)));
        }
    });

    let tx_clone = tx.clone();
    let handle_visibility = app_handle.clone();
    std::thread::spawn(move || {
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, IsZoomed, IsIconic, GetWindowLongW, GWL_STYLE, WS_MAXIMIZE, GetClientRect};
        use windows::Win32::Graphics::Gdi::ClientToScreen;
        use windows::Win32::Foundation::{RECT, POINT};
        let _ = VISIBILITY_THREAD.set(std::thread::current());
        let mut last_visible = true;
        let mut last_dock_overlap: Option<bool> = None;
        let mut last_dock_fullscreen: Option<bool> = None;
        let mut last_dock_shell_surface: Option<bool> = None;
        let mut last_notch_fullscreen: Option<bool> = None;
        let mut last_secondary_dock_fullscreen = std::collections::HashMap::<String, (bool, Instant)>::new();
        let mut last_secondary_dock_shell_surface = std::collections::HashMap::<String, (bool, Instant)>::new();
        let mut last_secondary_overlap = std::collections::HashMap::<String, (bool, Instant)>::new();
        let mut last_notch_overlap: Option<bool> = None;
        let mut last_dock_maximized: Option<bool> = None;
        let mut last_fg_maximized = false;
        let mut last_hwnd = HWND(std::ptr::null_mut());
        let mut last_emit = Instant::now();
        let mut is_known_shell = false;
        let mut is_desktop_focused = false;
        let my_process_id = std::process::id();
        let mut last_monitor_update = Instant::now() - Duration::from_secs(5);
        let mut cached_scale = 1.0f64;
        let mut last_start_check = HWND(std::ptr::null_mut());
        let mut start_open = false;
        let mut start_focused_at = Instant::now();

        loop {
            if SHUTTING_DOWN.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(150));
                continue;
            }
            unsafe {
                let now = Instant::now();
                // Start and Search open above the screen's bottom edge, where the dock would cover
                // them. The check opens the process, so only redo it when the foreground changes.
                let foreground = GetForegroundWindow();
                if foreground != last_start_check {
                    last_start_check = foreground;
                    start_open = crate::commands::is_start_menu_window(foreground);
                    start_focused_at = now;
                }
                let root_fg = windows::Win32::UI::WindowsAndMessaging::GetAncestor(foreground, windows::Win32::UI::WindowsAndMessaging::GA_ROOT);
                let target_fg = if !root_fg.is_invalid() && !root_fg.0.is_null() { root_fg } else { foreground };
                let start_on_screen = start_open && IsWindowVisible(target_fg).as_bool() && {
                    let mut cloaked = 0u32;
                    DwmGetWindowAttribute(target_fg, windows::Win32::Graphics::Dwm::DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_err() || cloaked == 0
                };
                let start_shown = start_counts_as_shown(start_open, start_on_screen, now.duration_since(start_focused_at));
                let hide_on_start = crate::utils::get_setting_str(&handle_visibility, "fern-dock-hide-on-start")
                    .map(|v| v != "false")
                    .unwrap_or(true);
                let start_tap_monitor = if !hide_on_start
                    && !start_shown
                    && now_ms() - START_TAP_MS.load(Ordering::Relaxed) < START_TAP_GRACE_MS
                {
                    cursor_monitor_origin()
                } else {
                    None
                };
                let start_monitor = if start_shown { monitor_rect_for_hwnd(target_fg) } else { None };
                let start_on = |window: HWND| {
                    let win_mon = monitor_rect_for_hwnd(window);
                    let detected = match (start_monitor, win_mon) {
                        (Some((sx, sy, _, _)), Some((x, y, _, _))) => sx == x && sy == y,
                        _ => false,
                    };
                    let tapped = match (start_tap_monitor, win_mon) {
                        (Some((cx, cy)), Some((x, y, _, _))) => cx == x && cy == y,
                        _ => false,
                    };
                    detected || tapped
                };
                if now.duration_since(last_monitor_update) > Duration::from_millis(1000) {
                    if let Some(m) = handle_visibility.primary_monitor().ok().flatten() {
                        cached_scale = m.scale_factor();
                        last_monitor_update = now;
                    }
                }
                use windows::Win32::Graphics::Gdi::{MonitorFromRect, GetMonitorInfoA, MONITORINFO, MONITOR_DEFAULTTONULL};
                let settings_hwnd = handle_visibility.get_webview_window("settings").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize);
                let calendar_hwnd = handle_visibility.get_webview_window("calendar").and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize);
                use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, WS_EX_TOOLWINDOW};
                let mut hwnd = GetForegroundWindow();
                
                // Find the first meaningful window for overlap detection.
                let mut check_count = 0;
                let mut found_meaningful = false;
                while !hwnd.is_invalid() && check_count < 15 {
                    let mut process_id = 0u32;
                    GetWindowThreadProcessId(hwnd, Some(&mut process_id));
                    
                    let hwnd_val = hwnd.0 as isize;
                    let is_user_panel = settings_hwnd == Some(hwnd_val) || calendar_hwnd == Some(hwnd_val);
                    let is_fern = process_id == my_process_id && !is_user_panel;
                    let is_visible = IsWindowVisible(hwnd).as_bool();
                    let is_iconic = IsIconic(hwnd).as_bool();
                    
                    let mut cloaked = 0u32;
                    let is_cloaked = DwmGetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok() && cloaked != 0;

                    let mut rect = RECT::default();
                    let has_valid_rect = GetWindowRect(hwnd, &mut rect).is_ok()
                        && (rect.right - rect.left) > 0
                        && (rect.bottom - rect.top) > 0
                        && !MonitorFromRect(&rect, MONITOR_DEFAULTTONULL).is_invalid();
                    let is_bg_toolwindow = check_count > 0
                        && (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0) != 0;
                    
                    if is_fern || !is_visible || is_iconic || is_cloaked || !has_valid_rect || is_bg_toolwindow {
                        hwnd = windows::Win32::UI::WindowsAndMessaging::GetWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::GW_HWNDNEXT).unwrap_or_default();
                        check_count += 1;
                    } else {
                        found_meaningful = true;
                        break;
                    }
                }
                if !found_meaningful {
                    hwnd = HWND(std::ptr::null_mut());
                }

                let mut should_overlap = false;
                let mut should_notch_overlap = false;
                let mut should_maximized = false;
                // Foreground window's monitor, rect and whether it covers that monitor (maximized or fullscreen).
                let mut fg_info: Option<(RECT, RECT, bool)> = None;
                let mut current_is_fs = false;

                // Checked every tick, the foreground hwnd doesn't change on maximize so the adaptive dock
                // would wait for the 3s refresh.
                let fg_is_maximized = !hwnd.is_invalid() && IsZoomed(hwnd).as_bool();

                if !hwnd.is_invalid() && (hwnd != last_hwnd || fg_is_maximized != last_fg_maximized || last_emit.elapsed() >= Duration::from_secs(3)) {
                    last_hwnd = hwnd;
                    last_fg_maximized = fg_is_maximized;
                    let mut class_name = [0u8; 256];
                    let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameA(hwnd, &mut class_name);
                    let class_str = std::str::from_utf8(&class_name[..len as usize]).unwrap_or("");
                    let mut text = [0u16; 512];
                    let text_len = windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut text);
                    let title = String::from_utf16_lossy(&text[..text_len as usize]);

                    let is_desktop = class_str == "Progman" || class_str == "WorkerW";
                    let is_start = (hwnd == foreground && start_open)
                        || ((class_str == "Windows.UI.Core.CoreWindow" || class_str == "SimpleWindow")
                            && (title == "Start" || title == "Search"));
                    let is_shell = class_str == "Shell_TrayWnd" || class_str == "Shell_SecondaryTrayWnd" || is_start;
                    
                    is_known_shell = is_desktop || is_shell;
                    is_desktop_focused = is_desktop;
                }

                if !hwnd.is_invalid() {
                    if is_known_shell {
                        should_overlap = false;
                        current_is_fs = false;
                    } else {
                        // Current hwnd is now guaranteed to be visible, non-iconic and non-cloaked
                        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
                        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                        
                        let is_transient = (ex_style & WS_EX_TOOLWINDOW.0) != 0;
                        
                        if !is_transient {
                            let mut rect = RECT::default();
                            let dwm_res = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32);
                            let has_rect = dwm_res.is_ok() || GetWindowRect(hwnd, &mut rect).is_ok();

                            if has_rect {
                                // Use MONITOR_DEFAULTTONULL against the measured rect so windows mid-restore
                                // from Windows' off-screen minimized park (-32000, -32000) never get blamed on
                                // the nearest monitor (primary) while IsZoomed() is already true.
                                let h_monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONULL);
                                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                                
                                if !h_monitor.is_invalid() && GetMonitorInfoA(h_monitor, &mut mi).as_bool() && rect_intersects_monitor(rect, mi.rcMonitor) {
                                    let screen_rect = mi.rcMonitor;
                                    let is_maximized = IsZoomed(hwnd).as_bool() || (style & WS_MAXIMIZE.0) != 0;

                                    
                                    let mut is_client_fullscreen = false;
                                    let mut client_rect = RECT::default();
                                    if GetClientRect(hwnd, &mut client_rect).is_ok() {
                                        let mut top_left = POINT { x: client_rect.left, y: client_rect.top };
                                        let mut bottom_right = POINT { x: client_rect.right, y: client_rect.bottom };
                                        let _ = ClientToScreen(hwnd, &mut top_left);
                                        let _ = ClientToScreen(hwnd, &mut bottom_right);
                                        
                                        is_client_fullscreen = top_left.x <= screen_rect.left + 2 && top_left.y <= screen_rect.top + 2 &&
                                                               bottom_right.x >= screen_rect.right - 2 && bottom_right.y >= screen_rect.bottom - 2;
                                    }

                                    let is_matches_screen = rect.left <= screen_rect.left + 2 && rect.top <= screen_rect.top + 2 && 
                                                            rect.right >= screen_rect.right - 2 && rect.bottom >= screen_rect.bottom - 2;
                                    
                                    let is_d3d_fs = windows::Win32::UI::Shell::SHQueryUserNotificationState()
                                        .is_ok_and(|state| state == windows::Win32::UI::Shell::QUNS_RUNNING_D3D_FULL_SCREEN);

                                    current_is_fs = ((is_client_fullscreen || is_matches_screen)
                                        && !crate::utils::is_captioned_maximized(is_maximized, style))
                                        || is_d3d_fs;
                                    fg_info = Some((screen_rect, rect, current_is_fs || is_maximized));

                                    if current_is_fs || is_maximized {
                                        should_overlap = true;
                                        should_notch_overlap = true;
                                        // Maximized (not fullscreen) windows leave the dock strip empty, so it can stretch to a full taskbar.
                                        should_maximized = is_maximized && !current_is_fs;
                                    } else {
                                        should_overlap = false;
                                        if let Some(dr) = crate::state::dock_overlap_rect("dock") {
                                            should_overlap = rect_overlaps_dock(screen_rect.left, screen_rect.bottom, rect, dr, cached_scale);
                                        }

                                        if let Some(nr) = crate::state::notch_rects().lock().ok().and_then(|m| m.get("main").copied()) {
                                            should_notch_overlap = rect_overlaps_notch(screen_rect.left, screen_rect.top, rect, nr, cached_scale);
                                        }
                                    }
                                }
                            }
                        } else {
                            should_overlap = false;
                            current_is_fs = false;
                        }
                    }
                } else {
                    last_hwnd = hwnd;
                    should_overlap = false;
                    let is_d3d_fs = windows::Win32::UI::Shell::SHQueryUserNotificationState()
                        .is_ok_and(|state| state == windows::Win32::UI::Shell::QUNS_RUNNING_D3D_FULL_SCREEN);
                    current_is_fs = is_d3d_fs;
                    is_known_shell = false;
                    is_desktop_focused = false;
                }

                // Overlap only counts on the monitor the foreground window is on, unless another
                // monitor has a maximized/fullscreen/covering window in front on its z-order.
                let monitor_of = |label: &str| {
                    handle_visibility.get_webview_window(label)
                        .and_then(|w| w.hwnd().ok())
                        .and_then(monitor_rect_for_hwnd)
                };
                let fg_on = |label: &str| match (fg_info, monitor_of(label)) {
                    (Some((screen, _, _)), Some((mx, my, _, _))) => screen.left == mx && screen.top == my,
                    _ => true,
                };
                let focused_monitor = resolve_focused_monitor(
                    !hwnd.is_invalid(),
                    is_desktop_focused,
                    is_known_shell,
                    if is_known_shell { cursor_monitor_origin() } else { None },
                    fg_info.map(|(screen, _, _)| (screen.left, screen.top)),
                );
                let covers = scan_monitor_covers(my_process_id);
                let fullscreen_monitors: Vec<(i32, i32)> = covers.iter().filter(|c| c.fullscreen).map(|c| c.origin).collect();
                let unfocused_panel_overlap = |label: &str, is_dock: bool| -> Option<bool> {
                    let (mx, my, _, _) = monitor_of(label)?;
                    if focused_monitor == Some((mx, my)) {
                        return None;
                    }
                    let Some(cover) = covers.iter().find(|c| c.origin == (mx, my)) else { return Some(false) };
                    if cover.covered {
                        return Some(true);
                    }
                    Some(if is_dock {
                        crate::state::dock_overlap_rect(label)
                            .is_some_and(|dr| cover.rects.iter().any(|r| rect_overlaps_dock(mx, cover.bottom, *r, dr, cached_scale)))
                    } else {
                        crate::state::notch_rects().lock().ok().and_then(|m| m.get(label).copied())
                            .is_some_and(|nr| cover.rects.iter().any(|r| rect_overlaps_notch(mx, my, *r, nr, cached_scale)))
                    })
                };
                if !fg_on("dock") {
                    should_overlap = false;
                    should_maximized = false;
                }
                if !fg_on("main") {
                    should_notch_overlap = false;
                }
                if let Some(overlap) = unfocused_panel_overlap("dock", true) {
                    should_overlap = overlap;
                }
                if let Some(overlap) = unfocused_panel_overlap("main", false) {
                    should_notch_overlap = overlap;
                }
                should_notch_overlap = notch_overlap_during_start(start_shown, should_notch_overlap, last_notch_overlap.unwrap_or(should_notch_overlap));

                // Only report dock overlap when the dock window is actually visible.
                // On autostart, the overlap thread starts before init_dock shows the window,
                // and without this guard it emits dock-overlap:true which hides the dock
                // right after init_dock emits dock-overlap:false.
                let dock_visible = handle_visibility.get_webview_window("dock")
                    .is_some_and(|w| w.is_visible().unwrap_or(false) || crate::direct_flip::is_hidden("dock"));
                let effective_dock_overlap = should_overlap && dock_visible;

                crate::state::set_overlap(crate::state::dock_overlap(), "dock", if effective_dock_overlap { 1 } else { 0 });
                crate::state::set_overlap(crate::state::notch_overlap(), "main", if should_notch_overlap { 1 } else { 0 });
                CURRENT_FOREGROUND_FULLSCREEN.store(current_is_fs, Ordering::Relaxed);
                // Start counts as fullscreen for the dock when fern-dock-hide-on-start is enabled;
                // otherwise it reveals the dock on that monitor via dock-shell-surface.
                let start_on_dock = handle_visibility.get_webview_window("dock")
                    .and_then(|w| w.hwnd().ok())
                    .is_some_and(start_on);
                CURRENT_START_ON_DOCK.store(start_on_dock, Ordering::Relaxed);
                let fullscreen_on = |label: &str| match monitor_of(label) {
                    Some((mx, my, _, _)) => monitor_fullscreen((mx, my), focused_monitor, current_is_fs, &fullscreen_monitors),
                    None => current_is_fs && fg_on(label),
                };
                let fullscreen_on_dock = fullscreen_on("dock") || (hide_on_start && start_on_dock);
                crate::state::set_flag(crate::state::dock_fullscreen(), "dock", fullscreen_on_dock);
                CURRENT_FOREGROUND_FULLSCREEN_ON_DOCK.store(fullscreen_on_dock, Ordering::Relaxed);
                if Some(fullscreen_on_dock) != last_dock_fullscreen || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("dock", "dock-fullscreen", fullscreen_on_dock);
                    last_dock_fullscreen = Some(fullscreen_on_dock);
                }
                let shell_surface_on_dock = !hide_on_start && start_on_dock;
                if Some(shell_surface_on_dock) != last_dock_shell_surface || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("dock", "dock-shell-surface", shell_surface_on_dock);
                    last_dock_shell_surface = Some(shell_surface_on_dock);
                }
                let fullscreen_on_notch = fullscreen_on("main");
                crate::state::set_flag(crate::state::notch_fullscreen(), "main", fullscreen_on_notch);
                if Some(fullscreen_on_notch) != last_notch_fullscreen || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("main", "notch-fullscreen", fullscreen_on_notch);
                    last_notch_fullscreen = Some(fullscreen_on_notch);
                }

                if Some(effective_dock_overlap) != last_dock_overlap || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("dock", "dock-overlap", effective_dock_overlap);
                    last_dock_overlap = Some(effective_dock_overlap);
                    last_emit = Instant::now();
                }

                if Some(should_notch_overlap) != last_notch_overlap || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("main", "notch-overlap", should_notch_overlap);
                    last_notch_overlap = Some(should_notch_overlap);
                }

                // Docks and notches on other monitors get their own overlap state.
                for (label, win) in handle_visibility.webview_windows() {
                    let is_dock = is_dock_label(&label);
                    if !(is_dock || is_notch_label(&label)) || crate::state::monitor_suffix(&label).is_none() {
                        continue;
                    }
                    let Ok(w_hwnd) = win.hwnd() else { continue };
                    let Some((mx, my, _mw, _mh)) = monitor_rect_for_hwnd(w_hwnd) else { continue };
                    {
                        let fullscreen_here = monitor_fullscreen((mx, my), focused_monitor, current_is_fs, &fullscreen_monitors)
                            || (is_dock && hide_on_start && start_on(w_hwnd));
                        if is_dock {
                            crate::state::set_flag(crate::state::dock_fullscreen(), &label, fullscreen_here);
                        } else {
                            crate::state::set_flag(crate::state::notch_fullscreen(), &label, fullscreen_here);
                        }
                        let needs_update = last_secondary_dock_fullscreen.get(&label)
                            .is_none_or(|(previous, sent_at)| *previous != fullscreen_here || sent_at.elapsed() >= Duration::from_secs(3));
                        if needs_update {
                            let event = if is_dock { "dock-fullscreen" } else { "notch-fullscreen" };
                            let _ = handle_visibility.emit_to(label.as_str(), event, fullscreen_here);
                            last_secondary_dock_fullscreen.insert(label.clone(), (fullscreen_here, now));
                        }
                        if is_dock {
                            let shell_here = !hide_on_start && start_on(w_hwnd);
                            let needs_shell_update = last_secondary_dock_shell_surface.get(&label)
                                .is_none_or(|(previous, sent_at)| *previous != shell_here || sent_at.elapsed() >= Duration::from_secs(3));
                            if needs_shell_update {
                                let _ = handle_visibility.emit_to(label.as_str(), "dock-shell-surface", shell_here);
                                last_secondary_dock_shell_surface.insert(label.clone(), (shell_here, now));
                            }
                        }
                    }
                    let overlap = match fg_info {
                        Some((screen, rect, covers)) if screen.left == mx && screen.top == my => {
                            if covers {
                                true
                            } else if is_dock {
                                crate::state::dock_overlap_rect(&label).is_some_and(|dr| {
                                    rect_overlaps_dock(mx, screen.bottom, rect, dr, cached_scale)
                                })
                            } else {
                                crate::state::notch_rects().lock().ok().and_then(|m| m.get(&label).copied()).is_some_and(|nr| {
                                    rect_overlaps_notch(mx, screen.top, rect, nr, cached_scale)
                                })
                            }
                        }
                        // Another monitor has focus: a maximized or fullscreen window there still covers the panel.
                        _ => unfocused_panel_overlap(label.as_str(), is_dock).unwrap_or(false),
                    };
                    // Same visibility guard as the primary dock.
                    let overlap = overlap && (!is_dock || win.is_visible().unwrap_or(false) || crate::direct_flip::is_hidden(&label));
                    let map = if is_dock { crate::state::dock_overlap() } else { crate::state::notch_overlap() };
                    let overlap = if is_dock {
                        overlap
                    } else {
                        notch_overlap_during_start(start_shown, overlap, crate::state::get_overlap(map, &label) == 1)
                    };
                    let new_val = if overlap { 1 } else { 0 };
                    let changed = crate::state::get_overlap(map, &label) != new_val;
                    if changed {
                        crate::state::set_overlap(map, &label, new_val);
                    }
                    let needs_overlap_emit = changed
                        || last_secondary_overlap.get(&label)
                            .is_none_or(|(previous, sent_at)| *previous != overlap || sent_at.elapsed() >= Duration::from_secs(3));
                    if needs_overlap_emit {
                        let event = if is_dock { "dock-overlap" } else { "notch-overlap" };
                        let _ = handle_visibility.emit_to(label.as_str(), event, overlap);
                        last_secondary_overlap.insert(label.clone(), (overlap, now));
                    }
                }

                // Maximized signal for the adaptive dock. Not tied to dock visibility so init_dock can send the current value.
                CURRENT_FOREGROUND_MAXIMIZED.store(should_maximized, Ordering::Relaxed);
                if Some(should_maximized) != last_dock_maximized || last_emit.elapsed() >= Duration::from_secs(3) {
                    let _ = handle_visibility.emit_to("dock", "dock-maximized", should_maximized);
                    last_dock_maximized = Some(should_maximized);
                }

                // Update full-screen visibility (hides primary TopBar/Corners only when primary monitor is fullscreen)
                if fullscreen_on_notch && last_visible {
                    let _ = tx_clone.send(SystemCommand::ToggleVisibility(false));
                    last_visible = false;
                } else if !fullscreen_on_notch && !last_visible {
                    let _ = tx_clone.send(SystemCommand::ToggleVisibility(true));
                    last_visible = true;
                }

                // DWM Direct-Flip Protection: natively SW_HIDE Smart/Peek panels & HUD overlay on
                // monitors covered by a real fullscreen window (never Start menu), and relax Fern's
                // background polling loops while fullscreen is active.
                let df_enabled = crate::direct_flip::is_enabled(&handle_visibility);
                if !df_enabled {
                    crate::direct_flip::restore_all(&handle_visibility);
                    crate::memory_saver::set_fullscreen_relaxed(false);
                } else {
                    let kb_focus_label = crate::state::KEYBOARD_FOCUS_LABEL
                        .lock()
                        .ok()
                        .and_then(|g| g.clone());
                    let dock_busy = crate::state::MENU_IS_OPEN.load(Ordering::Relaxed)
                        || crate::state::DOCK_IS_DRAGGING.load(Ordering::Relaxed);
                    let mut any_real_fullscreen = fullscreen_on_notch || fullscreen_on("dock") || current_is_fs;

                    if let Some(win) = handle_visibility.get_webview_window("main") {
                        let mode = crate::monitors::resolve_mode_for_label(&handle_visibility, crate::types::WindowKind::Notch, "main");
                        let should_hide = crate::direct_flip::should_hide_panel(
                            true,
                            fullscreen_on_notch,
                            &mode,
                            kb_focus_label.as_deref() == Some("main"),
                            false,
                        );
                        crate::direct_flip::sync_window(&win, "main", should_hide);
                    }

                    if let Some(win) = handle_visibility.get_webview_window("dock") {
                        let real_fs_on_dock = fullscreen_on("dock");
                        let mode = crate::monitors::resolve_mode_for_label(&handle_visibility, crate::types::WindowKind::Dock, "dock");
                        let should_hide = crate::direct_flip::should_hide_panel(
                            true,
                            real_fs_on_dock,
                            &mode,
                            kb_focus_label.as_deref() == Some("dock"),
                            dock_busy,
                        );
                        crate::direct_flip::sync_window(&win, "dock", should_hide);
                    }

                    for (label, win) in handle_visibility.webview_windows() {
                        let is_dock = is_dock_label(&label);
                        if !(is_dock || is_notch_label(&label)) || crate::state::monitor_suffix(&label).is_none() {
                            continue;
                        }
                        let Ok(w_hwnd) = win.hwnd() else { continue };
                        let Some((mx, my, _, _)) = monitor_rect_for_hwnd(w_hwnd) else { continue };
                        let real_fs_here = monitor_fullscreen((mx, my), focused_monitor, current_is_fs, &fullscreen_monitors);
                        if real_fs_here {
                            any_real_fullscreen = true;
                        }
                        let kind = if is_dock { crate::types::WindowKind::Dock } else { crate::types::WindowKind::Notch };
                        let mode = crate::monitors::resolve_mode_for_label(&handle_visibility, kind, &label);
                        let should_hide = crate::direct_flip::should_hide_panel(
                            true,
                            real_fs_here,
                            &mode,
                            kb_focus_label.as_deref() == Some(label.as_str()),
                            is_dock && dock_busy,
                        );
                        crate::direct_flip::sync_window(&win, &label, should_hide);
                    }

                    if let Some(ov_win) = handle_visibility.get_webview_window("overlay") {
                        let should_hide = crate::direct_flip::should_hide_overlay(
                            true,
                            fullscreen_on_notch,
                            crate::state::OVERLAY_IN_SPLASH.load(Ordering::Relaxed),
                        );
                        crate::direct_flip::sync_window(&ov_win, "overlay", should_hide);
                    }

                    crate::memory_saver::set_fullscreen_relaxed(any_real_fullscreen);
                }

                // Enforce native taskbar hiding (periodic check)
                if NATIVE_TASKBAR_HIDDEN.load(Ordering::Relaxed) {
                    use windows::Win32::UI::WindowsAndMessaging::{FindWindowA, IsWindowVisible};
                    let tray_class = windows::core::PCSTR(c"Shell_TrayWnd".as_ptr() as *const u8);

                    let mut should_rehide = false;
                    if let Ok(tray_hwnd) = FindWindowA(tray_class, windows::core::PCSTR::null()) {
                        if IsWindowVisible(tray_hwnd).as_bool() { should_rehide = true; }
                    }
                    if !should_rehide {
                        should_rehide = crate::utils::secondary_taskbars().iter().any(|&hwnd| IsWindowVisible(hwnd).as_bool());
                    }

                    if should_rehide {
                        set_taskbar_visibility(false, false);
                    }
                }
            }

            // Capture UI state is event-driven: WinEvent callbacks set the
            // recheck flag on top-level window show/hide/destroy and foreground
            // changes, so the scan below only runs when something changed
            // (throttled to coalesce bursts of window events).
            let scan_now = now_ms();
            if CAPTURE_RECHECK.load(Ordering::Relaxed)
                && scan_now - CAPTURE_LAST_SCAN_MS.load(Ordering::Relaxed) >= 400
            {
                CAPTURE_RECHECK.store(false, Ordering::Relaxed);
                CAPTURE_LAST_SCAN_MS.store(scan_now, Ordering::Relaxed);
                let active = is_capture_ui_present();
                if active != CAPTURE_UI_ACTIVE.load(Ordering::Relaxed) {
                    CAPTURE_UI_ACTIVE.store(active, Ordering::Relaxed);
                    apply_capture_ui_state(&handle_visibility, active);
                }
            }
            std::thread::park_timeout(crate::memory_saver::poll_interval_idle_only(std::time::Duration::from_millis(150)));
        }
    });

    tx
}



/// Start takes the foreground a moment before it appears, and judging the windows under it in that
/// moment would flash the dock. But its window can also stay foreground after closing, off screen,
/// so this only holds for a moment or the dock could stay hidden with Start gone.
fn start_counts_as_shown(start_open: bool, on_screen: bool, since_focused: Duration) -> bool {
    const APPEARING: Duration = Duration::from_millis(1000);
    start_open && (on_screen || since_focused < APPEARING)
}

/// Start doesn't reach the top of the screen, so it has no say over the notch. Judged like any other
/// window it would count as "nothing overlaps", and a Smart notch hidden by a maximized window would
/// come down for as long as Start is open.
fn notch_overlap_during_start(start_shown: bool, computed: bool, last: bool) -> bool {
    if start_shown { last } else { computed }
}

fn should_retry_artwork(attempts: u32) -> bool {
    attempts < 8 || (attempts - 8) % 15 == 0
}

fn cursor_monitor_origin() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoA, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    unsafe {
        let mut pt = POINT::default();
        if GetCursorPos(&mut pt).is_err() {
            return None;
        }
        let h_monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoA(h_monitor, &mut mi).as_bool() {
            Some((mi.rcMonitor.left, mi.rcMonitor.top))
        } else {
            None
        }
    }
}

/// What the z-order shows on one monitor: the windows down to (and including) the first maximized
/// or monitor-sized one, which hides everything below it.
struct MonitorCover {
    origin: (i32, i32),
    bottom: i32,
    rects: Vec<windows::Win32::Foundation::RECT>,
    covered: bool,
    fullscreen: bool,
}

struct CoverScan {
    own_pid: u32,
    monitors: Vec<MonitorCover>,
}

/// True when a window rect actually intersects the monitor's bounds. Filters out
/// Windows' off-screen minimized park around (-32000, -32000) during SW_RESTORE.
fn rect_intersects_monitor(rect: windows::Win32::Foundation::RECT, screen: windows::Win32::Foundation::RECT) -> bool {
    rect.left < screen.right
        && rect.right > screen.left
        && rect.top < screen.bottom
        && rect.bottom > screen.top
}

/// True when a window should be ignored during z-order monitor cover scanning:
/// our own process windows, tool windows, and click-through transparent overlays.
fn should_skip_cover_window(own_pid: u32, pid: u32, ex_style: u32) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT};
    pid == own_pid || (ex_style & (WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0)) != 0
}

/// True when `rect` covers `screen` within a 2px DPI rounding tolerance (matching `is_window_fullscreen`).
fn rect_covers_monitor(rect: windows::Win32::Foundation::RECT, screen: windows::Win32::Foundation::RECT) -> bool {
    rect.left <= screen.left + 2
        && rect.top <= screen.top + 2
        && rect.right >= screen.right - 2
        && rect.bottom >= screen.bottom - 2
}

unsafe extern "system" fn cover_scan_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Dwm::DWMWA_CLOAKED;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoA, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONULL};
    use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowRect, IsIconic, IsZoomed};
    let scan = &mut *(lparam.0 as *mut CoverScan);
    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        return true.into();
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if should_skip_cover_window(scan.own_pid, pid, ex_style) {
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
    let mut rect = RECT::default();
    let has_rect = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut rect as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_ok()
        || GetWindowRect(hwnd, &mut rect).is_ok();
    if !has_rect {
        return true.into();
    }
    let monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONULL);
    if monitor.is_invalid() {
        return true.into();
    }
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if !GetMonitorInfoA(monitor, &mut mi).as_bool() {
        return true.into();
    }
    let screen = mi.rcMonitor;
    if !rect_intersects_monitor(rect, screen) {
        return true.into();
    }
    let origin = (screen.left, screen.top);
    let index = match scan.monitors.iter().position(|m| m.origin == origin) {
        Some(i) => i,
        None => {
            scan.monitors.push(MonitorCover { origin, bottom: screen.bottom, rects: Vec::new(), covered: false, fullscreen: false });
            scan.monitors.len() - 1
        }
    };
    let entry = &mut scan.monitors[index];
    if entry.covered {
        return true.into();
    }
    entry.rects.push(rect);
    // Small windows (popups, picture-in-picture) above a fullscreen app don't decide whether the
    // monitor is fullscreen; a maximized or monitor-sized window does.
    let covers = rect_covers_monitor(rect, screen);
    if covers || IsZoomed(hwnd).as_bool() {
        entry.covered = true;
        entry.fullscreen = crate::utils::is_window_fullscreen(hwnd);
    }
    true.into()
}

/// Per monitor, the windows that are not hidden behind a maximized or monitor-sized one. Read from
/// the z-order rather than focus, so a window keeps its monitor's panels hidden while another
/// monitor has focus.
fn scan_monitor_covers(own_pid: u32) -> Vec<MonitorCover> {
    use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
    let mut scan = CoverScan { own_pid, monitors: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(cover_scan_proc), LPARAM(&mut scan as *mut CoverScan as isize));
    }
    scan.monitors
}

/// Monitor whose overlap and fullscreen state follow the foreground window rather than the
/// z-order scan. Returns `None` unless the foreground window was actually measured (`fg_info`)
/// or is a known shell surface, so transient/unmeasured windows never claim a monitor.
fn resolve_focused_monitor(
    hwnd_valid: bool,
    is_desktop_focused: bool,
    is_known_shell: bool,
    cursor_origin: Option<(i32, i32)>,
    fg_screen_origin: Option<(i32, i32)>,
) -> Option<(i32, i32)> {
    if !hwnd_valid || is_desktop_focused {
        None
    } else if is_known_shell {
        cursor_origin
    } else {
        fg_screen_origin
    }
}

/// Fullscreen state of the monitor at `origin`. The focused monitor keeps the foreground check;
/// the others use the z-order scan.
fn monitor_fullscreen(origin: (i32, i32), focused_monitor: Option<(i32, i32)>, focused_is_fullscreen: bool, covered: &[(i32, i32)]) -> bool {
    if focused_monitor == Some(origin) { focused_is_fullscreen } else { covered.contains(&origin) }
}

fn rect_overlaps_dock(
    monitor_left: i32,
    monitor_bottom: i32,
    fg_rect: windows::Win32::Foundation::RECT,
    dr: crate::types::IntRect,
    scale: f64,
) -> bool {
    let d_left = monitor_left + (dr.x as f64 * scale) as i32;
    let d_right = d_left + (dr.width as f64 * scale) as i32;
    let trigger_y = monitor_bottom - (56.0 * scale) as i32;
    fg_rect.left < d_right - 4 && fg_rect.right > d_left + 4 && fg_rect.bottom > trigger_y + 4
}

fn rect_overlaps_notch(
    monitor_left: i32,
    monitor_top: i32,
    fg_rect: windows::Win32::Foundation::RECT,
    nr: crate::types::IntRect,
    scale: f64,
) -> bool {
    if nr.width <= 0 || nr.height <= 0 {
        return false;
    }
    let n_left = monitor_left + (nr.x as f64 * scale) as i32;
    let n_right = n_left + (nr.width as f64 * scale) as i32;
    let trigger_y = monitor_top + ((nr.y.max(0) + 36) as f64 * scale) as i32;
    fg_rect.left < n_right - 4 && fg_rect.right > n_left + 4 && fg_rect.top < trigger_y - 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn start_leaves_the_notch_as_it_was() {
        assert!(notch_overlap_during_start(true, false, true), "hidden stays hidden");
        assert!(!notch_overlap_during_start(true, true, false), "visible stays visible");
        assert!(notch_overlap_during_start(false, true, false), "without Start the computed value counts");
        assert!(!notch_overlap_during_start(false, false, true));
    }

    #[test]
    fn start_counts_while_opening_but_not_forever() {
        let just_now = Duration::from_millis(100);
        let long_ago = Duration::from_millis(1000);
        assert!(start_counts_as_shown(true, true, long_ago), "on screen");
        assert!(start_counts_as_shown(true, false, just_now), "focused but still appearing");
        assert!(!start_counts_as_shown(true, false, long_ago), "focused but closed, the dock must come back");
        assert!(!start_counts_as_shown(false, true, just_now), "not Start");
    }

    #[test]
    fn primary_overlap_helpers_account_for_monitor_origin() {
        use super::{rect_overlaps_dock, rect_overlaps_notch};
        use crate::types::IntRect;
        use windows::Win32::Foundation::RECT;

        let dock_rect = IntRect { x: 600, y: 0, width: 720, height: 60 };
        // Specific monitor placed at x=1920..3840
        let fg_on_right = RECT { left: 2600, top: 800, right: 3100, bottom: 1060 };
        assert!(rect_overlaps_dock(1920, 1080, fg_on_right, dock_rect, 1.0));
        // Window only on primary monitor (0..1920) must not overlap dock on monitor at x=1920
        let fg_on_primary = RECT { left: 600, top: 800, right: 1300, bottom: 1060 };
        assert!(!rect_overlaps_dock(1920, 1080, fg_on_primary, dock_rect, 1.0));

        let notch_rect = IntRect { x: 760, y: 0, width: 400, height: 36 };
        let fg_top_right = RECT { left: 2700, top: 10, right: 2900, bottom: 500 };
        assert!(rect_overlaps_notch(1920, 0, fg_top_right, notch_rect, 1.0));
        assert!(!rect_overlaps_notch(1920, 0, fg_on_primary, notch_rect, 1.0));
    }

    #[test]
    fn smtc_artwork_retry_backs_off_after_eight_attempts() {
        use super::should_retry_artwork;
        for a in 0..8 {
            assert!(should_retry_artwork(a), "attempt {a} should retry");
        }
        assert!(should_retry_artwork(8));
        for a in 9..23 {
            assert!(!should_retry_artwork(a), "attempt {a} should skip");
        }
        assert!(should_retry_artwork(23));
    }

    #[test]
    fn media_session_selection_prefers_current_and_fresh_over_stale_first() {
        use super::{choose_media_session_index, MediaSessionCandidate};

        let stale = MediaSessionCandidate {
            index: 0,
            source_id: "Brave".into(),
            title: "Stale Video".into(),
            is_playing: true,
            last_updated_ticks: 100,
        };
        let active = MediaSessionCandidate {
            index: 1,
            source_id: "Brave".into(),
            title: "Current Video".into(),
            is_playing: true,
            last_updated_ticks: 500,
        };
        let paused_spotify = MediaSessionCandidate {
            index: 2,
            source_id: "Spotify.exe".into(),
            title: "Paused Song".into(),
            is_playing: false,
            last_updated_ticks: 900,
        };
        let entries = vec![stale, active, paused_spotify];

        // When Windows GetCurrentSession points to index 1, choose it over stale index 0
        assert_eq!(
            choose_media_session_index(&entries, None, "Brave", "Stale Video", Some(("Brave", "Current Video"))),
            Some(1)
        );
        // Even with SELECTED_MEDIA_SOURCE="Brave", prefer current/fresh playing session
        assert_eq!(
            choose_media_session_index(&entries, Some("Brave"), "Brave", "Stale Video", Some(("Brave", "Current Video"))),
            Some(1)
        );
        // Without GetCurrentSession, fresher last_updated_ticks wins among playing sessions
        assert_eq!(
            choose_media_session_index(&entries, None, "Brave", "Stale Video", None),
            Some(1)
        );
        // When selected source is paused but has a playing entry vs paused entry, playing wins
        let mixed = vec![
            MediaSessionCandidate {
                index: 0,
                source_id: "Brave".into(),
                title: "Paused Stale".into(),
                is_playing: false,
                last_updated_ticks: 100,
            },
            MediaSessionCandidate {
                index: 1,
                source_id: "Brave".into(),
                title: "Playing Now".into(),
                is_playing: true,
                last_updated_ticks: 200,
            },
        ];
        assert_eq!(
            choose_media_session_index(&mixed, Some("Brave"), "", "", None),
            Some(1)
        );
    }

    #[test]
    fn fullscreen_monitor_stays_hidden_when_focus_moves_to_another_monitor() {
        use super::monitor_fullscreen;
        let covered = [(0, 0)];
        assert!(monitor_fullscreen((0, 0), Some((1920, 0)), false, &covered));
        assert!(!monitor_fullscreen((1920, 0), Some((1920, 0)), false, &covered));
    }

    #[test]
    fn focused_monitor_follows_the_foreground_window() {
        use super::monitor_fullscreen;
        assert!(!monitor_fullscreen((0, 0), Some((0, 0)), false, &[(0, 0)]));
        assert!(monitor_fullscreen((0, 0), Some((0, 0)), true, &[]));
        assert!(!monitor_fullscreen((0, 0), None, false, &[]));
    }

    #[test]
    fn resolve_focused_monitor_never_claims_monitor_for_unmeasured_window() {
        use super::resolve_focused_monitor;

        // Invalid HWND -> None (all monitors fall back to z-order scan).
        assert_eq!(resolve_focused_monitor(false, false, false, Some((0, 0)), Some((0, 0))), None);
        // Desktop focused -> None.
        assert_eq!(resolve_focused_monitor(true, true, true, Some((0, 0)), Some((0, 0))), None);
        // Known shell (Start/Taskbar) -> cursor monitor origin.
        assert_eq!(resolve_focused_monitor(true, false, true, Some((1920, 0)), None), Some((1920, 0)));
        // Transient/unmeasured window (fg_info is None) -> None (must NOT claim primary (0, 0)).
        assert_eq!(resolve_focused_monitor(true, false, false, Some((0, 0)), None), None);
        // Measured foreground window -> its monitor origin.
        assert_eq!(resolve_focused_monitor(true, false, false, Some((0, 0)), Some((1920, 0))), Some((1920, 0)));
    }

    #[test]
    fn off_screen_minimized_park_rect_does_not_intersect_any_monitor() {
        use super::rect_intersects_monitor;
        use windows::Win32::Foundation::RECT;

        let primary = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        let secondary = RECT { left: 1920, top: 0, right: 3840, bottom: 1080 };
        let parked = RECT { left: -31992, top: -32000, right: -31848, bottom: -31980 };

        assert!(!rect_intersects_monitor(parked, primary));
        assert!(!rect_intersects_monitor(parked, secondary));
        assert!(rect_intersects_monitor(RECT { left: 1920, top: 0, right: 3840, bottom: 1080 }, secondary));
        assert!(rect_intersects_monitor(RECT { left: -100, top: 100, right: 200, bottom: 400 }, primary));
        // Touching edge only (zero overlap area) does not intersect.
        assert!(!rect_intersects_monitor(RECT { left: 1920, top: 0, right: 2500, bottom: 600 }, primary));
    }

    #[test]
    fn window_over_the_panel_strip_counts_as_overlap_on_any_monitor() {
        use super::{rect_overlaps_dock, rect_overlaps_notch};
        use windows::Win32::Foundation::RECT;

        let dock = crate::types::IntRect { x: 880, y: 0, width: 800, height: 64 };
        let near_bottom = RECT { left: 2560 + 1000, top: 600, right: 2560 + 1600, bottom: 1420 };
        let above_dock = RECT { left: 2560 + 1000, top: 200, right: 2560 + 1600, bottom: 900 };
        let beside_dock = RECT { left: 2560 + 100, top: 600, right: 2560 + 600, bottom: 1420 };
        assert!(rect_overlaps_dock(2560, 1440, near_bottom, dock, 1.0));
        assert!(!rect_overlaps_dock(2560, 1440, above_dock, dock, 1.0));
        assert!(!rect_overlaps_dock(2560, 1440, beside_dock, dock, 1.0));

        let notch = crate::types::IntRect { x: 1080, y: 0, width: 400, height: 40 };
        let at_top = RECT { left: 2560 + 1200, top: 0, right: 2560 + 1800, bottom: 700 };
        let lower = RECT { left: 2560 + 1200, top: 300, right: 2560 + 1800, bottom: 900 };
        assert!(rect_overlaps_notch(2560, 0, at_top, notch, 1.0));
        assert!(!rect_overlaps_notch(2560, 0, lower, notch, 1.0));
    }

    #[test]
    fn cover_scan_skips_transparent_overlays_and_tolerates_dpi_rounding() {
        use super::{rect_covers_monitor, should_skip_cover_window};
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::{WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT};

        assert!(should_skip_cover_window(100, 100, 0));
        assert!(should_skip_cover_window(100, 200, WS_EX_TOOLWINDOW.0));
        assert!(should_skip_cover_window(100, 200, WS_EX_TRANSPARENT.0));
        assert!(!should_skip_cover_window(100, 200, 0));

        let screen = RECT { left: 1920, top: 0, right: 3840, bottom: 1080 };
        // Exact cover and 2px fractional-DPI inset both count as covering.
        assert!(rect_covers_monitor(screen, screen));
        assert!(rect_covers_monitor(
            RECT { left: 1922, top: 2, right: 3838, bottom: 1078 },
            screen
        ));
        // Smaller window (e.g. inset by 3px) does not cover.
        assert!(!rect_covers_monitor(
            RECT { left: 1923, top: 0, right: 3840, bottom: 1080 },
            screen
        ));
    }
}
