use std::sync::OnceLock;
use windows::core::Interface;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_ALL};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

/// RAII guard for COM initialization on worker/blocking threads.
/// Uninitializes only when `CoInitializeEx` returned `S_OK` or `S_FALSE` (`is_ok()`),
/// leaving mismatched-apartment failures (`RPC_E_CHANGED_MODE`) untouched.
#[must_use]
pub struct ComGuard {
    pub initialized: bool,
}

impl ComGuard {
    pub fn mta() -> Self {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
        Self {
            initialized: unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok(),
        }
    }

    pub fn sta() -> Self {
        use windows::Win32::System::Com::{
            CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        };
        Self {
            initialized: unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
            }
            .is_ok(),
        }
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

pub fn resolve_shortcut(path: &str) -> Option<(String, String)> {
    unsafe {
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_ALL).ok()?;
        let persist_file: IPersistFile = shell_link.cast().ok()?;

        let wide_path: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        persist_file.Load(windows::core::PCWSTR(wide_path.as_ptr()), windows::Win32::System::Com::STGM(0)).ok()?;

        let _ = shell_link.Resolve(HWND(std::ptr::null_mut()), 1 | 16 | 32);

        let mut buffer = [0u16; 260];
        let mut data = windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW::default();
        shell_link.GetPath(&mut buffer, &mut data, 0).ok()?;

        let mut arg_buffer = [0u16; 1024];
        let _ = shell_link.GetArguments(&mut arg_buffer);

        let target = String::from_utf16_lossy(&buffer).trim_matches(char::from(0)).to_string();
        let args = String::from_utf16_lossy(&arg_buffer).trim_matches(char::from(0)).to_string();

        if target.trim().is_empty() { None } else { Some((target, args)) }
    }
}

pub fn exe_description(path: &str) -> Option<String> {
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    static CACHE: OnceLock<std::sync::Mutex<std::collections::HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(path).cloned()) {
        return hit;
    }

    let result = unsafe {
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        let path_w = wide(path);
        let size = GetFileVersionInfoSizeW(windows::core::PCWSTR(path_w.as_ptr()), None);
        if size == 0 {
            None
        } else {
            let mut buf = vec![0u8; size as usize];
            if GetFileVersionInfoW(windows::core::PCWSTR(path_w.as_ptr()), None, size, buf.as_mut_ptr() as *mut _).is_err() {
                None
            } else {
                let query = |sub: &str| -> Option<(*mut std::ffi::c_void, u32)> {
                    let sub_w = wide(sub);
                    let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
                    let mut len = 0u32;
                    let ok = VerQueryValueW(buf.as_ptr() as *const _, windows::core::PCWSTR(sub_w.as_ptr()), &mut ptr, &mut len);
                    if ok.as_bool() && !ptr.is_null() && len > 0 { Some((ptr, len)) } else { None }
                };
                query("\\VarFileInfo\\Translation").and_then(|(ptr, _)| {
                    let lang = *(ptr as *const u16);
                    let codepage = *(ptr as *const u16).add(1);
                    let (desc, _) = query(&format!("\\StringFileInfo\\{:04x}{:04x}\\FileDescription", lang, codepage))?;
                    let s = String::from_utf16_lossy(windows::core::PCWSTR(desc as *const u16).as_wide()).trim().to_string();
                    if s.is_empty() { None } else { Some(s) }
                })
            }
        }
    };

    if let Ok(mut c) = cache.lock() { c.insert(path.to_string(), result.clone()); }
    result
}

/// Icon file a shortcut declares for itself (Firefox web apps, customized shortcuts).
/// Only real image files, icon references into exes/DLLs are ignored.
pub fn get_shortcut_icon_location(path: &str) -> Option<String> {
    unsafe {
        let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_ALL).ok()?;
        let persist_file: IPersistFile = shell_link.cast().ok()?;
        let wide_path: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        persist_file.Load(windows::core::PCWSTR(wide_path.as_ptr()), windows::Win32::System::Com::STGM(0)).ok()?;

        let mut buffer = [0u16; 512];
        let mut index = 0i32;
        shell_link.GetIconLocation(&mut buffer, &mut index).ok()?;

        let raw = String::from_utf16_lossy(&buffer).trim_matches(char::from(0)).trim().to_string();
        if raw.is_empty() { return None; }

        let location = expand_env_vars(&raw);
        let ext = std::path::Path::new(&location).extension()?.to_str()?.to_lowercase();
        if !matches!(ext.as_str(), "ico" | "png" | "jpg" | "jpeg" | "bmp" | "webp" | "gif") { return None; }
        if !std::path::Path::new(&location).exists() { return None; }
        Some(location)
    }
}

/// Expands %VAR% from the process environment.
fn expand_env_vars(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('%') {
            let name = &after[..end];
            if !name.is_empty() && !name.contains(' ') {
                match std::env::var(name) {
                    Ok(value) => result.push_str(&value),
                    Err(_) => {
                        result.push('%');
                        result.push_str(name);
                        result.push('%');
                    }
                }
            } else {
                result.push('%');
                result.push_str(name);
                result.push('%');
            }
            rest = &after[end + 1..];
        } else {
            result.push_str(&rest[start..]);
            rest = "";
            break;
        }
    }
    result.push_str(rest);
    result
}

/// Resolves a bare exe name (notepad.exe, msedge) to a full path: App Paths registry, then the
/// search path, then known install locations. Default pins need this, notepad.exe resolves to the
/// Store package whose icon comes from the manifest.
pub fn resolve_executable_path(name: &str) -> Option<String> {
    let trimmed = name.trim().trim_matches('"');
    if trimmed.is_empty() || trimmed.contains('\\') || trimmed.contains('/') { return None; }

    let file = if trimmed.to_lowercase().ends_with(".exe") {
        trimmed.to_string()
    } else {
        format!("{}.exe", trimmed)
    };

    if let Some(path) = app_paths_lookup(&file) {
        if std::path::Path::new(&path).exists() { return Some(path); }
    }
    if let Some(path) = search_system_path(&file) {
        return Some(path);
    }
    known_install_location(&file)
}

/// The version in a Squirrel version folder's name: `app-1.0.9264` is
/// `[1, 0, 9264]`. Compared as numbers, so `app-1.0.10000` is newer.
fn squirrel_folder_version(name: &str) -> Option<Vec<u64>> {
    let version = name
        .get(..4)?
        .eq_ignore_ascii_case("app-")
        .then(|| &name[4..])?;
    version
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect()
}

/// The newest of a Squirrel app's version folders, by name.
fn newest_squirrel_folder<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    names
        .into_iter()
        .filter_map(|name| squirrel_folder_version(name).map(|v| (v, name)))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, name)| name)
}

/// Resolves the current executable for a Squirrel-installed app (Discord, Slack,
/// GitHub Desktop, Postman, Figma) when its old `app-<version>` folder was deleted
/// by an update, or returns None if the path still exists or isn't Squirrel-managed.
pub fn moved_squirrel_exe(path: &str) -> Option<String> {
    let path = std::path::Path::new(path.trim().trim_matches('"'));
    if path.exists() {
        return None;
    }
    let folder = path.parent()?;
    let folder_name = folder.file_name()?.to_str()?;
    if squirrel_folder_version(folder_name).is_none()
        && !folder_name.eq_ignore_ascii_case("current")
    {
        return None;
    }
    let root = folder.parent()?;
    let exe = path.file_name()?;
    let names: Vec<String> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().join(exe).is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let newest = newest_squirrel_folder(names.iter().map(String::as_str)).or_else(|| {
        names
            .iter()
            .find(|n| n.eq_ignore_ascii_case("current"))
            .map(String::as_str)
    })?;
    Some(root.join(newest).join(exe).to_string_lossy().into_owned())
}

/// App Paths registry lookup (HKCU, then HKLM).
fn app_paths_lookup(file: &str) -> Option<String> {
    use windows::Win32::System::Registry::{HKEY, RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};

    unsafe {
        let subkey: Vec<u16> = format!("Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{}", file)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            let mut key = HKEY::default();
            if RegOpenKeyExW(root, windows::core::PCWSTR(subkey.as_ptr()), None, KEY_READ, &mut key).0 != 0 { continue; }

            let mut buffer = [0u16; 1024];
            let mut size = (buffer.len() * 2) as u32;
            let status = RegQueryValueExW(
                key,
                windows::core::PCWSTR::null(),
                None,
                None,
                Some(buffer.as_mut_ptr() as *mut u8),
                Some(&mut size),
            );
            let _ = RegCloseKey(key);
            if status.0 != 0 { continue; }

            let value = String::from_utf16_lossy(&buffer[..(size as usize / 2).min(buffer.len())]);
            let value = value.trim_matches(char::from(0)).trim().trim_matches('"').to_string();
            if !value.is_empty() { return Some(value); }
        }
        None
    }
}

/// PATH, System32, Windows.
fn search_system_path(file: &str) -> Option<String> {
    use windows::Win32::Storage::FileSystem::SearchPathW;

    unsafe {
        let wide: Vec<u16> = file.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buffer = vec![0u16; 32768];
        let len = SearchPathW(None, windows::core::PCWSTR(wide.as_ptr()), windows::core::PCWSTR::null(), Some(&mut buffer), None);
        if len == 0 || len as usize >= buffer.len() { return None; }

        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        if std::path::Path::new(&path).exists() { Some(path) } else { None }
    }
}

/// Last resort for apps not in App Paths or PATH.
fn known_install_location(file: &str) -> Option<String> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let home = std::env::var("USERPROFILE").unwrap_or_default();

    let candidates: Vec<String> = match file.to_lowercase().as_str() {
        "code.exe" | "code-insiders.exe" => vec![
            format!("{}\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe", home),
            format!("{}\\Programs\\Microsoft VS Code\\Code.exe", local),
            "C:\\Program Files\\Microsoft VS Code\\Code.exe".into(),
            "C:\\Program Files (x86)\\Microsoft VS Code\\Code.exe".into(),
        ],
        "msedge.exe" => vec![
            "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe".into(),
            "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe".into(),
            format!("{}\\Microsoft\\Edge\\Application\\msedge.exe", local),
        ],
        "notepad.exe" => vec![
            "C:\\Windows\\System32\\notepad.exe".into(),
            "C:\\Windows\\notepad.exe".into(),
        ],
        "explorer.exe" => vec!["C:\\Windows\\explorer.exe".into()],
        _ => Vec::new(),
    };

    candidates.into_iter().find(|c| std::path::Path::new(c).exists())
}

/// The window's AppUserModelID, same identity the taskbar groups by
/// (package id for Store apps, web app id for browser web apps).
pub fn get_window_app_user_model_id(hwnd: HWND) -> Option<String> {
    unsafe {
        use windows::Win32::Foundation::PROPERTYKEY;
        use windows::Win32::System::Com::StructuredStorage::{PropVariantClear, PropVariantToStringAlloc};
        use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};

        const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
            fmtid: windows::core::GUID {
                data1: 0x9F4C2855,
                data2: 0x9F79,
                data3: 0x4B39,
                data4: [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3],
            },
            pid: 5,
        };

        let store: IPropertyStore = SHGetPropertyStoreForWindow(hwnd).ok()?;
        let mut prop = store.GetValue(&PKEY_APP_USER_MODEL_ID).ok()?;
        let result = PropVariantToStringAlloc(&prop)
            .ok()
            .map(|pwstr| String::from_utf16_lossy(pwstr.as_wide()).trim().to_string());
        let _ = PropVariantClear(&mut prop);
        result.filter(|s| !s.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::expand_env_vars;

    #[test]
    fn env_expansion() {
        let out = expand_env_vars("%SystemRoot%\\System32");
        assert!(out.to_lowercase().ends_with("\\system32"), "got {out}");
        assert!(!out.contains('%'), "got {out}");

        // Unknown variables stay verbatim
        assert_eq!(expand_env_vars("%FERN_NOT_A_REAL_VAR%\\x"), "%FERN_NOT_A_REAL_VAR%\\x");

        // Unclosed percent is left alone
        assert_eq!(expand_env_vars("50% done"), "50% done");

        // Plain paths are untouched
        assert_eq!(expand_env_vars("C:\\plain\\path"), "C:\\plain\\path");
    }

    #[test]
    fn bare_executable_resolution() {
        use super::resolve_executable_path;

        for name in ["notepad", "notepad.exe", "msedge"] {
            let path = resolve_executable_path(name).unwrap_or_else(|| panic!("{name} did not resolve"));
            assert!(std::path::Path::new(&path).exists(), "{name} -> {path} does not exist");
            assert!(path.to_lowercase().ends_with(".exe"), "{name} -> {path}");
        }

        // Already-qualified paths and nonsense are not resolved
        assert_eq!(resolve_executable_path("C:\\Windows\\notepad.exe"), None);
        assert_eq!(resolve_executable_path("fern-definitely-not-installed"), None);
        assert_eq!(resolve_executable_path(""), None);
    }

    #[test]
    fn picks_the_newest_squirrel_version_by_number() {
        use super::newest_squirrel_folder;
        assert_eq!(
            newest_squirrel_folder(["app-1.0.9264", "app-1.0.10000", "app-1.0.999", "packages"]),
            Some("app-1.0.10000")
        );
        assert_eq!(
            newest_squirrel_folder(["App-3.2", "app-3.10"]),
            Some("app-3.10")
        );
        assert_eq!(
            newest_squirrel_folder(["app-beta", "current", "packages"]),
            None
        );
    }

    #[test]
    fn finds_a_squirrel_app_after_an_update() {
        use super::moved_squirrel_exe;
        let root = std::env::temp_dir().join(format!("fern-squirrel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old_dir = root.join("app-1.0.999");
        let new_dir = root.join("app-1.0.1000");
        std::fs::create_dir_all(&new_dir).unwrap();
        let current = new_dir.join("Discord.exe");
        std::fs::write(&current, b"").unwrap();

        let stale = old_dir.join("Discord.exe");
        assert_eq!(
            moved_squirrel_exe(&stale.to_string_lossy()),
            Some(current.to_string_lossy().into_owned())
        );
        assert_eq!(moved_squirrel_exe(&current.to_string_lossy()), None);
        assert_eq!(
            moved_squirrel_exe(&root.join("Discord.exe").to_string_lossy()),
            None
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
