
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegDeleteTreeW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK, MB_TOPMOST, MB_YESNO, SW_SHOWNORMAL,
};
use windows::core::PCWSTR;

use crate::uninstall_registry::{
    desktop_dir, legacy_start_menu_dir, start_menu_programs_dir, wide, UNINSTALL_SUBKEY,
};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const TITLE: &str = "Fern";

pub fn requested() -> bool {
    let by_arg = std::env::args().skip(1).any(|a| a == "--uninstall");
    let by_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .is_some_and(|n| n == "uninstall.exe");
    by_arg || by_name
}

fn message(text: &str, flags: windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_STYLE) -> windows::Win32::UI::WindowsAndMessaging::MESSAGEBOX_RESULT {
    let text_w = wide(text);
    let title_w = wide(TITLE);
    unsafe { MessageBoxW(None, PCWSTR(text_w.as_ptr()), PCWSTR(title_w.as_ptr()), flags | MB_TOPMOST) }
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut _ as *mut _),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

fn relaunch_elevated(exe: &Path) -> bool {
    let exe_w: Vec<u16> = exe.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let verb_w = wide("runas");
    let args_w = wide("--uninstall");
    let result = unsafe {
        ShellExecuteW(None, PCWSTR(verb_w.as_ptr()), PCWSTR(exe_w.as_ptr()), PCWSTR(args_w.as_ptr()), None, SW_SHOWNORMAL)
    };
    (result.0 as isize) > 32
}

fn run_hidden(program: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
}

fn stop_running_fern() {
    let not_self = format!("PID ne {}", std::process::id());
    let filters = ["/FI", "IMAGENAME eq fern.exe", "/FI", not_self.as_str()];

    let still_running = || {
        run_hidden("tasklist", &filters).is_some_and(|out| out.to_lowercase().contains("fern.exe"))
    };

    let _ = run_hidden("taskkill", &filters);
    for _ in 0..16 {
        if !still_running() { return; }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }

    let mut forced = vec!["/F"];
    forced.extend_from_slice(&filters);
    let _ = run_hidden("taskkill", &forced);
    for _ in 0..8 {
        if !still_running() { return; }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

fn restore_taskbar_if_left_hidden() {
    let Ok(appdata) = std::env::var("APPDATA") else { return };
    let flag: PathBuf = Path::new(&appdata).join("com.kagangungor.fern").join("taskbar_hidden.flag");
    if flag.exists() {
        crate::utils::set_taskbar_visibility(true, true);
        let _ = std::fs::remove_file(&flag);
    }
}

fn remove_shortcuts(all_users: bool) {
    let mut lnks = Vec::new();
    if let Some(d) = start_menu_programs_dir(all_users) { lnks.push(d.join("Fern.lnk")); }
    if let Some(d) = legacy_start_menu_dir(all_users) { lnks.push(d.join("Fern.lnk")); }
    if let Some(d) = desktop_dir(all_users) { lnks.push(d.join("Fern.lnk")); }
    for lnk in lnks {
        let _ = std::fs::remove_file(lnk);
    }
}

fn remove_registry(all_users: bool) {
    unsafe {
        let root = if all_users { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER };
        let subkey_w = wide(UNINSTALL_SUBKEY);
        let _ = RegDeleteTreeW(root, PCWSTR(subkey_w.as_ptr()));
        let install_key = wide(r"Software\kagangungor\fern");
        let _ = RegDeleteTreeW(root, PCWSTR(install_key.as_ptr()));

        let run_key = wide(r"Software\Microsoft\Windows\CurrentVersion\Run");
        let approved_key = wide(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
        let value = wide("fern");
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(run_key.as_ptr()), PCWSTR(value.as_ptr()));
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, PCWSTR(approved_key.as_ptr()), PCWSTR(value.as_ptr()));
    }
}

// Files Fern itself puts in the install folder (the payload is just fern.exe)
const APP_FILES: &[&str] = &["fern.exe", "uninstall.exe", ".install-scope"];

// A short name like "fern" also appears in other words, and a match means the whole folder gets deleted.
fn is_fern_folder_name(name: &str) -> bool {
    name == "fern" || ["fern-", "fern_", "fern "].iter().any(|prefix| name.starts_with(prefix))
}

// The uninstaller deletes a dedicated folder with everything in it. Someone may have installed into
// an existing folder that happens to be called "fern-projects", so anything that isn't ours rules it out.
fn holds_only_app_files(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.flatten().all(|entry| {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            entry.path().is_file() && APP_FILES.iter().any(|file| file.eq_ignore_ascii_case(&name))
        }),
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

// True when the folder belongs to Fern alone and can be removed as a whole
fn is_dedicated_install_dir(dir: &Path) -> bool {
    let Some(name) = dir.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
        return false;
    };

    // Folder name must identify a Fern install
    if !is_fern_folder_name(&name) || !holds_only_app_files(dir) {
        return false;
    }

    // Never a drive root
    if dir.parent().is_none() {
        return false;
    }

    // Never the user profile, Desktop or Downloads
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let user_path = Path::new(&userprofile);
        if dir == user_path || dir == user_path.join("Desktop") || dir == user_path.join("Downloads") {
            return false;
        }
    }

    true
}

fn remove_dir_command(install_dir: &Path, run_once: &str) -> String {
    let dir = install_dir.to_string_lossy().replace('\'', "''");
    format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; \
         for ($i = 0; $i -lt 40 -and (Test-Path -LiteralPath '{dir}'); $i++) {{ \
           Remove-Item -LiteralPath '{dir}' -Recurse -Force -ErrorAction SilentlyContinue -ErrorVariable failed; \
           if (Test-Path -LiteralPath '{dir}') {{ Start-Sleep -Milliseconds 500 }} \
         }}; \
         if (Test-Path -LiteralPath '{dir}') {{ \
           Add-Content (Join-Path $env:TEMP 'fern-uninstall.log') (\"Could not remove {dir}: \" + ($failed | Out-String)); \
           New-ItemProperty -Path '{run_once}' -Name 'FernCleanup' -Value 'cmd /c rd /s /q \"{dir}\"' -Force -ErrorAction SilentlyContinue | Out-Null \
         }}",
        pid = std::process::id(),
    )
}

fn remove_app_files_command(install_dir: &Path, run_once: &str) -> String {
    let dir = install_dir.to_string_lossy().replace('\'', "''");
    let paths: Vec<String> = APP_FILES
        .iter()
        .map(|f| install_dir.join(f).to_string_lossy().replace('\'', "''"))
        .collect();
    let files = paths.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(", ");
    let del_args = paths.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(" ");
    format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; \
         $files = @({files}); \
         $left = {{ @($files | Where-Object {{ Test-Path -LiteralPath $_ }}) }}; \
         for ($i = 0; $i -lt 40 -and (& $left).Count; $i++) {{ \
           & $left | ForEach-Object {{ Remove-Item -LiteralPath $_ -Force -ErrorAction SilentlyContinue -ErrorVariable +failed }}; \
           if ((& $left).Count) {{ Start-Sleep -Milliseconds 500 }} \
         }}; \
         if ((& $left).Count) {{ \
           Add-Content (Join-Path $env:TEMP 'fern-uninstall.log') (\"Could not remove Fern files from {dir}: \" + ($failed | Out-String)); \
           New-ItemProperty -Path '{run_once}' -Name 'FernCleanup' -Value 'cmd /c del /f /q {del_args}' -Force -ErrorAction SilentlyContinue | Out-Null \
         }}",
        pid = std::process::id(),
    )
}

pub fn run() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(install_dir) = exe.parent().map(Path::to_path_buf) else { return };

    let all_users = crate::uninstall_registry::install_scope(&install_dir).unwrap_or(false);

    if all_users && !is_elevated() {
        if !relaunch_elevated(&exe) {
            message("Fern was installed for all users, so uninstalling it needs administrator permission.", MB_OK | MB_ICONINFORMATION);
        }
        return;
    }

    if message(
        "Uninstall Fern?\n\nThis removes the app and its shortcuts. Your settings are kept in case you reinstall.",
        MB_YESNO | MB_ICONQUESTION,
    ) != IDYES
    {
        return;
    }

    stop_running_fern();
    restore_taskbar_if_left_hidden();
    remove_shortcuts(all_users);
    remove_registry(all_users);

    let dedicated = is_dedicated_install_dir(&install_dir);
    let run_once = if all_users {
        r"HKLM:\Software\Microsoft\Windows\CurrentVersion\RunOnce"
    } else {
        r"HKCU:\Software\Microsoft\Windows\CurrentVersion\RunOnce"
    };
    let ps_command = if dedicated {
        remove_dir_command(&install_dir, run_once)
    } else {
        // Shared folder (e.g. D:\Apps): remove only Fern's own files, never the folder
        remove_app_files_command(&install_dir, run_once)
    };
    let spawn = |flags: u32| {
        std::process::Command::new("powershell")
            .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &ps_command])
            .current_dir(std::env::temp_dir())
            .creation_flags(flags)
            .spawn()
    };
    let spawned = spawn(CREATE_NO_WINDOW | 0x0100_0000).or_else(|_| spawn(CREATE_NO_WINDOW));

    if spawned.is_err() {
        let text = if dedicated {
            format!("Fern was unregistered, but its folder couldn't be removed automatically. You can delete it yourself:\n\n{}", install_dir.display())
        } else {
            format!("Fern was unregistered, but its files couldn't be removed automatically. You can delete {} yourself from:\n\n{}", APP_FILES.join(", "), install_dir.display())
        };
        message(&text, MB_OK | MB_ICONINFORMATION);
        return;
    }
    message("Fern has been uninstalled.", MB_OK | MB_ICONINFORMATION);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedicated_dir_rejects_a_folder_holding_other_files() {
        let dir = std::env::temp_dir().join(format!("fern-dedicated-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("FERN.EXE"), "x").unwrap();
        std::fs::write(dir.join("uninstall.exe"), "x").unwrap();
        assert!(is_dedicated_install_dir(&dir), "only the app's own files");

        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        assert!(!is_dedicated_install_dir(&dir), "a file that isn't ours");
        std::fs::remove_file(dir.join("notes.txt")).unwrap();

        std::fs::create_dir(dir.join("projects")).unwrap();
        assert!(!is_dedicated_install_dir(&dir), "a folder that isn't ours");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dedicated_dir_rejects_lookalikes() {
        assert!(is_dedicated_install_dir(Path::new(r"C:\Users\u\AppData\Local\Programs\Fern")));
        assert!(is_dedicated_install_dir(Path::new(r"D:\Apps\fern-beta")));
        assert!(is_dedicated_install_dir(Path::new(r"D:\Apps\Fern 2")));
        assert!(!is_dedicated_install_dir(Path::new(r"D:\Apps\Fernando")));
        assert!(!is_dedicated_install_dir(Path::new(r"D:\Apps\Ferndale")));
        assert!(!is_dedicated_install_dir(Path::new(r"D:\Apps\my-fern")));
    }

    #[test]
    fn dedicated_dir_needs_fern_in_its_name() {
        assert!(is_dedicated_install_dir(Path::new(r"C:\Users\u\AppData\Local\Programs\Fern")));
        assert!(is_dedicated_install_dir(Path::new(r"D:\Apps\fern-beta")));
        assert!(!is_dedicated_install_dir(Path::new(r"D:\Apps")));
    }

    #[test]
    fn drive_root_is_never_dedicated() {
        assert!(!is_dedicated_install_dir(Path::new(r"D:\")));
    }

    #[test]
    fn shared_folder_cleanup_only_targets_app_files() {
        let cmd = remove_app_files_command(Path::new(r"D:\Apps"), r"HKCU:\RunOnce");
        assert!(cmd.contains(r"'D:\Apps\fern.exe'"));
        assert!(cmd.contains(r"'D:\Apps\uninstall.exe'"));
        assert!(!cmd.contains("-Recurse"));
        assert!(!cmd.contains("rd /s"));
    }
}
