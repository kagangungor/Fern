pub mod everything;

use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use windows::core::PCWSTR;
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, REG_SZ,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

#[derive(serde::Serialize, Clone, Debug, PartialEq, Eq)]
pub struct FileSearchResult {
    pub name: String,
    pub path: String,
    pub parent_dir: String,
    pub is_dir: bool,
    pub extension: Option<String>,
}

#[derive(serde::Serialize, Clone, Debug, PartialEq, Eq)]
pub struct BrowserInfo {
    pub name: String,
    pub path: String,
}

pub trait FileSearchProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn is_available(&self) -> bool;
    fn search(&self, query: &str, limit: usize) -> Vec<FileSearchResult>;
}

pub struct SearchManager {
    providers: Vec<Box<dyn FileSearchProvider>>,
}

impl SearchManager {
    pub fn default_providers() -> Self {
        Self {
            providers: vec![Box::new(everything::EverythingProvider)],
        }
    }

    pub fn active_provider_id(&self) -> Option<&'static str> {
        self.providers
            .iter()
            .find(|p| p.is_available())
            .map(|p| p.id())
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<FileSearchResult> {
        let clamped = limit.clamp(1, 12);
        for provider in &self.providers {
            if provider.is_available() {
                return provider.search(query, clamped);
            }
        }
        Vec::new()
    }
}

#[tauri::command]
pub fn get_file_search_status() -> Option<String> {
    SearchManager::default_providers()
        .active_provider_id()
        .map(str::to_string)
}

pub(crate) fn extract_exe_from_command(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next()?
    } else {
        trimmed.split_whitespace().next()?
    };
    let clean = path.trim();
    if clean.to_ascii_lowercase().ends_with(".exe") {
        Some(clean.to_string())
    } else {
        None
    }
}

fn read_reg_sz(hkey: HKEY, value_name: Option<&str>) -> Option<String> {
    let wide_name: Option<Vec<u16>> =
        value_name.map(|s| s.encode_utf16().chain(std::iter::once(0)).collect());
    let name_ptr = wide_name
        .as_ref()
        .map(|v| PCWSTR(v.as_ptr()))
        .unwrap_or(PCWSTR::null());
    let mut reg_type = REG_SZ;
    let mut byte_len: u32 = 0;
    unsafe {
        if RegQueryValueExW(hkey, name_ptr, None, Some(&mut reg_type), None, Some(&mut byte_len))
            .is_err()
            || byte_len < 2
        {
            return None;
        }
        let mut buf = vec![0u16; (byte_len as usize + 1) / 2];
        if RegQueryValueExW(
            hkey,
            name_ptr,
            None,
            Some(&mut reg_type),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut byte_len),
        )
        .is_err()
        {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..end]).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }
}

fn scan_registry_browsers(root: HKEY, out: &mut Vec<BrowserInfo>, seen: &mut HashSet<String>) {
    let subkey: Vec<u16> = "SOFTWARE\\Clients\\StartMenuInternet\0"
        .encode_utf16()
        .collect();
    let mut parent = HKEY::default();
    unsafe {
        if RegOpenKeyExW(root, PCWSTR(subkey.as_ptr()), Some(0), KEY_READ, &mut parent).is_err() {
            return;
        }
        let mut index = 0u32;
        loop {
            let mut name_buf = [0u16; 256];
            let mut name_len = name_buf.len() as u32;
            if RegEnumKeyExW(
                parent,
                index,
                Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
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
            let key_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);

            let key_wide: Vec<u16> = key_name.encode_utf16().chain(std::iter::once(0)).collect();
            let mut browser_key = HKEY::default();
            let display_name = if RegOpenKeyExW(
                parent,
                PCWSTR(key_wide.as_ptr()),
                Some(0),
                KEY_READ,
                &mut browser_key,
            )
            .is_ok()
            {
                let label = read_reg_sz(browser_key, None);
                let _ = RegCloseKey(browser_key);
                label
            } else {
                None
            };

            let cmd_sub: Vec<u16> = format!("{key_name}\\shell\\open\\command\0")
                .encode_utf16()
                .collect();
            let mut cmd_key = HKEY::default();
            if RegOpenKeyExW(parent, PCWSTR(cmd_sub.as_ptr()), Some(0), KEY_READ, &mut cmd_key)
                .is_ok()
            {
                if let Some(cmd_raw) = read_reg_sz(cmd_key, None) {
                    if let Some(exe) = extract_exe_from_command(&cmd_raw) {
                        let norm = exe.to_ascii_lowercase();
                        if Path::new(&exe).exists() && seen.insert(norm) {
                            let fallback_name = Path::new(&exe)
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or(&key_name)
                                .to_string();
                            out.push(BrowserInfo {
                                name: display_name.unwrap_or(fallback_name),
                                path: exe,
                            });
                        }
                    }
                }
                let _ = RegCloseKey(cmd_key);
            }
        }
        let _ = RegCloseKey(parent);
    }
}

#[tauri::command]
pub fn get_installed_browsers() -> Vec<BrowserInfo> {
    let mut browsers = Vec::new();
    let mut seen = HashSet::new();

    scan_registry_browsers(HKEY_CURRENT_USER, &mut browsers, &mut seen);
    scan_registry_browsers(HKEY_LOCAL_MACHINE, &mut browsers, &mut seen);

    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let pf = std::env::var("ProgramFiles").unwrap_or_default();
    let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_default();

    let known: [(&str, String); 10] = [
        ("Google Chrome", format!(r"{pf}\Google\Chrome\Application\chrome.exe")),
        ("Microsoft Edge", format!(r"{pf86}\Microsoft\Edge\Application\msedge.exe")),
        ("Brave", format!(r"{pf}\BraveSoftware\Brave-Browser\Application\brave.exe")),
        ("Brave", format!(r"{local}\BraveSoftware\Brave-Browser\Application\brave.exe")),
        ("Firefox", format!(r"{pf}\Mozilla Firefox\firefox.exe")),
        ("Zen Browser", format!(r"{pf}\Zen Browser\zen.exe")),
        ("Helium", format!(r"{local}\imput\Helium\Application\chrome.exe")),
        ("Vivaldi", format!(r"{local}\Vivaldi\Application\vivaldi.exe")),
        ("Opera", format!(r"{local}\Programs\Opera\opera.exe")),
        ("Opera GX", format!(r"{local}\Programs\Opera GX\opera.exe")),
    ];

    for (name, path) in known {
        let norm = path.to_ascii_lowercase();
        if Path::new(&path).exists() && seen.insert(norm) {
            browsers.push(BrowserInfo {
                name: name.to_string(),
                path,
            });
        }
    }

    browsers.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    browsers
}

pub(crate) fn is_safe_web_url(url: &str) -> bool {
    let trimmed = url.trim();
    trimmed.starts_with("https://") || trimmed.starts_with("http://")
}

#[tauri::command]
pub fn open_web_url(url: String, browser_path: Option<String>) -> Result<(), String> {
    let trimmed = url.trim();
    if !is_safe_web_url(trimmed) {
        return Err("Only http:// and https:// URLs are allowed".to_string());
    }
    if let Some(bp) = browser_path.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        if Path::new(&bp).exists() {
            Command::new(&bp)
                .arg(trimmed)
                .spawn()
                .map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    let wide_url: Vec<u16> = trimmed.encode_utf16().chain(std::iter::once(0)).collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    unsafe {
        let res = ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(wide_url.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        if (res.0 as isize) <= 32 {
            return Err("Failed to open URL".to_string());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn search_files(query: String, limit: Option<usize>) -> Vec<FileSearchResult> {
    let trimmed = query.trim().to_string();
    if trimmed.chars().count() < 2 {
        return Vec::new();
    }
    let cap = limit.unwrap_or(5).clamp(1, 12);
    tauri::async_runtime::spawn_blocking(move || {
        SearchManager::default_providers().search(&trimmed, cap)
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
pub fn open_file_path(path: String, reveal_in_folder: bool) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err("Path does not exist".to_string());
    }
    if reveal_in_folder {
        Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", path.replace('"', "")))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let wide_path: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    unsafe {
        let res = ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(wide_path.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        if (res.0 as isize) <= 32 {
            return Err("Failed to open path".to_string());
        }
    }
    Ok(())
}

#[tauri::command]
pub fn launcher_system_action(action: String) -> Result<(), String> {
    match action.as_str() {
        "lock" => unsafe { LockWorkStation().map_err(|e| e.to_string()) },
        "sleep" => unsafe {
            let ok = SetSuspendState(false, false, false);
            if ok {
                Ok(())
            } else {
                Err("SetSuspendState failed".to_string())
            }
        },
        _ => Err(format!("Unknown system action: {action}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider {
        available: bool,
        items: Vec<FileSearchResult>,
    }

    impl FileSearchProvider for MockProvider {
        fn id(&self) -> &'static str {
            "mock"
        }
        fn is_available(&self) -> bool {
            self.available
        }
        fn search(&self, _query: &str, limit: usize) -> Vec<FileSearchResult> {
            self.items.iter().take(limit).cloned().collect()
        }
    }

    #[test]
    fn manager_picks_first_available_provider_and_clamps_limit() {
        let sample = FileSearchResult {
            name: "notes.md".into(),
            path: r"C:\notes.md".into(),
            parent_dir: r"C:\".into(),
            is_dir: false,
            extension: Some("md".into()),
        };
        let mgr = SearchManager {
            providers: vec![
                Box::new(MockProvider {
                    available: false,
                    items: vec![],
                }),
                Box::new(MockProvider {
                    available: true,
                    items: vec![sample.clone(); 20],
                }),
            ],
        };
        let res = mgr.search("notes", 50);
        assert_eq!(res.len(), 12);
        assert_eq!(res[0].name, "notes.md");
        assert_eq!(mgr.active_provider_id(), Some("mock"));
    }

    #[test]
    fn extract_exe_from_command_parses_quoted_and_unquoted() {
        assert_eq!(
            extract_exe_from_command(r#""C:\Program Files\Brave\brave.exe" -- "%1""#),
            Some(r"C:\Program Files\Brave\brave.exe".to_string())
        );
        assert_eq!(
            extract_exe_from_command(r"C:\Firefox\firefox.exe %1"),
            Some(r"C:\Firefox\firefox.exe".to_string())
        );
        assert_eq!(extract_exe_from_command("not-an-exe"), None);
    }

    #[test]
    fn is_safe_web_url_rejects_non_http_schemes() {
        assert!(is_safe_web_url("https://duckduckgo.com/?q=fern"));
        assert!(is_safe_web_url("http://localhost:1420"));
        assert!(!is_safe_web_url("file:///C:/Windows/System32/cmd.exe"));
        assert!(!is_safe_web_url("javascript:alert(1)"));
    }
}
