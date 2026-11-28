use std::collections::HashMap;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::{HWND, LPARAM};

use crate::state::*;
use crate::utils::*;

fn get_cache_key(path: &str, name: Option<&str>) -> String {
    let path_lc = path.to_lowercase();
    let name_lc = name.map(|n| n.to_lowercase()).unwrap_or_default();
    if path_lc.contains("msedge.exe") || path_lc.contains("chrome.exe") || path_lc.contains("brave.exe") || path_lc.contains("applicationframehost.exe") {
        format!("{}:{}", path, name_lc)
    } else {
        path.to_string()
    }
}

fn get_custom_icons_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(|e| e.to_string())?.join("custom_icons");
    let _ = std::fs::create_dir_all(&dir);
    Ok(dir)
}

fn sanitize_filename(key: &str) -> String {
    key.replace(|c: char| c == ':' || c == '\\' || c == '/' || c == '*' || c == '?' || c == '"' || c == '<' || c == '>' || c == '|', "_")
}

/// Reads a --flag=value argument from a command line, handles quotes.
fn extract_arg(args: &str, key: &str) -> Option<String> {
    let idx = args.find(key)?;
    let rest = &args[idx + key.len()..];
    let value = if let Some(stripped) = rest.strip_prefix('"') {
        stripped.split('"').next().unwrap_or("")
    } else {
        rest.split_whitespace().next().unwrap_or("")
    };
    let value = value.trim().trim_matches('"');
    if value.is_empty() { None } else { Some(value.to_string()) }
}

/// Normalizes an AppsFolder entry or AUMID into a shell parsing name.
pub(super) fn shell_parsing_name_for_aumid(path: &str) -> String {
    let p = path.trim().trim_matches('"');
    if p.is_empty() {
        return String::new();
    }
    let lower = p.to_ascii_lowercase();
    if lower.starts_with("shell:appsfolder\\") {
        let rest = &p["shell:AppsFolder\\".len()..];
        if rest.is_empty() {
            return String::new();
        }
        return format!("shell:AppsFolder\\{}", rest);
    }
    if p.starts_with("::") || lower.starts_with("shell:") {
        return p.to_string();
    }
    format!("shell:AppsFolder\\{}", p)
}

/// True for shell app ids (PackageFamily!App, {GUID}\..., ::{CLSID}\..., etc.), not file paths.
pub fn is_aumid_path(path: &str) -> bool {
    let p = path.trim().trim_matches('"');
    if p.is_empty() || p.len() < 3 { return false; }
    if p.to_lowercase().starts_with("shell:appsfolder\\") { return true; }
    if p.starts_with("::{") { return true; }
    if p.starts_with('{') && p.contains("}\\") { return true; }
    if p.contains('\\') || p.contains('/') { return false; }
    // Some installers (e.g. CapCut) register dot-flattened path AppIDs in AppsFolder like
    // `c:.users.user.appdata.local.capcut.apps.9.7.0.4153.capcut.exe`.
    let bytes = p.as_bytes();
    if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'.' {
        return true;
    }
    if p.contains(':') { return false; }
    if p.to_lowercase().ends_with(".exe") { return false; }
    if !p.contains('.') && !p.contains('!') { return false; }
    if std::path::Path::new(p).exists() { return false; }
    true
}

pub(super) fn is_browser_host_process(path: &str) -> bool {
    let p = path.to_lowercase();
    p.contains("msedge.exe") || p.contains("chrome.exe") || p.contains("brave.exe") || p.contains("vivaldi.exe") || p.contains("firefox.exe")
}

/// Icon for a shell app id via shell:AppsFolder (SHGetFileInfoW can't do these).
fn icon_from_aumid(aumid: &str) -> Option<String> {
    unsafe {
        use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName};

        let parsing_name = shell_parsing_name_for_aumid(aumid);
        if parsing_name.is_empty() { return None; }

        let wide: Vec<u16> = parsing_name.encode_utf16().chain(std::iter::once(0)).collect();
        let factory: IShellItemImageFactory = SHCreateItemFromParsingName(windows::core::PCWSTR(wide.as_ptr()), None).ok()?;
        crate::utils::image_factory_to_base64(&factory)
    }
}

/// Package family name (Name_PublisherId) from a WindowsApps exe path.
pub(super) fn package_family_from_windows_apps_path(path: &str) -> Option<String> {
    let lower = path.to_lowercase();
    let idx = lower.find("\\windowsapps\\")?;
    let rest = &path[idx + "\\windowsapps\\".len()..];
    let folder = rest.split(['\\', '/']).next()?;
    let (before_publisher, publisher) = folder.rsplit_once("__")?;
    let name = before_publisher.split('_').next()?;
    if name.is_empty() || publisher.is_empty() { return None; }
    Some(format!("{}_{}", name, publisher))
}

/// Looks up the exact AUMID of a package family by walking shell:AppsFolder.
pub(super) fn find_aumid_by_family(family: &str) -> Option<String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{SHGetKnownFolderIDList, FOLDERID_AppsFolder, SHGetDesktopFolder, IShellFolder, IEnumIDList, SHGetNameFromIDList, ILCombine, ILFree, SIGDN_DESKTOPABSOLUTEPARSING, SHCONTF_FOLDERS, SHCONTF_NONFOLDERS};

    unsafe {
        let pidl_apps = SHGetKnownFolderIDList(&FOLDERID_AppsFolder, 0, None).ok()?;
        let result = (|| -> Option<String> {
            let desktop = SHGetDesktopFolder().ok()?;
            let apps_folder: IShellFolder = desktop.BindToObject(pidl_apps, None).ok()?;
            let mut enum_id: Option<IEnumIDList> = None;
            if apps_folder.EnumObjects(HWND(std::ptr::null_mut()), (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0) as u32, &mut enum_id).is_err() { return None; }
            let enum_id = enum_id?;

            let prefix = format!("{}!", family);
            let mut pidl_buf: [*mut windows::Win32::UI::Shell::Common::ITEMIDLIST; 1] = [std::ptr::null_mut()];
            let mut fetched = 0;
            while enum_id.Next(&mut pidl_buf, Some(&mut fetched)).is_ok() && fetched > 0 {
                let pidl_item = pidl_buf[0];
                pidl_buf[0] = std::ptr::null_mut();
                if pidl_item.is_null() { continue; }

                let absolute_pidl = ILCombine(Some(pidl_apps as *const _), Some(pidl_item as *const _));
                if absolute_pidl.is_null() {
                    CoTaskMemFree(Some(pidl_item as *const _));
                    continue;
                }

                let mut found = None;
                if let Ok(p_ptr) = SHGetNameFromIDList(absolute_pidl, SIGDN_DESKTOPABSOLUTEPARSING) {
                    let s = String::from_utf16_lossy(windows::core::PCWSTR(p_ptr.0).as_wide());
                    CoTaskMemFree(Some(p_ptr.0 as *const _));
                    if s.starts_with(&prefix) { found = Some(s); }
                }
                ILFree(Some(absolute_pidl as *const _));
                CoTaskMemFree(Some(pidl_item as *const _));
                if found.is_some() { return found; }
            }
            None
        })();
        CoTaskMemFree(Some(pidl_apps as *const _));
        result
    }
}

/// Icon for exes inside an MSIX package. The exe has none, so use the manifest logo.
fn packaged_exe_icon(path: &str) -> Option<String> {
    let family = package_family_from_windows_apps_path(path)?;
    let aumid = find_aumid_by_family(&family)?;
    icon_from_aumid(&aumid)
}

/// Process command line via WMI, used to spot PWAs inside browser processes.
fn process_command_line(pid: u32) -> Option<String> {
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(rename_all = "PascalCase")]
    struct Win32Process {
        command_line: Option<String>,
    }

    let com = wmi::COMLibrary::new().ok()?;
    let connection = wmi::WMIConnection::new(com).ok()?;
    let results: Vec<Win32Process> = connection
        .raw_query(format!("SELECT CommandLine FROM Win32_Process WHERE ProcessId = {}", pid))
        .ok()?;
    results.into_iter().next().and_then(|p| p.command_line)
}

fn pwa_icon_from_command_line(process_path: &str, command_line: &str) -> Option<String> {
    // Store PWAs from Edge carry their package id here.
    if let Some(aumid) = extract_arg(command_line, "--ip-aumid=") {
        if let Some(icon) = icon_from_aumid(&aumid) { return Some(icon); }
    }
    // Installed web app windows carry the web app id.
    if command_line.contains("--app-id=") {
        if let Some(icon_path) = find_browser_pwa_icon(process_path, command_line) {
            if let Some(icon) = crate::utils::image_file_to_base64(&icon_path) { return Some(icon); }
        }
    }
    None
}

/// Web app ids are 32 chars, a-p.
pub(crate) fn browser_web_app_id_from_aumid(id: &str) -> Option<String> {
    let is_web_app_id = |s: &str| s.len() == 32 && s.bytes().all(|b| (b'a'..=b'p').contains(&b));
    if let Some(segment) = id.rsplit('.').next() {
        if is_web_app_id(segment) { return Some(segment.to_string()); }
    }
    if let Some(idx) = id.find("_crx_") {
        if let Some(segment) = id[idx + 5..].split('.').next() {
            if is_web_app_id(segment) { return Some(segment.to_string()); }
        }
    }
    None
}

/// True if an AUMID belongs to a browser-hosted app (PWA) and not the browser itself.
/// Brave uses a shortened id, so the _crx_ marker is checked directly.
pub(crate) fn is_browser_pwa_aumid(id: &str) -> bool {
    id.contains('!') || id.contains("_crx_") || browser_web_app_id_from_aumid(id).is_some()
}

unsafe fn pwa_icon_for_window(hwnd: HWND, process_path: &str) -> Option<String> {
    // The window's own AUMID is the most reliable, the command line isn't (Edge reuses its browser process).
    match crate::utils::get_window_app_user_model_id(hwnd) {
        Some(id) if id.contains('!') => {
            if let Some(icon) = icon_from_aumid(&id) { return Some(icon); }
        }
        Some(id) if is_browser_pwa_aumid(&id) => {
            // PWA window: app id from the AUMID, else from the command line.
            let mut pid = 0u32;
            windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == 0 { return None; }
            if let Some(app_id) = browser_web_app_id_from_aumid(&id) {
                if let Some(icon_path) = find_browser_pwa_icon(process_path, &format!("--app-id={}", app_id)) {
                    if let Some(icon) = crate::utils::image_file_to_base64(&icon_path) { return Some(icon); }
                }
            }
            if let Some(command_line) = process_command_line(pid) {
                if let Some(icon) = pwa_icon_from_command_line(process_path, &command_line) { return Some(icon); }
            }
            return None;
        }
        // Plain browser window. Its process command line can carry an --app-id from another
        // window, so skip it and let the caller use the window icon.
        Some(_) => return None,
        None => {}
    }
    // Fall back to the command line when the window has no id.
    let mut pid = 0u32;
    windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == 0 { return None; }
    let command_line = process_command_line(pid)?;
    pwa_icon_from_command_line(process_path, &command_line)
}

/// Finds a web app's icon in the browser profile (Manifest Resources/<id>/Icons,
/// or the old _crx_ layout).
fn find_browser_pwa_icon(executable_path: &str, args: &str) -> Option<String> {
    let app_id = extract_arg(args, "--app-id=")?;
    let local = std::env::var("LOCALAPPDATA").ok()?;

    let chrome_base = format!("{}\\Google\\Chrome\\User Data", local);
    let edge_base = format!("{}\\Microsoft\\Edge\\User Data", local);
    let brave_base = format!("{}\\BraveSoftware\\Brave-Browser\\User Data", local);
    let vivaldi_base = format!("{}\\Vivaldi\\User Data", local);
    let browser_path = executable_path.to_lowercase();
    let bases = if browser_path.contains("msedge") {
        vec![edge_base, chrome_base]
    } else if browser_path.contains("brave") {
        vec![brave_base, chrome_base]
    } else if browser_path.contains("vivaldi") {
        vec![vivaldi_base, chrome_base]
    } else {
        vec![chrome_base, edge_base]
    };

    let profile = extract_arg(args, "--profile-directory=").unwrap_or_else(|| "Default".to_string());

    for base in bases {
        if !std::path::Path::new(&base).is_dir() { continue; }

        let mut profiles = vec![profile.clone()];
        if let Ok(entries) = std::fs::read_dir(&base) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if !profiles.iter().any(|p| p.eq_ignore_ascii_case(&name)) { profiles.push(name); }
                }
            }
        }

        for prof in profiles {
            let web_apps = std::path::Path::new(&base).join(&prof).join("Web Applications");
            if !web_apps.is_dir() { continue; }

            let candidates = [
                web_apps.join("Manifest Resources").join(&app_id),
                web_apps.join(&app_id),
                web_apps.join(format!("_crx_{}", app_id)),
            ];
            for dir in candidates {
                if let Some(icon) = find_largest_image_in_dir(&dir) { return Some(icon); }
            }
        }
    }
    None
}

fn find_largest_image_in_dir(dir: &std::path::Path) -> Option<String> {
    if !dir.is_dir() { return None; }
    let mut best: Option<(u64, String)> = None;
    collect_images(dir, 0, &mut best);
    best.map(|(_, path)| path)
}

fn collect_images(dir: &std::path::Path, depth: u32, best: &mut Option<(u64, String)>) {
    if depth > 3 { return; }
    let Ok(entries) = std::fs::read_dir(dir) else { return; };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_images(&path, depth + 1, best);
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()) else { continue; };
        if !matches!(ext.as_str(), "png" | "ico" | "jpg" | "jpeg" | "bmp" | "webp") { continue; }
        let score = image_size_score(&path);
        let is_better = match best.as_ref() {
            Some((best_score, _)) => score > *best_score,
            None => true,
        };
        if is_better {
            *best = Some((score, path.to_string_lossy().to_string()));
        }
    }
}

/// Scores an icon by the size in its file name (256.png, 192x192.png).
fn image_size_score(path: &std::path::Path) -> u64 {
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    let mut score: u64 = 0;
    for token in name.split(|c: char| !c.is_ascii_digit() && c != 'x') {
        if token.is_empty() { continue; }
        if let Some(idx) = token.find('x') {
            let (w, h) = token.split_at(idx);
            let h = &h[1..];
            if let (Ok(w), Ok(h)) = (w.parse::<u64>(), h.parse::<u64>()) {
                score = score.max(w * h);
            }
        } else if let Ok(n) = token.parse::<u64>() {
            score = score.max(n * n);
        }
    }
    score
}

static LAST_ICON_SAVE_REQUEST: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

pub fn schedule_icon_cache_save(app: AppHandle) {
    let now = crate::utils::get_now_ms();
    LAST_ICON_SAVE_REQUEST.store(now, Ordering::Relaxed);

    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
        let last = LAST_ICON_SAVE_REQUEST.load(Ordering::Relaxed);
        if now == last || crate::utils::get_now_ms() - last >= 950 {
            if let Ok(dir) = app.path().app_config_dir() {
                let cache_path = dir.join("icons_cache.json");
                if let Some(c) = crate::state::ICON_CACHE.get() {
                    let snapshot = if let Ok(lock) = c.lock() {
                        serde_json::to_string(&*lock).unwrap_or_default()
                    } else {
                        String::new()
                    };
                    if !snapshot.is_empty() {
                        let _ = tauri::async_runtime::spawn_blocking(move || {
                            let _ = std::fs::write(&cache_path, snapshot);
                        }).await;
                    }
                }
            }
        }
    });
}

fn generic_exe_icon() -> Option<&'static str> {
    use std::sync::OnceLock;
    static GENERIC: OnceLock<Option<String>> = OnceLock::new();
    GENERIC
        .get_or_init(|| unsafe {
            use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
            use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
            use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES};
            use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

            let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let mut shfi: SHFILEINFOW = std::mem::zeroed();
            let name: Vec<u16> = "placeholder.exe".encode_utf16().chain(std::iter::once(0)).collect();
            let res = SHGetFileInfoW(
                windows::core::PCWSTR(name.as_ptr()),
                FILE_ATTRIBUTE_NORMAL,
                Some(&mut shfi),
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
            );
            let icon = if res != 0 && !shfi.hIcon.is_invalid() {
                let icon = icon_to_base64(shfi.hIcon);
                let _ = DestroyIcon(shfi.hIcon);
                icon
            } else {
                None
            };
            if com { CoUninitialize(); }
            icon
        })
        .as_deref()
}

fn is_generic_icon(icon: &str) -> bool {
    generic_exe_icon().is_some_and(|g| g == icon)
}

fn icon_from_exe_neighbourhood(exe: &str) -> Option<String> {
    use windows::Win32::UI::Shell::ExtractIconExW;
    use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};

    let wide: Vec<u16> = exe.encode_utf16().chain(std::iter::once(0)).collect();
    let mut large = HICON::default();
    let extracted = unsafe { ExtractIconExW(windows::core::PCWSTR(wide.as_ptr()), 0, Some(&mut large), None, 1) };
    if extracted > 0 && !large.is_invalid() {
        let icon = unsafe { icon_to_base64(large) };
        let _ = unsafe { DestroyIcon(large) };
        if let Some(icon) = icon.filter(|i| !is_generic_icon(i)) {
            return Some(icon);
        }
    }

    let dir = std::path::Path::new(exe).parent()?;
    let stem = std::path::Path::new(exe).file_stem()?.to_string_lossy().to_string();
    let candidates = [
        dir.join(format!("{}.ico", stem)),
        dir.join("icon.ico"),
        dir.join("app.ico"),
        dir.parent()?.join(format!("{}.ico", stem)),
        dir.parent()?.join("icon.ico"),
    ];
    candidates
        .iter()
        .filter(|p| p.is_file())
        .find_map(|p| image_file_to_base64(&p.to_string_lossy()))
        .filter(|i| !is_generic_icon(i))
}

#[tauri::command]
pub async fn get_app_icon(app: AppHandle, path: String, name: Option<String>, hwnd: Option<isize>) -> Result<Option<String>, String> {
    let cache_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let cache_path = cache_dir.join("icons_cache.json");
    let cache_key = get_cache_key(&path, name.as_deref());

    if path == "fern-settings" {
        if let Some(icon) = app.default_window_icon() {
            if let Some(rgba) = image::RgbaImage::from_raw(icon.width(), icon.height(), icon.rgba().to_vec()) {
                let mut out = Vec::new();
                if image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).is_ok() {
                    use base64::Engine;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&out);
                    return Ok(Some(format!("data:image/png;base64,{}", b64)));
                }
            }
        }
    }

    // Strategy 0: Check for custom icon first
    let custom_icons_dir = cache_dir.join("custom_icons");
    let custom_file = custom_icons_dir.join(format!("{}.png", sanitize_filename(&cache_key)));
    if custom_file.exists() {
        if let Ok(data) = std::fs::read(&custom_file) {
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
            return Ok(Some(format!("data:image/png;base64,{}", b64)));
        }
    }

    // Java programs share one javaw.exe, so the exe icon is the same for all of them. Use the window's own icon.
    if let Some(h) = hwnd {
        let path_lc = path.to_lowercase();
        if path_lc.ends_with("\\javaw.exe") || path_lc.ends_with("\\java.exe") {
            let icon = tauri::async_runtime::spawn_blocking(move || unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{GetClassLongPtrW, GCLP_HICON, WM_GETICON, ICON_BIG, SendMessageTimeoutW, SMTO_ABORTIFHUNG};
                let h_hwnd = HWND(h as *mut _);
                let mut res = 0usize;
                let _ = SendMessageTimeoutW(h_hwnd, WM_GETICON, windows::Win32::Foundation::WPARAM(ICON_BIG as usize), windows::Win32::Foundation::LPARAM(0), SMTO_ABORTIFHUNG, 250, Some(&mut res));
                let mut h_icon = windows::Win32::UI::WindowsAndMessaging::HICON(res as *mut _);
                if h_icon.is_invalid() {
                    h_icon = windows::Win32::UI::WindowsAndMessaging::HICON(GetClassLongPtrW(h_hwnd, GCLP_HICON) as *mut _);
                }
                if h_icon.is_invalid() { None } else { icon_to_base64(h_icon) }
            }).await.unwrap_or(None);
            if let Some(icon) = icon {
                return Ok(Some(icon));
            }
        }
    }

    let cache = ICON_CACHE.get_or_init(|| {
        let version_marker = cache_dir.join("icons_cache_v2");
        if !version_marker.exists() {
            let _ = std::fs::remove_file(&cache_path);
            let _ = std::fs::create_dir_all(&cache_dir);
            let _ = std::fs::write(&version_marker, b"1");
        }
        let mut map = std::collections::HashMap::new();
        if let Ok(content) = std::fs::read_to_string(&cache_path) {
            if let Ok(existing) = serde_json::from_str::<std::collections::HashMap<String, String>>(&content) {
                map = existing;
            }
        }
        std::sync::Mutex::new(map)
    });

    if path.to_ascii_lowercase().ends_with(".url") {
        let p = path.clone();
        if let Ok(Some(icon)) = tauri::async_runtime::spawn_blocking(move || url_shortcut_icon(&p)).await {
            if let Ok(mut c) = cache.lock() {
                c.insert(cache_key.clone(), icon.clone());
            }
            schedule_icon_cache_save(app.clone());
            return Ok(Some(icon));
        }
    }

    let is_uwp_host = path.to_lowercase().contains("applicationframehost.exe");
    let stale_store_path = path.to_lowercase().contains("\\windowsapps\\") && !std::path::Path::new(&path).exists();
    let match_by_name = is_uwp_host || stale_store_path;
    let lookup_name: Option<String> = name.clone().or_else(|| {
        if stale_store_path {
            std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().to_string())
        } else {
            None
        }
    });

    if match_by_name {
        let apps_cache = INSTALLED_APPS_CACHE.get_or_init(|| std::sync::Mutex::new(Vec::new()));
        let empty = apps_cache.lock().map(|a| a.is_empty()).unwrap_or(true);
        if empty && !IS_SCANNING.load(Ordering::Relaxed) { crate::services::trigger_app_scan(); }
        let start = std::time::Instant::now();
        while IS_SCANNING.load(Ordering::Relaxed) && start.elapsed() < std::time::Duration::from_secs(10) {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    let mut matched_app_path: Option<String> = None;
    if let Some(apps_cache) = INSTALLED_APPS_CACHE.get() {
        if let Ok(apps) = apps_cache.lock() {
            let scanned = apps.iter().find(|a| {
                a.icon.is_some()
                    && (a.path == path
                        || (match_by_name && lookup_name.as_deref().is_some_and(|n| a.name.eq_ignore_ascii_case(n))))
            });
            if let Some(scanned) = scanned {
                let icon = scanned.icon.clone().unwrap();
                if let Ok(mut c) = cache.lock() {
                    c.insert(cache_key.clone(), icon.clone());
                }
                schedule_icon_cache_save(app.clone());
                return Ok(Some(icon));
            }
            if match_by_name {
                matched_app_path = apps
                    .iter()
                    .find(|a| lookup_name.as_deref().is_some_and(|n| a.name.eq_ignore_ascii_case(n)))
                    .map(|a| a.path.clone());
            }
        }
    }

    {
        let path_for_pin = path.clone();
        let pin_icon = tauri::async_runtime::spawn_blocking(move || unsafe {
            use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED, CoUninitialize};
            let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let icon = crate::taskbar_pins::icon_for_target(&path_for_pin);
            if com { CoUninitialize(); }
            icon
        }).await.unwrap_or(None);

        if let Some(icon) = pin_icon {
            if let Ok(mut c) = cache.lock() { c.insert(cache_key.clone(), icon.clone()); }
            schedule_icon_cache_save(app.clone());
            return Ok(Some(icon));
        }
    }

    let cached = cache.lock().ok().and_then(|c| c.get(&cache_key).cloned());
    if let Some(icon) = cached {
        let stale = tauri::async_runtime::spawn_blocking({
            let icon = icon.clone();
            move || is_generic_icon(&icon)
        })
        .await
        .unwrap_or(false);
        if !stale {
            return Ok(Some(icon));
        }
        if let Ok(mut c) = cache.lock() { c.remove(&cache_key); }
    }

    // Resolve bare names (notepad.exe) first so Store packages get their own logo, not the System32 stub.
    let mut path = matched_app_path.unwrap_or(path);
    if !path.contains('\\') && !path.contains('/') {
        if let Some(resolved) = resolve_executable_path(&path) {
            path = resolved;
        }
    }
    if let Some(moved) = crate::utils::moved_squirrel_exe(&path) {
        path = moved;
    }

    // Strategy 2: shell-resolved icons for Store/UWP/PWA ids and exes inside MSIX packages.
    let aumid_opt = if is_aumid_path(&path) {
        Some(path.trim().to_string())
    } else {
        None
    };
    let packaged_exe = path.to_lowercase().contains("\\windowsapps\\");
    if aumid_opt.is_some() || packaged_exe {
        let path_owned = path.clone();
        let result = tauri::async_runtime::spawn_blocking(move || {
            let com_ok = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED).is_ok() };
            let icon = match aumid_opt {
                Some(aumid) => icon_from_aumid(&aumid),
                None => packaged_exe_icon(&path_owned),
            };
            if com_ok { unsafe { windows::Win32::System::Com::CoUninitialize(); } }
            icon
        }).await.unwrap_or(None);

        if let Some(base64) = result {
            if let Ok(mut c) = cache.lock() { c.insert(cache_key.clone(), base64.clone()); }
            schedule_icon_cache_save(app);
            return Ok(Some(base64));
        }
    }

    // Strategy 3: Extract icon from live window HWND
    if let Some(h) = hwnd {
        let path_owned = path.clone();
        let result = tauri::async_runtime::spawn_blocking(move || unsafe {
            use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED, CoUninitialize};
            use windows::Win32::UI::WindowsAndMessaging::{GetClassLongPtrW, GCLP_HICON, WM_GETICON, ICON_BIG, SendMessageTimeoutW, SMTO_ABORTIFHUNG};
            
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

            // Browser-hosted PWAs only show the browser icon on the window, so go by the
            // window's AUMID (command line as fallback).
            if is_browser_host_process(&path_owned) {
                if let Some(icon) = pwa_icon_for_window(HWND(h as *mut _), &path_owned) {
                    CoUninitialize();
                    return Some(icon);
                }
            }

            let h_hwnd = HWND(h as *mut _);
            
            let mut h_icon = windows::Win32::UI::WindowsAndMessaging::HICON(GetClassLongPtrW(h_hwnd, GCLP_HICON) as *mut _);
            if h_icon.is_invalid() {
                h_icon = windows::Win32::UI::WindowsAndMessaging::HICON(GetClassLongPtrW(h_hwnd, windows::Win32::UI::WindowsAndMessaging::GCL_HICON) as *mut _);
            }

            if h_icon.is_invalid() {
                let mut res = 0usize;
                let _ = SendMessageTimeoutW(h_hwnd, WM_GETICON, windows::Win32::Foundation::WPARAM(ICON_BIG as usize), windows::Win32::Foundation::LPARAM(0), SMTO_ABORTIFHUNG, 250, Some(&mut res));
                if res != 0 { h_icon = windows::Win32::UI::WindowsAndMessaging::HICON(res as *mut _); }
            }
            
            let res = if !h_icon.is_invalid() { icon_to_base64(h_icon) } else { None };
            
            CoUninitialize();
            res
        }).await.unwrap_or(None);

        if let Some(base64) = result {
            if let Ok(mut c) = cache.lock() {
                c.insert(cache_key.clone(), base64.clone());
            }
            schedule_icon_cache_save(app);
            return Ok(Some(base64));
        }
    }

    // Strategy 4: Extract icon from file path
    let path_clone = path.clone();
    let ck_clone = cache_key.clone();
    let file_icon = tauri::async_runtime::spawn_blocking(move || unsafe {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, CoUninitialize};
        use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
        use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;
        
        let com_ok = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let result = {
            let (mut actual_path, args) = if path_clone.to_lowercase().ends_with(".lnk") {
                resolve_shortcut(&path_clone).unwrap_or((path_clone.clone(), String::new()))
            } else {
                (path_clone.clone(), String::new())
            };

            // Shortcuts can have their own icon file (Firefox web apps).
            let mut icon_data = if path_clone.to_lowercase().ends_with(".lnk") {
                get_shortcut_icon_location(&path_clone).and_then(|p| image_file_to_base64(&p))
            } else {
                None
            };

            // Web apps keep their icon in the browser profile.
            if icon_data.is_none() {
                if let Some(pwa_icon_path) = find_browser_pwa_icon(&actual_path, &args) {
                    icon_data = image_file_to_base64(&pwa_icon_path);
                }
            }

            // UWP shortcuts launch `explorer.exe shell:AppsFolder\<AUMID>`.
            if icon_data.is_none() {
                if let Some(aumid) = extract_arg(&args, "shell:AppsFolder\\") {
                    icon_data = icon_from_aumid(&aumid);
                }
            }

            if let Some(icon) = icon_data {
                if let Ok(mut lock) = ICON_CACHE.get().unwrap().lock() { lock.insert(ck_clone, icon.clone()); }
                if com_ok { CoUninitialize(); }
                return Some(icon);
            }

            // Robust path resolution for common apps
            if !std::path::Path::new(&actual_path).is_absolute() {
                let lower = actual_path.to_lowercase();
                if lower == "code" || lower == "code.exe" {
                    if let Ok(home) = std::env::var("USERPROFILE") {
                        let p = format!("{}\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe", home);
                        if std::path::Path::new(&p).exists() { actual_path = p; }
                    }
                } else if lower == "msedge" || lower == "msedge.exe" {
                    let candidates = [
                        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
                    ];
                    for c in &candidates {
                        if std::path::Path::new(c).exists() { actual_path = c.to_string(); break; }
                    }
                } else if lower == "notepad" || lower == "notepad.exe" {
                    let candidates = [
                        r"C:\Windows\System32\notepad.exe",
                        r"C:\Windows\notepad.exe",
                    ];
                    for c in &candidates {
                        if std::path::Path::new(c).exists() { actual_path = c.to_string(); break; }
                    }
                } else if lower == "explorer" || lower == "explorer.exe" {
                    let p = r"C:\Windows\explorer.exe";
                    if std::path::Path::new(p).exists() { actual_path = p.to_string(); }
                }
            }

            if actual_path.to_lowercase().starts_with("shell:") {
                if let Some(icon) = crate::utils::icon_from_parsing_name(&actual_path) {
                    if let Ok(mut lock) = ICON_CACHE.get().unwrap().lock() { lock.insert(ck_clone, icon.clone()); }
                    if com_ok { CoUninitialize(); }
                    return Some(icon);
                }
            }

            let mut shfi: SHFILEINFOW = std::mem::zeroed();
            let icon_path = &actual_path;
            let path_u16: Vec<u16> = icon_path.encode_utf16().chain(std::iter::once(0)).collect();
            let res = SHGetFileInfoW(windows::core::PCWSTR(path_u16.as_ptr()), Default::default(), Some(&mut shfi), std::mem::size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | SHGFI_LARGEICON);

            if res != 0 && !shfi.hIcon.is_invalid() {
                let mut base64_icon = icon_to_base64(shfi.hIcon);
                let _ = DestroyIcon(shfi.hIcon);
                if base64_icon.as_deref().is_some_and(is_generic_icon) {
                    base64_icon = icon_from_exe_neighbourhood(&actual_path);
                }
                if let Some(ref base64) = base64_icon {
                    if let Ok(mut lock) = ICON_CACHE.get().unwrap().lock() { lock.insert(ck_clone, base64.clone()); }
                }
                Some(base64_icon)
            } else { None }
        };
        if com_ok { CoUninitialize(); }
        result.flatten()
    }).await.map_err(|e| e.to_string())?;

    if file_icon.is_some() {
        schedule_icon_cache_save(app);
    }
    Ok(file_icon)
}

#[tauri::command]
pub async fn clear_icon_cache(app: AppHandle) -> Result<(), String> {
    let cache_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let cache_path = cache_dir.join("icons_cache.json");
    let _ = std::fs::remove_file(&cache_path);
    if let Some(c) = ICON_CACHE.get() {
        if let Ok(mut lock) = c.lock() {
            lock.clear();
        }
    }
    let _ = app.emit("icon-cache-cleared", ());
    Ok(())
}

#[tauri::command]
pub async fn set_custom_icon(app: AppHandle, cache_key: String, icon_data: String) -> Result<String, String> {
    use base64::Engine;
    use image::GenericImageView;

    // Validate and decode the base64 data URI
    let b64_str = if icon_data.starts_with("data:") {
        icon_data.split(',').nth(1).ok_or("Invalid data URI")?
    } else {
        &icon_data
    };

    let raw_bytes = base64::engine::general_purpose::STANDARD
        .decode(b64_str)
        .map_err(|e| format!("Invalid base64: {}", e))?;

    // Decode image to validate and get dimensions
    let img = image::load_from_memory(&raw_bytes)
        .map_err(|e| format!("Invalid image: {}", e))?;

    let (w, h) = img.dimensions();
    if w < 16 || h < 16 {
        return Err("Icon must be at least 16x16 pixels".into());
    }

    // Resize to fit within 256x256 preserving aspect ratio, then center on transparent canvas
    let target = 256u32;
    let final_img = if w > target || h > target {
        let scaled = img.resize(target, target, image::imageops::FilterType::Lanczos3);
        let (sw, sh) = scaled.dimensions();
        let mut canvas = image::RgbaImage::new(target, target);
        let ox = (target - sw) / 2;
        let oy = (target - sh) / 2;
        image::imageops::overlay(&mut canvas, &scaled, ox as i64, oy as i64);
        image::DynamicImage::ImageRgba8(canvas)
    } else {
        img
    };

    // Encode to PNG bytes
    let mut png_bytes: Vec<u8> = Vec::new();
    final_img.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| format!("Failed to encode PNG: {}", e))?;

    // Save to custom_icons directory
    let icons_dir = get_custom_icons_dir(&app)?;
    let filename = format!("{}.png", sanitize_filename(&cache_key));
    let file_path = icons_dir.join(&filename);
    std::fs::write(&file_path, &png_bytes).map_err(|e| e.to_string())?;

    // Update manifest mapping sanitized filename -> original cache key
    let manifest_path = icons_dir.join("custom_icons.json");
    let mut manifest: std::collections::HashMap<String, String> = if let Ok(content) = std::fs::read_to_string(&manifest_path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        std::collections::HashMap::new()
    };
    manifest.insert(sanitize_filename(&cache_key), cache_key);
    let _ = std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap_or_default());

    // Return the data URI for immediate use
    let b64 = base64::engine::general_purpose::STANDARD.encode(&png_bytes);
    Ok(format!("data:image/png;base64,{}", b64))
}

#[tauri::command]
pub async fn remove_custom_icon(app: AppHandle, path: String, name: Option<String>) -> Result<(), String> {
    let cache_key = get_cache_key(&path, name.as_deref());
    let icons_dir = get_custom_icons_dir(&app)?;
    let sanitized = sanitize_filename(&cache_key);
    let filename = format!("{}.png", sanitized);
    let file_path = icons_dir.join(&filename);
    if file_path.exists() {
        std::fs::remove_file(&file_path).map_err(|e| e.to_string())?;
    }
    // Remove from manifest
    let manifest_path = icons_dir.join("custom_icons.json");
    if let Ok(content) = std::fs::read_to_string(&manifest_path) {
        if let Ok(mut manifest) = serde_json::from_str::<std::collections::HashMap<String, String>>(&content) {
            manifest.remove(&sanitized);
            let _ = std::fs::write(&manifest_path, serde_json::to_string(&manifest).unwrap_or_default());
        }
    }
    // Also remove from persistent cache so the original icon is re-fetched
    if let Some(c) = ICON_CACHE.get() {
        if let Ok(mut lock) = c.lock() {
            lock.remove(&cache_key);
        }
    }
    schedule_icon_cache_save(app);
    Ok(())
}

#[tauri::command]
pub async fn get_custom_icons(app: AppHandle) -> Result<HashMap<String, String>, String> {
    let icons_dir = get_custom_icons_dir(&app)?;
    let manifest_path = icons_dir.join("custom_icons.json");
    let manifest: std::collections::HashMap<String, String> = if let Ok(content) = std::fs::read_to_string(&manifest_path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        std::collections::HashMap::new()
    };
    let mut result = HashMap::new();
    if let Ok(entries) = std::fs::read_dir(&icons_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("png") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    // Use original cache key from manifest, fallback to sanitized stem
                    let key = manifest.get(stem).cloned().unwrap_or_else(|| stem.to_string());
                    if let Ok(data) = std::fs::read(&path) {
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                        result.insert(key, format!("data:image/png;base64,{}", b64));
                    }
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod pwa_icon_tests {
    use super::*;
    use crate::types::AppInfo;
    use super::super::dock::{is_pwa_shortcut_for, pwa_launch_target};

    #[test]
    fn aumid_detection() {
        assert!(is_aumid_path("4DF9E0F8.Netflix_mcm4njqhnhss8!Netflix.App"));
        assert!(is_aumid_path("Microsoft.VisualStudioCode"));
        assert!(is_aumid_path("shell:AppsFolder\\4DF9E0F8.Netflix_mcm4njqhnhss8!Netflix.App"));
        assert!(is_aumid_path("{6D809377-6AF0-444B-8957-A3773F02200E}\\Blender Foundation\\Blender 4.5\\blender.exe"));
        assert!(is_aumid_path("{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\compmgmt.msc"));
        assert!(is_aumid_path("::{4234D49B-0245-4DF3-B780-3893943456E1}\\Microsoft.Getstarted_8wekyb3d8bbwe!App"));
        assert!(is_aumid_path("c:.users.user.appdata.local.capcut.apps.9.7.0.4153.capcut.exe"));
        assert!(!is_aumid_path("C:\\Windows\\explorer.exe"));
        assert!(!is_aumid_path("msedge.exe"));
        assert!(!is_aumid_path("msedge"));
        assert!(!is_aumid_path(""));

        assert_eq!(
            shell_parsing_name_for_aumid("::{4234D49B-0245-4DF3-B780-3893943456E1}\\Microsoft.Getstarted_8wekyb3d8bbwe!App"),
            "::{4234D49B-0245-4DF3-B780-3893943456E1}\\Microsoft.Getstarted_8wekyb3d8bbwe!App"
        );
        assert_eq!(
            shell_parsing_name_for_aumid("shell:AppsFolder\\4DF9E0F8.Netflix_mcm4njqhnhss8!Netflix.App"),
            "shell:AppsFolder\\4DF9E0F8.Netflix_mcm4njqhnhss8!Netflix.App"
        );
        assert_eq!(
            shell_parsing_name_for_aumid("{6D809377-6AF0-444B-8957-A3773F02200E}\\Blender Foundation\\Blender 4.5\\blender.exe"),
            "shell:AppsFolder\\{6D809377-6AF0-444B-8957-A3773F02200E}\\Blender Foundation\\Blender 4.5\\blender.exe"
        );
        assert_eq!(
            shell_parsing_name_for_aumid("4DF9E0F8.Netflix_mcm4njqhnhss8!Netflix.App"),
            "shell:AppsFolder\\4DF9E0F8.Netflix_mcm4njqhnhss8!Netflix.App"
        );
        assert_eq!(
            shell_parsing_name_for_aumid("c:.users.user.appdata.local.capcut.apps.9.7.0.4153.capcut.exe"),
            "shell:AppsFolder\\c:.users.user.appdata.local.capcut.apps.9.7.0.4153.capcut.exe"
        );
    }

    #[test]
    fn url_shortcut_icon_parsing_and_legacy_pwa_repair() {
        let ini = "[InternetShortcut]\r\nURL=https://example.com\r\nIconIndex=2\r\nIconFile=C:\\Icons\\app.ico\r\n";
        assert_eq!(
            parse_url_shortcut_icon(ini),
            Some(("C:\\Icons\\app.ico".to_string(), 2))
        );
        assert_eq!(
            repair_legacy_pwa_path("C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe", Some("--app-id=xyz123")).as_deref(),
            Some("shell:AppsFolder\\MSEdge._crx_xyz123")
        );
        assert_eq!(
            repair_legacy_pwa_path("shell:AppsFolder\\Chrome._crx_abc", Some("--app-id=abc")),
            None
        );
    }

    #[test]
    fn command_line_arg_parsing() {
        let args = "--profile-directory=\"Profile 1\" --app-id=abcdef --ip-aumid=Package_Pub!App";
        assert_eq!(extract_arg(args, "--app-id="), Some("abcdef".into()));
        assert_eq!(extract_arg(args, "--profile-directory="), Some("Profile 1".into()));
        assert_eq!(extract_arg(args, "--ip-aumid="), Some("Package_Pub!App".into()));
        assert_eq!(extract_arg(args, "--missing="), None);
        assert_eq!(extract_arg("shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App", "shell:AppsFolder\\"), Some("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into()));
    }

    #[test]
    fn package_family_from_paths() {
        assert_eq!(
            package_family_from_windows_apps_path("C:\\Program Files\\WindowsApps\\5319275A.WhatsAppDesktop_2.2634.101.0_x64__cv1g1gvanyjgm\\WhatsApp.Root.exe").as_deref(),
            Some("5319275A.WhatsAppDesktop_cv1g1gvanyjgm")
        );
        assert_eq!(package_family_from_windows_apps_path("C:\\Windows\\explorer.exe"), None);
        assert_eq!(package_family_from_windows_apps_path("C:\\Program Files\\App\\app.exe"), None);
    }

    #[test]
    fn browser_web_app_ids_from_aumids() {
        assert_eq!(browser_web_app_id_from_aumid("Chrome.edhbnieanoeijlkpgkminebadpibapgm").as_deref(), Some("edhbnieanoeijlkpgkminebadpibapgm"));
        assert_eq!(browser_web_app_id_from_aumid("Chrome._crx_edhbnieanoeijlkpgkminebadpibapgm").as_deref(), Some("edhbnieanoeijlkpgkminebadpibapgm"));
        assert_eq!(browser_web_app_id_from_aumid("4DF9E0F8.Netflix_mcm4njqhnhss8!Netflix.App"), None);
        assert_eq!(browser_web_app_id_from_aumid("MSEdge"), None);
    }

    #[test]
    fn browser_pwa_aumid_detection() {
        assert!(is_browser_pwa_aumid("Chrome.edhbnieanoeijlkpgkminebadpibapgm"));
        assert!(is_browser_pwa_aumid("Brave._crx_edhbnieanoeijlkpgkminebadpibapgm"));
        assert!(is_browser_pwa_aumid("Brave._crx_agimnkijcamfeangaknmldooml"));
        assert!(is_browser_pwa_aumid("4DF9E0F8.Netflix_mcm4njqhnhss8!Netflix.App"));
        assert!(!is_browser_pwa_aumid("MSEdge"));
        assert!(!is_browser_pwa_aumid("Chrome"));
        assert!(!is_browser_pwa_aumid("Brave"));
    }

    #[test]
    fn image_size_scoring_prefers_larger_dimensions() {
        use std::path::Path;
        assert!(image_size_score(Path::new("C:\\x\\Icons\\256.png")) > image_size_score(Path::new("C:\\x\\Icons\\64.png")));
        assert!(image_size_score(Path::new("C:\\x\\512x512.png")) > image_size_score(Path::new("C:\\x\\192x192.png")));
        assert_eq!(image_size_score(Path::new("C:\\x\\icon.png")), 0);
    }

    #[test]
    fn pwa_shortcut_matching() {
        let app = AppInfo {
            name: "YouTube".into(),
            path: "C:\\Users\\x\\Start Menu\\Programs\\YouTube.lnk".into(),
            icon: None,
            is_running: false,
            hwnd: None,
            executable: Some("brave.exe".into()),
            all_hwnds: None,
            args: None,
        };
        assert!(is_pwa_shortcut_for(&app, "youtube", "brave.exe"));
        assert!(is_pwa_shortcut_for(&app, "YouTube", "BRAVE.EXE"));
        assert!(!is_pwa_shortcut_for(&app, "YouTube", "msedge.exe"));
        assert!(!is_pwa_shortcut_for(&app, "YouTube Music", "brave.exe"));
    }

    #[test]
    fn pwa_launch_target_resolution() {
        // Non-browser paths and missing titles don't resolve.
        assert_eq!(pwa_launch_target("C:\\Windows\\notepad.exe", Some("Notepad")), None);
        assert_eq!(pwa_launch_target("C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", None), None);
        assert_eq!(pwa_launch_target("C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", Some("   ")), None);

        // With the cache filled, a running PWA resolves to its shortcut, a same-titled app in another browser doesn't.
        let _ = INSTALLED_APPS_CACHE.set(std::sync::Mutex::new(vec![
            AppInfo {
                name: "YouTube".into(),
                path: "C:\\Start Menu\\YouTube.lnk".into(),
                icon: None,
                is_running: false,
                hwnd: None,
                executable: Some("brave.exe".into()),
                all_hwnds: None,
                args: None,
            },
            AppInfo {
                name: "Netflix".into(),
                path: "C:\\Start Menu\\Netflix.lnk".into(),
                icon: None,
                is_running: false,
                hwnd: None,
                executable: Some("msedge.exe".into()),
                all_hwnds: None,
                args: None,
            },
        ]));
        assert_eq!(
            pwa_launch_target("C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", Some("YouTube")),
            Some("C:\\Start Menu\\YouTube.lnk".into())
        );
        assert_eq!(
            pwa_launch_target("C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe", Some("YouTube")),
            None
        );
        assert_eq!(
            pwa_launch_target("C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe", Some("Netflix")),
            Some("C:\\Start Menu\\Netflix.lnk".into())
        );
    }
}

pub(crate) fn parse_url_shortcut_icon(content: &str) -> Option<(String, i32)> {
    let mut icon_file = None;
    let mut icon_index = 0i32;
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some((k, v)) = trimmed.split_once('=') {
            if k.trim().eq_ignore_ascii_case("IconFile") {
                let val = crate::taskbar_pins::expand_env(v.trim());
                if !val.is_empty() {
                    icon_file = Some(val);
                }
            } else if k.trim().eq_ignore_ascii_case("IconIndex") {
                icon_index = v.trim().parse::<i32>().unwrap_or(0);
            }
        }
    }
    icon_file.map(|f| (f, icon_index))
}

fn url_shortcut_icon(path: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let (icon_file, icon_index) = parse_url_shortcut_icon(&content)?;
    let icon_path = std::path::Path::new(&icon_file);
    if !icon_path.exists() {
        return None;
    }
    let ext = icon_path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if matches!(ext.as_str(), "png" | "ico" | "jpg" | "jpeg" | "bmp" | "webp") {
        if let Ok(img) = image::open(icon_path) {
            let mut png = Vec::new();
            if img.thumbnail(128, 128).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_ok() {
                use base64::Engine;
                return Some(format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png)));
            }
        }
    }
    unsafe {
        use windows::Win32::UI::Shell::ExtractIconExW;
        use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};
        let wide: Vec<u16> = icon_file.encode_utf16().chain(Some(0)).collect();
        let mut large = HICON::default();
        let count = ExtractIconExW(windows::core::PCWSTR(wide.as_ptr()), icon_index, Some(&mut large), None, 1);
        if count > 0 && !large.is_invalid() {
            let b64 = icon_to_base64(large);
            let _ = DestroyIcon(large);
            return b64;
        }
    }
    None
}

pub(crate) fn repair_legacy_pwa_path(path: &str, args: Option<&str>) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    if lower.starts_with("shell:appsfolder\\") {
        return None;
    }
    let args_str = args?;
    let app_id = args_str
        .split_whitespace()
        .find_map(|tok| tok.strip_prefix("--app-id="))
        .map(|s| s.trim_matches('"'))?;
    if app_id.is_empty() {
        return None;
    }
    if lower.ends_with("msedge.exe") {
        Some(format!("shell:AppsFolder\\MSEdge._crx_{app_id}"))
    } else if lower.ends_with("chrome.exe") {
        Some(format!("shell:AppsFolder\\Chrome._crx_{app_id}"))
    } else if lower.ends_with("brave.exe") {
        Some(format!("shell:AppsFolder\\Brave._crx_{app_id}"))
    } else {
        None
    }
}
