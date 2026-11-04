use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_MENU, VK_SPACE, VK_S,
};

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct SearchLauncherInfo {
    pub id: String,
    pub name: String,
    pub hotkey: String,
    pub is_available: bool,
}

pub fn send_alt_chord(key: VIRTUAL_KEY) {
    let input = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { Default::default() },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [
        input(VK_MENU, false),
        input(key, false),
        input(key, true),
        input(VK_MENU, true),
    ];
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
}

fn is_process_running(target_exe: &str) -> bool {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return false;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            let target_lower = target_exe.to_lowercase();
            loop {
                let name = String::from_utf16_lossy(
                    &entry.szExeFile[..entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len())],
                ).to_lowercase();
                if name == target_lower {
                    let _ = windows::Win32::Foundation::CloseHandle(snapshot);
                    return true;
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snapshot);
        false
    }
}

#[tauri::command]
pub fn get_search_launchers() -> Vec<SearchLauncherInfo> {
    let local_app_data = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let program_files = std::env::var("ProgramFiles").unwrap_or_default();
    let program_files_x86 = std::env::var("ProgramFiles(x86)").unwrap_or_default();

    let powertoys_paths = [
        format!(r"{program_files}\PowerToys\PowerToys.PowerLauncher.exe"),
        format!(r"{local_app_data}\Microsoft\PowerToys\PowerToys.PowerLauncher.exe"),
    ];
    let raycast_paths = [
        format!(r"{local_app_data}\Programs\Raycast\Raycast.exe"),
    ];
    let flow_paths = [
        format!(r"{local_app_data}\FlowLauncher\Flow.Launcher.exe"),
    ];
    let everything_paths = [
        format!(r"{program_files}\Everything\Everything.exe"),
        format!(r"{program_files_x86}\Everything\Everything.exe"),
    ];
    let listary_paths = [
        format!(r"{program_files}\Listary\Listary.exe"),
        format!(r"{program_files_x86}\Listary\Listary.exe"),
    ];

    let powertoys_available = is_process_running("PowerToys.PowerLauncher.exe")
        || is_process_running("PowerToys.exe")
        || powertoys_paths.iter().any(|p| std::path::Path::new(p).exists());

    let raycast_available = is_process_running("Raycast.exe")
        || raycast_paths.iter().any(|p| std::path::Path::new(p).exists());

    let flow_available = is_process_running("Flow.Launcher.exe")
        || flow_paths.iter().any(|p| std::path::Path::new(p).exists());

    let everything_available = is_process_running("Everything.exe")
        || everything_paths.iter().any(|p| std::path::Path::new(p).exists());

    let listary_available = is_process_running("Listary.exe")
        || listary_paths.iter().any(|p| std::path::Path::new(p).exists());

    vec![
        SearchLauncherInfo {
            id: "windows".to_string(),
            name: "Windows Arama".to_string(),
            hotkey: "Win + S".to_string(),
            is_available: true,
        },
        SearchLauncherInfo {
            id: "powertoys".to_string(),
            name: "PowerToys Run".to_string(),
            hotkey: "Alt + Boşluk".to_string(),
            is_available: powertoys_available,
        },
        SearchLauncherInfo {
            id: "raycast".to_string(),
            name: "Raycast".to_string(),
            hotkey: "Alt + Boşluk".to_string(),
            is_available: raycast_available,
        },
        SearchLauncherInfo {
            id: "flow".to_string(),
            name: "Flow Launcher".to_string(),
            hotkey: "Alt + Boşluk".to_string(),
            is_available: flow_available,
        },
        SearchLauncherInfo {
            id: "everything".to_string(),
            name: "Everything".to_string(),
            hotkey: "Hızlı Dosya Arama".to_string(),
            is_available: everything_available,
        },
        SearchLauncherInfo {
            id: "listary".to_string(),
            name: "Listary".to_string(),
            hotkey: "Ctrl + Ctrl / Alt + Boşluk".to_string(),
            is_available: listary_available,
        },
    ]
}

#[tauri::command]
pub fn trigger_search_launcher(launcher_id: Option<String>) {
    let launcher = launcher_id.unwrap_or_else(|| "windows".to_string());
    match launcher.to_lowercase().as_str() {
        "powertoys" | "raycast" | "flow" | "listary" => {
            // Modern desktop launchers default to Alt+Space
            send_alt_chord(VK_SPACE);
        }
        "everything" => {
            let program_files = std::env::var("ProgramFiles").unwrap_or_default();
            let everything_exe = format!(r"{program_files}\Everything\Everything.exe");
            if std::path::Path::new(&everything_exe).exists() {
                let _ = std::process::Command::new(&everything_exe).spawn();
            } else {
                let _ = std::process::Command::new("explorer.exe").arg("everything:").spawn();
            }
        }
        _ => {
            // Default to Windows Search (Win + S)
            crate::commands::send_win_chord(VK_S);
        }
    }
}
