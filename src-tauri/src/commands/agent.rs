use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_dialog::DialogExt;
use windows::Win32::Foundation::HWND;

#[tauri::command]
pub async fn focus_agent_window(window: u64) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_MENU};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, IsIconic, IsWindow, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };
    unsafe {
        let hwnd = HWND(window as usize as *mut _);
        if !IsWindow(Some(hwnd)).as_bool() {
            return;
        }
        let _ = ShowWindow(hwnd, if IsIconic(hwnd).as_bool() { SW_RESTORE } else { SW_SHOW });
        keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
        keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let current = GetCurrentThreadId();
        let attached = foreground != 0 && foreground != current && AttachThreadInput(current, foreground, true).as_bool();
        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attached {
            let _ = AttachThreadInput(current, foreground, false);
        }
    }
}

#[tauri::command]
pub fn agent_answer(id: u64, allow: bool, choices: Option<Vec<usize>>) {
    crate::agent_api::answer(id, allow, choices.unwrap_or_default());
}

#[tauri::command]
pub fn get_agent_integrations_status() -> Vec<crate::agent_api::IntegrationStatus> {
    crate::agent_api::integrations_status()
}

#[tauri::command]
pub async fn sync_agent_integration(id: String, enable: bool) -> Vec<crate::agent_api::IntegrationStatus> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::agent_api::sync_single_integration(&id, enable);
        crate::agent_api::integrations_status()
    })
    .await
    .unwrap_or_default()
}

const AGENT_SOUND_STATES: [&str; 3] = ["needs_input", "done", "error"];

fn agent_sound_files(state: &str) -> Vec<std::path::PathBuf> {
    let Some(dir) = crate::agent_api::data_dir().map(|d| d.join("sounds")) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_stem().and_then(|s| s.to_str()) == Some(state))
        .collect()
}

#[tauri::command]
pub async fn pick_agent_sound(app: AppHandle, state: String) -> Result<bool, String> {
    use tauri_plugin_dialog::DialogExt;
    const MAX_SOUND_BYTES: u64 = 3 * 1024 * 1024;
    if !AGENT_SOUND_STATES.contains(&state.as_str()) {
        return Err("Unknown state".into());
    }
    let Some(path) = app
        .dialog()
        .file()
        .set_title("Choose a sound")
        .add_filter("Audio", &["mp3", "wav", "ogg", "m4a"])
        .blocking_pick_file()
    else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > MAX_SOUND_BYTES {
        return Err("Sound is too large (maximum 3MB)".into());
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !["mp3", "wav", "ogg", "m4a"].contains(&ext.as_str()) {
        return Err("Unsupported file type".into());
    }
    let dir = crate::agent_api::data_dir().ok_or("No data folder")?.join("sounds");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for old in agent_sound_files(&state) {
        let _ = std::fs::remove_file(old);
    }
    std::fs::copy(&path, dir.join(format!("{state}.{ext}"))).map_err(|e| e.to_string())?;
    let _ = app.emit("agent-sound-changed", ());
    Ok(true)
}

#[tauri::command]
pub async fn reset_agent_sound(app: AppHandle, state: String) {
    for old in agent_sound_files(&state) {
        let _ = std::fs::remove_file(old);
    }
    let _ = app.emit("agent-sound-changed", ());
}

#[tauri::command]
pub async fn agent_sound_data(state: String) -> Option<String> {
    use base64::Engine;
    let path = agent_sound_files(&state).into_iter().next()?;
    let mime = match path.extension()?.to_str()? {
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        _ => return None,
    };
    let bytes = std::fs::read(path).ok()?;
    Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

#[tauri::command]
pub async fn agent_sound_custom() -> Vec<String> {
    AGENT_SOUND_STATES.iter().filter(|s| !agent_sound_files(s).is_empty()).map(|s| s.to_string()).collect()
}
