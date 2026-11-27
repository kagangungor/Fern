pub mod everything;

use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::Command;

use windows::core::PCWSTR;
use windows::Win32::System::Power::SetSuspendState;
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

pub trait FileSearchProvider: Send + Sync {
    #[allow(dead_code)]
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
    }
}
