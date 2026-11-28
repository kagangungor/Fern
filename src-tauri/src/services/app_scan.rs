use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use tauri::Manager;
use windows::Win32::Foundation::HWND;
use std::path::Path;
use crate::types::*;
use crate::state::*;
use crate::utils::*;

pub fn trigger_app_scan() {
    if IS_SCANNING.load(Ordering::Relaxed) { return; }
    IS_SCANNING.store(true, Ordering::Relaxed);
    
    std::thread::spawn(|| {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, CoTaskMemFree};
        use windows::Win32::UI::Shell::{SHGetKnownFolderIDList, FOLDERID_AppsFolder, SHGetDesktopFolder, IShellFolder, IEnumIDList, SHGetNameFromIDList, SIGDN_NORMALDISPLAY, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH, SIGDN_URL};
        let mut apps = Vec::new();
        unsafe {
            let com_ok = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            // Use a scope to ensure COM objects are dropped before CoUninitialize
            {
                if let Ok(pidl_apps) = SHGetKnownFolderIDList(&FOLDERID_AppsFolder, 0, None) {
                    if let Ok(desktop) = SHGetDesktopFolder() {
                        if let Ok(apps_folder) = desktop.BindToObject::<_, IShellFolder>(pidl_apps, None) {
                            let mut enum_id: Option<IEnumIDList> = None;
                            let res = apps_folder.EnumObjects(HWND(std::ptr::null_mut()), (windows::Win32::UI::Shell::SHCONTF_FOLDERS.0 | windows::Win32::UI::Shell::SHCONTF_NONFOLDERS.0) as u32, &mut enum_id);
                            
                            if res.is_ok() {
                                if let Some(enum_id) = enum_id {
                                    let mut items = [std::ptr::null_mut(); 1];
                                    let mut fetched = 0;
                                    while enum_id.Next(&mut items, Some(&mut fetched)).is_ok() && fetched > 0 {
                                        let pidl_item = items[0];
                                        let abs_item = windows::Win32::UI::Shell::ILCombine(Some(pidl_apps as *const _), Some(pidl_item as *const _));

                                        let name = if let Ok(n_ptr) = SHGetNameFromIDList(abs_item, SIGDN_NORMALDISPLAY) {
                                            let s = String::from_utf16_lossy(windows::core::PCWSTR(n_ptr.0).as_wide());
                                            CoTaskMemFree(Some(n_ptr.0 as *const _));
                                            s
                                        } else { "Unknown".to_string() };

                                        // Parsing name: exe path for Win32 apps, AUMID for packaged ones.
                                        let path = if let Ok(p_ptr) = SHGetNameFromIDList(abs_item, SIGDN_DESKTOPABSOLUTEPARSING) {
                                            let s = String::from_utf16_lossy(windows::core::PCWSTR(p_ptr.0).as_wide());
                                            CoTaskMemFree(Some(p_ptr.0 as *const _));
                                            s
                                        } else if let Ok(p_ptr) = SHGetNameFromIDList(abs_item, SIGDN_FILESYSPATH) {
                                            let s = String::from_utf16_lossy(windows::core::PCWSTR(p_ptr.0).as_wide());
                                            CoTaskMemFree(Some(p_ptr.0 as *const _));
                                            s
                                        } else if let Ok(p_ptr) = SHGetNameFromIDList(abs_item, SIGDN_URL) {
                                            let s = String::from_utf16_lossy(windows::core::PCWSTR(p_ptr.0).as_wide());
                                            CoTaskMemFree(Some(p_ptr.0 as *const _));
                                            s
                                        } else { name.clone() };

                                        if is_launchable_entry(&name, &path) {
                                            // Icons are fetched lazily by get_app_icon and cached in ICON_CACHE so get_installed_apps stays tiny and instant.
                                            let executable = if path.contains('\\') || path.contains('/') {
                                                std::path::Path::new(&path).file_name().and_then(|n| n.to_str()).map(|s| s.to_string())
                                            } else {
                                                None
                                            };
                                            apps.push(AppInfo {
                                                name,
                                                path,
                                                icon: None,
                                                is_running: false,
                                                hwnd: None,
                                                executable,
                                                all_hwnds: None,
                                                args: None,
                                            });
                                        }
                                        if !abs_item.is_null() { CoTaskMemFree(Some(abs_item as *const _)); }
                                        CoTaskMemFree(Some(pidl_item as *const _));
                                        items[0] = std::ptr::null_mut();
                                    }
                                }
                            }
                        }
                    }
                    CoTaskMemFree(Some(pidl_apps as *const _));
                }
            } // Close COM scope
            if com_ok { CoUninitialize(); }
        }

        // Scan Start Menu .lnk shortcuts (depth-limited, no .exe scanning)
        // This catches Win32 apps that FOLDERID_AppsFolder may miss
        let mut start_menu_dirs: Vec<String> = Vec::new();
        if let Ok(programdata) = std::env::var("PROGRAMDATA") {
            start_menu_dirs.push(format!(r"{}\Microsoft\Windows\Start Menu\Programs", programdata));
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            start_menu_dirs.push(format!(r"{}\Microsoft\Windows\Start Menu\Programs", appdata));
        }

        // Shortcut resolution uses IShellLinkW, which needs COM on this thread.
        let mut resolved_target_names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut shortcut_targets: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let shortcut_com_ok = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() };
        for dir in &start_menu_dirs {
            let root = std::path::Path::new(dir);
            if root.exists() {
                collect_shortcuts(root, &mut apps, &mut resolved_target_names, &mut shortcut_targets, 0);
            }
        }
        if shortcut_com_ok { unsafe { CoUninitialize(); } }

        if let Ok(mut map) = RESOLVED_TARGET_NAMES
            .get_or_init(|| std::sync::RwLock::new(std::collections::HashMap::new()))
            .write()
        {
            *map = resolved_target_names;
        }

        if let Some(c) = INSTALLED_APPS_CACHE.get() {
            if let Ok(mut lock) = c.lock() {
                *lock = apps;
            }
        }
        IS_SCANNING.store(false, Ordering::Relaxed);
    });
}

/// Uninstallers, installers, help, docs, troubleshooters. Whole words only, so "Helper" or "Remover" stay.
fn is_helper_name(name: &str) -> bool {
    const HELPER_WORDS: &[&str] = &[
        "uninstall", "uninstaller", "installer", "setup", "help", "readme", "manual",
        "documentation", "troubleshoot", "troubleshooting", "repair",
    ];
    const HELPER_PHRASES: &[&str] = &[
        "release notes",
        "desktop overlay host",
        "debuggable package manager",
        "developer command prompt",
        "developer powershell",
        "crash reporter",
        "crash handler",
    ];
    let lower = name.to_lowercase();
    if HELPER_PHRASES.iter().any(|p| lower.contains(p)) {
        return true;
    }
    lower
        .split(|c: char| !c.is_alphanumeric())
        .any(|token| HELPER_WORDS.contains(&token))
}

/// Drops entries that aren't real apps (web links, documents, protocol handlers).
fn is_launchable_entry(name: &str, path: &str) -> bool {
    if name.is_empty() || name == "Unknown" || is_helper_name(name) { return false; }
    let lower = path.to_lowercase();
    if lower.is_empty() { return false; }
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("file:")
        || lower.starts_with("steam:") || lower.starts_with("::{")
        || (lower.starts_with("shell:") && !lower.starts_with("shell:appsfolder")) {
        return false;
    }
    true
}

fn is_shared_host_exe(exe: &str) -> bool {
    const SHARED_HOST_EXES: &[&str] = &[
        "cmd.exe", "powershell.exe", "pwsh.exe", "wt.exe", "explorer.exe", "mmc.exe",
        "rundll32.exe", "mshta.exe", "wscript.exe", "cscript.exe", "javaw.exe", "java.exe",
        "python.exe", "pythonw.exe",
    ];
    SHARED_HOST_EXES.iter().any(|&host| exe.eq_ignore_ascii_case(host))
}

fn shortcut_display_name(path: &std::path::Path) -> String {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{SHFILEINFOW, SHGFI_DISPLAYNAME, SHGetFileInfoW};

    let fallback = || {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    };

    unsafe {
        let mut shfi: SHFILEINFOW = std::mem::zeroed();
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let res = SHGetFileInfoW(
            windows::core::PCWSTR(wide.as_ptr()),
            Default::default(),
            Some(&mut shfi),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_DISPLAYNAME,
        );
        if res != 0 {
            let len = shfi
                .szDisplayName
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(shfi.szDisplayName.len());
            let raw = String::from_utf16_lossy(&shfi.szDisplayName[..len]).trim().to_string();
            let cleaned = if raw.to_ascii_lowercase().ends_with(".lnk") {
                raw[..raw.len() - 4].trim().to_string()
            } else {
                raw
            };
            if !cleaned.is_empty() {
                return cleaned;
            }
        }
    }
    fallback()
}

pub(super) static RESOLVED_TARGET_NAMES: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<String, String>>> =
    std::sync::OnceLock::new();

pub(super) fn normalize_exe_path(path: &str) -> String {
    path.trim().replace('/', "\\").to_lowercase()
}

fn exe_parent_dir(path: &str) -> Option<String> {
    let norm = normalize_exe_path(path);
    if !norm.ends_with(".exe") {
        return None;
    }
    let idx = norm.rfind('\\')?;
    Some(norm[..idx].to_string())
}

fn is_duplicate_shortcut(
    existing: &AppInfo,
    existing_resolved_target: Option<&str>,
    name: &str,
    path_str: &str,
    is_pwa: bool,
    executable: Option<&str>,
    resolved_target: Option<&str>,
) -> bool {
    if existing.name.eq_ignore_ascii_case(name) || existing.path.eq_ignore_ascii_case(path_str) {
        return true;
    }
    if is_pwa {
        return false;
    }
    let Some(exe) = executable else {
        return false;
    };
    if is_shared_host_exe(exe) {
        return false;
    }
    let same_exe = existing
        .executable
        .as_deref()
        .is_some_and(|e| e.eq_ignore_ascii_case(exe));
    if !same_exe {
        return false;
    }
    let existing_target = existing_resolved_target.or_else(|| {
        existing
            .path
            .to_ascii_lowercase()
            .ends_with(".exe")
            .then_some(existing.path.as_str())
    });
    match (existing_target, resolved_target) {
        (Some(et), Some(rt)) => {
            normalize_exe_path(et) == normalize_exe_path(rt)
                || exe_parent_dir(et)
                    .is_some_and(|ed| exe_parent_dir(rt).as_deref() == Some(ed.as_str()))
        }
        _ => false,
    }
}

fn collect_shortcuts(
    dir: &std::path::Path,
    apps: &mut Vec<AppInfo>,
    resolved_target_names: &mut std::collections::HashMap<String, String>,
    shortcut_targets: &mut std::collections::HashMap<String, String>,
    depth: i32,
) {
    if depth > 3 { return; }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_shortcuts(&path, apps, resolved_target_names, shortcut_targets, depth + 1);
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk")) {
                let name = shortcut_display_name(&path);
                if name.is_empty() || is_helper_name(&name) || name.starts_with("Install") { continue; }

                // Keep the .lnk itself, its args identify PWAs and must be passed on launch.
                // The target goes in as the exe name so pins still match running windows.
                let path_str = path.to_string_lossy().to_string();
                let resolved = crate::utils::resolve_shortcut(&path_str);
                let resolved_target = resolved.as_ref().map(|(t, _)| t.as_str());
                let args = resolved.as_ref().map(|(_, a)| a.trim().to_string()).filter(|a| !a.is_empty());
                let is_pwa = args.as_deref().is_some_and(|a| a.contains("--app-id=") || a.contains("--app="));
                let executable = if is_pwa {
                    None
                } else {
                    resolved_target.and_then(|target| {
                        let file = std::path::Path::new(target).file_name().and_then(|n| n.to_str())?.to_string();
                        // UWP shortcuts launch explorer.exe with shell:AppsFolder, storing that would match File Explorer windows.
                        if file.eq_ignore_ascii_case("explorer.exe") { None } else { Some(file) }
                    })
                };

                if !is_pwa {
                    if let (Some(exe), Some(target)) = (executable.as_deref(), resolved_target) {
                        if !is_shared_host_exe(exe) {
                            let norm_target = normalize_exe_path(target);
                            if norm_target.ends_with(".exe") {
                                resolved_target_names.entry(norm_target).or_insert_with(|| name.clone());
                            }
                        }
                    }
                }

                let duplicate = apps.iter().any(|a| {
                    let existing_target = shortcut_targets.get(&a.path.to_lowercase()).map(|s| s.as_str());
                    is_duplicate_shortcut(
                        a,
                        existing_target,
                        &name,
                        &path_str,
                        is_pwa,
                        executable.as_deref(),
                        resolved_target,
                    )
                });
                if !duplicate {
                    if let Some(target) = resolved_target {
                        shortcut_targets.insert(path_str.to_lowercase(), target.to_string());
                    }
                    apps.push(AppInfo {
                        name,
                        path: path_str,
                        icon: None,
                        is_running: false,
                        hwnd: None,
                        executable,
                        all_hwnds: None,
                        args,
                    });
                }
            } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("url")) {
                let name = path.file_stem().unwrap().to_string_lossy().to_string();
                if is_helper_name(&name) || name.starts_with("Install") { continue; }
                let path_str = path.to_string_lossy().to_string();
                if !apps.iter().any(|a| a.name == name || a.path == path_str) {
                    apps.push(AppInfo {
                        name,
                        path: path_str,
                        icon: None,
                        is_running: false,
                        hwnd: None,
                        executable: None,
                        all_hwnds: None,
                        args: None,
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helper_shortcuts_are_not_launchable() {
        assert!(is_helper_name("Uninstall Brave"));
        assert!(is_helper_name("7-Zip Help"));
        assert!(is_helper_name("Readme"));
        assert!(is_helper_name("Get Help"));
        assert!(is_helper_name("Desktop Overlay Host"));
        assert!(is_helper_name("Debuggable Package Manager"));
        assert!(is_helper_name("Developer Command Prompt for VS 2022"));
        assert!(is_helper_name("Developer PowerShell for VS 2022"));
        // Whole-word matching: legit names containing helper words survive.
        assert!(!is_helper_name("Helper"));
        assert!(!is_helper_name("Photo Remover"));
        assert!(!is_helper_name("7-Zip File Manager"));
        assert!(!is_helper_name("Brave"));
        assert!(!is_helper_name("Blender 4.5"));
        assert!(!is_helper_name("Blockbench"));
        assert!(!is_helper_name("Denetim Masası"));
        assert!(!is_helper_name("Deskio"));
        assert!(!is_launchable_entry("Docs", "https://example.com"));
        assert!(is_launchable_entry("Brave", r"C:\Program Files\Brave\brave.exe"));
    }

    #[test]
    fn shortcut_dedup_preserves_distinct_apps_sharing_executable_name() {
        use super::is_duplicate_shortcut;
        use crate::types::AppInfo;

        let firefox = AppInfo {
            name: "Firefox".to_string(),
            path: r"C:\Program Files\Mozilla Firefox\firefox.exe".to_string(),
            icon: None,
            is_running: false,
            hwnd: None,
            executable: Some("firefox.exe".to_string()),
            all_hwnds: None,
            args: None,
        };

        // Distinct app sharing firefox.exe in another directory must NOT be deduplicated.
        assert!(!is_duplicate_shortcut(
            &firefox,
            None,
            "Tor Browser",
            r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Tor Browser.lnk",
            false,
            Some("firefox.exe"),
            Some(r"C:\Users\u\Desktop\Tor Browser\Browser\firefox.exe"),
        ));

        // Same resolved target path with a different shortcut display name IS a duplicate.
        assert!(is_duplicate_shortcut(
            &firefox,
            None,
            "Mozilla Firefox",
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Mozilla Firefox.lnk",
            false,
            Some("firefox.exe"),
            Some(r"C:\Program Files\Mozilla Firefox\firefox.exe"),
        ));

        // Same display name IS a duplicate even without a resolved target.
        assert!(is_duplicate_shortcut(
            &firefox,
            None,
            "Firefox",
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Firefox.lnk",
            false,
            Some("firefox.exe"),
            None,
        ));

        // Two .lnk shortcuts pointing to the same target are deduplicated via existing_resolved_target.
        let tor_lnk = AppInfo {
            name: "Tor Browser".to_string(),
            path: r"C:\Users\u\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Tor Browser.lnk".to_string(),
            icon: None,
            is_running: false,
            hwnd: None,
            executable: Some("firefox.exe".to_string()),
            all_hwnds: None,
            args: None,
        };
        assert!(is_duplicate_shortcut(
            &tor_lnk,
            Some(r"C:\Users\u\Desktop\Tor Browser\Browser\firefox.exe"),
            "Start Tor Browser",
            r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Start Tor Browser.lnk",
            false,
            Some("firefox.exe"),
            Some(r"C:\Users\u\Desktop\Tor Browser\Browser\firefox.exe"),
        ));
    }
}
