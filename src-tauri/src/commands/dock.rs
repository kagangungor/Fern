use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, Window};
use tauri_plugin_dialog::DialogExt;
use windows::Win32::Foundation::HWND;

use crate::services::{register_appbar, register_dock_appbar, sync_overlays, unregister_appbar_native};
use crate::state::*;
use crate::types::{AppInfo, IntRect};
use crate::utils::*;

use super::icons::{
    find_aumid_by_family, is_aumid_path, is_browser_host_process,
    package_family_from_windows_apps_path, shell_parsing_name_for_aumid,
};
use super::notch::wait_for_splash;
use super::windows::{send_win_chord, toggle_start_menu};

#[tauri::command]
pub async fn set_dock_hovered(window: Window, hovered: bool) {
    crate::state::set_flag(crate::state::dock_hovered(), window.label(), hovered);
}

#[tauri::command]
pub async fn set_dock_dragging(dragging: bool) {
    crate::state::DOCK_IS_DRAGGING.store(dragging, Ordering::Relaxed);
}

#[tauri::command]
pub async fn update_dock_rect(window: Window, rect: IntRect, expanded: Option<bool>) {
    if expanded == Some(true) {
        if let Ok(mut m) = crate::state::dock_overlap_rects().lock() {
            m.insert(window.label().to_string(), rect);
        }
    }
    if let Ok(mut m) = crate::state::dock_rects().lock() {
        m.insert(window.label().to_string(), rect);
    }
    // A preview closing under a still cursor changes what the cursor is over.
    crate::services::request_cursor_recheck();
}

#[tauri::command]
pub async fn update_dock_expanded_rect(window: Window, rect: IntRect) {
    if let Ok(mut m) = crate::state::dock_expanded_rects().lock() {
        m.insert(window.label().to_string(), rect);
    }
}

#[tauri::command]
pub async fn init_dock(app: AppHandle, mode: String) {
    // Backend guard: bail if dock is disabled in settings.
    // The frontend already checks this, but settings.json may have a stale
    // value if the write didn't complete before restart. Reading here too
    // makes the dock reliably stay hidden regardless of frontend timing.
    let enabled = get_setting_str(&app, "fern-dock-enabled")
        .unwrap_or_else(|| "true".to_string());
    if enabled != "true" {
        if let Some(dock_win) = app.get_webview_window("dock") {
            let _ = dock_win.hide();
            crate::state::set_flag(crate::state::dock_appbar_registered(), "dock", false);
        }
        crate::startup::settle("dock disabled");
        return;
    }

    wait_for_splash().await;

    if let Some(dock_win) = app.get_webview_window("dock") {
        init_dock_window(&app, dock_win, &mode).await;
    }
    crate::startup::mark("dock shown");
    crate::startup::settle("dock shown");
}

pub async fn init_dock_window(app: &AppHandle, dock_win: tauri::WebviewWindow, mode: &str) {
    let label = dock_win.label().to_string();

    // 1. Always show first — idempotent, required before any positioning
    if !dock_win.is_visible().unwrap_or(false) {
        let _ = dock_win.set_position(tauri::PhysicalPosition::new(-32000, -32000));
    }
    let _ = dock_win.show();
    crate::direct_flip::clear_hidden(&label);
    if let Ok(hwnd) = dock_win.hwnd() { re_assert_topmost(hwnd); }

    // 2. Register as appbar (fixed) or manually position (auto-hide)
    if mode == "fixed" {
        // register_dock_appbar calls show() internally too, and handles retries
        register_dock_appbar(dock_win.clone());
    } else {
        let dock_clone = dock_win.clone();
        tauri::async_runtime::spawn(async move {
            for attempt in 0..20 {
                // Wait for monitor and window dimensions to be available.
                // Never use a hardcoded fallback — wrong values produce off-screen placement.
                // Extract HWND as isize before any await (raw pointer is not Send).
                let hwnd_val = dock_clone.hwnd().map(|h| h.0 as isize).unwrap_or(0);
                let ph = dock_clone.outer_size().map(|s| s.height as i32).unwrap_or(0);
                let monitor_info = crate::monitors::monitor_for_window_label(dock_clone.app_handle(), crate::types::WindowKind::Dock, dock_clone.label()).map(|m| {
                    let s = m.size();
                    let p = m.position();
                    (tauri::PhysicalSize::new(s.width, s.height), tauri::PhysicalPosition::new(p.x, p.y))
                });

                if hwnd_val != 0 {
                    if let Some((m_size, m_pos)) = monitor_info {
                        if ph <= 10 {
                            // outer_size() not ready yet — retry next tick
                            if attempt < 19 {
                                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                            }
                            continue;
                        }
                        let final_y = m_pos.y + m_size.height as i32 - ph;
                        unsafe {
                            use windows::Win32::UI::WindowsAndMessaging::{
                                SetWindowPos, SWP_NOZORDER, SWP_NOACTIVATE, SWP_FRAMECHANGED
                            };
                            use windows::Win32::Foundation::HWND;
                            let _ = SetWindowPos(
                                HWND(hwnd_val as *mut _), None,
                                m_pos.x, final_y,
                                m_size.width as i32, ph,
                                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED
                            );
                        }
                        // Re-assert topmost after repositioning
                        if let Ok(hwnd) = dock_clone.hwnd() { re_assert_topmost(hwnd); }
                        // Ensure visible after positioning
                        let _ = dock_clone.show();
                        break;
                    }
                }
                if attempt < 19 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                }
            }
        });
    }

    // 3. Hide taskbar after showing dock (not before, so user always has something)
    set_taskbar_visibility(false, false);
    NATIVE_TASKBAR_HIDDEN.store(true, Ordering::Relaxed);
    crate::tray_icons::schedule_taskbar_layout_refresh();

    // 4. Reset overlap state so the overlap thread re-syncs cleanly
    crate::state::set_overlap(crate::state::dock_overlap(), &label, 0);
    let _ = app.emit_to(label.as_str(), "dock-overlap", false);
    // Sync the maximized state so the adaptive dock expands right away.
    if label == "dock" {
        let _ = app.emit_to(label.as_str(), "dock-maximized", CURRENT_FOREGROUND_MAXIMIZED.load(Ordering::Relaxed));
    }
}

#[tauri::command]
pub async fn toggle_dock(app: AppHandle, enable: bool) {
    if let Some(dock_win) = app.get_webview_window("dock") {
        if enable {
            // Saved dock mode, smart on a fresh install.
            let saved_mode = crate::utils::get_setting_str(&app, "fern-dock-mode")
                .unwrap_or_else(|| "smart".to_string());
            init_dock(app.clone(), saved_mode).await;
        } else {
            let _ = dock_win.hide();
            if let Ok(hwnd) = dock_win.hwnd() {
                let hwnd_val = hwnd.0 as isize;
                tauri::async_runtime::spawn_blocking(move || {
                    unregister_appbar_native(HWND(hwnd_val as *mut _));
                });
            }
            crate::state::set_flag(crate::state::dock_appbar_registered(), "dock", false);
            set_taskbar_visibility(true, true);
            NATIVE_TASKBAR_HIDDEN.store(false, Ordering::Relaxed);

            // Re-sync other appbars
            if let Some(main_win) = app.get_webview_window("main") {
                if crate::state::get_flag(crate::state::main_appbar_registered(), "main") {
                    register_appbar(main_win);
                }
            }
        }
    }
}

#[tauri::command]
pub async fn sync_appbar(app: AppHandle) {
    let dock_enabled = get_setting_str(&app, "fern-dock-enabled")
        .unwrap_or_else(|| "true".to_string());
    for (label, win) in app.webview_windows() {
        if crate::state::is_notch_label(&label) {
            if crate::state::get_flag(crate::state::main_appbar_registered(), &label) {
                register_appbar(win);
            } else if let Ok(hwnd) = win.hwnd() {
                re_assert_topmost(hwnd);
            }
        } else if crate::state::is_dock_label(&label) {
            if dock_enabled == "true" && crate::state::get_flag(crate::state::dock_appbar_registered(), &label) {
                register_dock_appbar(win);
            } else if let Ok(hwnd) = win.hwnd() {
                re_assert_topmost(hwnd);
            }
        }
    }
    sync_overlays(&app);
}

#[tauri::command]
pub async fn change_dock_mode(app: AppHandle, mode: String) {
    // A disabled dock keeps the mode in settings (init_dock applies it on re-enable),
    // just don't show it now.
    let enabled = get_setting_str(&app, "fern-dock-enabled").unwrap_or_else(|| "true".to_string());
    if enabled != "true" {
        return;
    }
    wait_for_splash().await;
    let per_monitor = crate::monitors::is_per_monitor_active(&app, crate::types::WindowKind::Dock);
    let labels: Vec<String> = app.webview_windows().keys()
        .filter(|l| crate::state::is_dock_label(l))
        .cloned()
        .collect();
    for label in labels {
        let effective = if per_monitor {
            crate::monitors::resolve_mode_for_label(&app, crate::types::WindowKind::Dock, &label)
        } else {
            mode.clone()
        };
        if let Some(dock_win) = app.get_webview_window(&label) {
            change_dock_mode_for_window(&app, dock_win, &effective).await;
        }
    }
}

async fn change_dock_mode_for_window(app: &AppHandle, dock_win: tauri::WebviewWindow, mode: &str) {
    let label = dock_win.label().to_string();
    if mode == "fixed" {
        register_dock_appbar(dock_win.clone());
    } else {
        let _ = dock_win.show();
        if let Ok(hwnd) = dock_win.hwnd() {
            let hwnd_val = hwnd.0 as isize;
            tauri::async_runtime::spawn_blocking(move || {
                unregister_appbar_native(HWND(hwnd_val as *mut _));
            });
            crate::state::set_flag(crate::state::dock_appbar_registered(), &label, false);

            let dock_clone = dock_win.clone();
            tauri::async_runtime::spawn(async move {
                for attempt in 0..10 {
                    // Never fall back to a hardcoded pixel height.
                    // Extract HWND as isize before any await (raw pointer is not Send).
                    let hwnd_val = dock_clone.hwnd().map(|h| h.0 as isize).unwrap_or(0);
                    let ph = dock_clone.outer_size().map(|s| s.height as i32).unwrap_or(0);
                    let monitor_info = crate::monitors::monitor_for_window_label(dock_clone.app_handle(), crate::types::WindowKind::Dock, dock_clone.label()).map(|m| {
                        let s = m.size();
                        let p = m.position();
                        (tauri::PhysicalSize::new(s.width, s.height), tauri::PhysicalPosition::new(p.x, p.y))
                    });

                    if hwnd_val != 0 {
                        if let Some((m_size, m_pos)) = monitor_info {
                            if ph <= 10 {
                                if attempt < 9 {
                                    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                                }
                                continue;
                            }
                            let final_y = m_pos.y + m_size.height as i32 - ph;
                            unsafe {
                                use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOZORDER, SWP_NOACTIVATE, SWP_FRAMECHANGED};
                                use windows::Win32::Foundation::HWND;
                                let _ = SetWindowPos(HWND(hwnd_val as *mut _), None, m_pos.x, final_y, m_size.width as i32, ph, SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
                            }
                            if let Ok(hwnd) = dock_clone.hwnd() { re_assert_topmost(hwnd); }
                            break;
                        }
                    }
                    if attempt < 9 {
                        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                    }
                }
            });
        }
    }

    // Ensure always on top and native taskbar stays hidden
    if let Ok(hwnd) = dock_win.hwnd() { re_assert_topmost(hwnd); }
    set_taskbar_visibility(false, false);
    NATIVE_TASKBAR_HIDDEN.store(true, Ordering::Relaxed);

    // Sync the current overlap state immediately to the frontend
    let current = crate::state::get_overlap(crate::state::dock_overlap(), &label);
    if current != -1 {
        let _ = app.emit_to(label.as_str(), "dock-overlap", current == 1);
    }
    if label == "dock" {
        let _ = app.emit_to(label.as_str(), "dock-maximized", CURRENT_FOREGROUND_MAXIMIZED.load(Ordering::Relaxed));
    }

    // Double sync after a short delay to catch any layout changes
    let dock_clone = dock_win.clone();
    let label_clone = label.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        if crate::state::get_flag(crate::state::dock_appbar_registered(), &label_clone) {
            register_dock_appbar(dock_clone);
        }
    });
}

fn get_uwp_launch_cmd(exe_path: &str) -> Option<String> {
    let path = std::path::Path::new(exe_path);
    let mut is_windows_apps = false;
    let mut package_folder = String::new();
    
    for component in path.components() {
        if let std::path::Component::Normal(s) = component {
            let s_str = s.to_string_lossy();
            if is_windows_apps {
                package_folder = s_str.to_string();
                break;
            }
            if s_str.eq_ignore_ascii_case("WindowsApps") {
                is_windows_apps = true;
            }
        }
    }
    
    if !package_folder.is_empty() {
        // package_folder format: PackageName_Version_Architecture__PublisherId
        // We want: PackageName_PublisherId!App
        if let Some(publisher_idx) = package_folder.rfind("__") {
            let publisher_id = &package_folder[publisher_idx + 2..];
            if let Some(first_underscore) = package_folder.find('_') {
                let package_name = &package_folder[..first_underscore];
                return Some(format!("shell:AppsFolder\\{}_{}!App", package_name, publisher_id));
            }
        }
    }
    None
}

#[derive(serde::Serialize)]
pub struct DockDrive {
    letter: String,
    label: String,
    removable: bool,
}

#[tauri::command]
pub fn get_drives() -> Vec<DockDrive> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    let mask = unsafe { GetLogicalDrives() };
    let mut drives = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 { continue; }
        let letter = (b'A' + i as u8) as char;
        let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain(std::iter::once(0)).collect();
        let kind = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
        if kind != DRIVE_FIXED && kind != DRIVE_REMOVABLE { continue; }
        let mut name = [0u16; 64];
        let label = unsafe {
            GetVolumeInformationW(PCWSTR(root.as_ptr()), Some(&mut name), None, None, None, None)
                .map(|_| String::from_utf16_lossy(&name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())]))
                .unwrap_or_default()
        };
        drives.push(DockDrive { letter: letter.to_string(), label, removable: kind == DRIVE_REMOVABLE });
    }
    drives
}

#[tauri::command]
pub fn open_dock_extra(target: String) -> Result<(), String> {
    let path = if target == "recycle" {
        "shell:RecycleBinFolder".to_string()
    } else if target.len() == 1 && target.as_bytes()[0].is_ascii_uppercase() {
        format!("{target}:\\")
    } else {
        return Err(format!("unknown dock extra: {target}"));
    };
    std::process::Command::new("explorer.exe").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn open_app(app: AppHandle, app_name: String, args: Option<String>) {
    if app_name == "start" {
        toggle_start_menu();
        return;
    }

    if app_name == "search" {
        let launcher = crate::utils::get_setting_str(&app, "fern-search-launcher");
        tauri::async_runtime::spawn_blocking(move || crate::launchers::trigger_search_launcher(app, launcher));
        return;
    }

    if app_name == "taskview" {
        tauri::async_runtime::spawn_blocking(|| send_win_chord(windows::Win32::UI::Input::KeyboardAndMouse::VK_TAB));
        return;
    }

    if app_name == "taskmgr" {
        tauri::async_runtime::spawn_blocking(|| {
            let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
            let exe = format!("{root}\\System32\\Taskmgr.exe");
            let wide_exe: Vec<u16> = exe.encode_utf16().chain(std::iter::once(0)).collect();
            let wide_open: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
            let res = unsafe {
                windows::Win32::UI::Shell::ShellExecuteW(
                    None,
                    windows::core::PCWSTR(wide_open.as_ptr()),
                    windows::core::PCWSTR(wide_exe.as_ptr()),
                    None,
                    None,
                    windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
                )
            };
            if res.0 as usize <= 32 {
                crate::diagnostics::log(&format!("task manager failed to start: error {}", res.0 as usize));
            }
        });
        return;
    }

    if app_name == "fern-settings" {
        crate::on_demand_windows::open(&app, crate::on_demand_windows::SETTINGS);
        return;
    }

    tauri::async_runtime::spawn_blocking(move || launch_path(&app_name, args.as_deref()));
}

/// Starts another instance of an app. Running PWAs go through their Start Menu
/// shortcut so the new window belongs to the web app.
#[tauri::command]
pub async fn launch_new_instance(app_path: String, app_name: Option<String>, args: Option<String>) {
    tauri::async_runtime::spawn_blocking(move || {
        if app_path == "start" {
            return;
        }
        let target = pwa_launch_target(&app_path, app_name.as_deref()).unwrap_or(app_path);
        launch_path(&target, args.as_deref());
    });
}

/// Finds the Start Menu shortcut for a running PWA (same title, same browser).
pub(super) fn pwa_launch_target(path: &str, name: Option<&str>) -> Option<String> {
    let name = name?.trim();
    if name.is_empty() || !is_browser_host_process(path) {
        return None;
    }
    let host_exe = std::path::Path::new(path).file_name()?.to_str()?.to_lowercase();
    let apps = INSTALLED_APPS_CACHE.get()?.lock().ok()?;
    apps.iter()
        .find(|app| is_pwa_shortcut_for(app, name, &host_exe))
        .map(|app| app.path.clone())
}

pub(super) fn is_pwa_shortcut_for(app: &AppInfo, name: &str, host_exe: &str) -> bool {
    app.name.eq_ignore_ascii_case(name)
        && app.executable.as_deref().is_some_and(|exe| exe.eq_ignore_ascii_case(host_exe))
}

fn pe_subsystem(path: &str) -> Option<u16> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos).ok()?;
    let pe_offset = u32::from_le_bytes(dos[60..64].try_into().ok()?) as u64;
    file.seek(SeekFrom::Start(pe_offset)).ok()?;
    let mut header = [0u8; 24 + 70];
    file.read_exact(&mut header).ok()?;
    if &header[..4] != b"PE\0\0" {
        return None;
    }
    Some(u16::from_le_bytes([header[24 + 68], header[24 + 69]]))
}

fn gui_launcher_for(path: &str) -> Option<String> {
    let exe = std::path::Path::new(path);
    if pe_subsystem(path)? != 3 {
        return None;
    }
    let stem = exe.file_stem()?.to_str()?;
    let twin = exe.parent()?.join(format!("run{}.exe", stem));
    let twin_str = twin.to_str()?;
    (twin.exists() && pe_subsystem(twin_str) == Some(2)).then(|| twin_str.to_string())
}

/// ShellExecuteW can't resolve some shell:AppsFolder ids; explorer.exe can.
fn open_with_explorer(target: &str) {
    use std::os::windows::process::CommandExt;
    let _ = std::process::Command::new("explorer.exe")
        .arg(target)
        .creation_flags(0x0800_0000)
        .spawn();
}

/// Opens a path, shortcut or shell app id through the shell.
fn launch_path(path: &str, args: Option<&str>) {
    // ShellExecuteW and IShellFolder (find_aumid_by_family) require STA COM on worker threads.
    let _com = crate::utils::ComGuard::sta();
    unsafe {
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide_open: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();

        // Shortcuts and shell app ids go through the shell so args and package identity survive.
        let shell_target = if path.to_lowercase().ends_with(".lnk") {
            Some(path.to_string())
        } else if is_aumid_path(path) {
            let target = shell_parsing_name_for_aumid(path);
            (!target.is_empty()).then_some(target)
        } else {
            None
        };

        if let Some(target) = shell_target {
            let wide_target: Vec<u16> = target.encode_utf16().chain(std::iter::once(0)).collect();
            let res = ShellExecuteW(
                None,
                windows::core::PCWSTR(wide_open.as_ptr()),
                windows::core::PCWSTR(wide_target.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            );
            if res.0 as usize <= 32 {
                let msg = format!("launch_path: failed to open {} (error {})", target, res.0 as usize);
                eprintln!("{msg}");
                crate::diagnostics::log(&msg);
                if target.starts_with("shell:AppsFolder\\") {
                    open_with_explorer(&target);
                }
            }
            return;
        }

        if let Some(derived_cmd) = get_uwp_launch_cmd(path) {
            let exact = package_family_from_windows_apps_path(path)
                .and_then(|family| find_aumid_by_family(&family))
                .map(|id| format!("shell:AppsFolder\\{}", id.trim_start_matches("shell:AppsFolder\\")));
            let uwp_cmd = exact.clone().unwrap_or(derived_cmd);
            let wide_cmd: Vec<u16> = uwp_cmd.encode_utf16().chain(std::iter::once(0)).collect();
            
            let res = ShellExecuteW(
                None,
                windows::core::PCWSTR(wide_open.as_ptr()),
                windows::core::PCWSTR(wide_cmd.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            );
            
            if res.0 as usize <= 32 {
                let msg = format!("launch_path: failed to open UWP app {} (error {})", uwp_cmd, res.0 as usize);
                eprintln!("{msg}");
                crate::diagnostics::log(&msg);
                if uwp_cmd.starts_with("shell:AppsFolder\\") {
                    open_with_explorer(&uwp_cmd);
                }
            }
            return;
        }

        use std::path::Path;
        let mut final_path = path.to_string();

        if let Some(moved) = crate::utils::moved_squirrel_exe(&final_path) {
            final_path = moved;
        }

        if let Some(twin) = gui_launcher_for(&final_path) {
            final_path = twin;
        }

        let wide_path: Vec<u16> = final_path.encode_utf16().chain(std::iter::once(0)).collect();
        let wide_open: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
        let wide_args: Option<Vec<u16>> = args
            .filter(|a| !a.trim().is_empty())
            .map(|a| a.encode_utf16().chain(std::iter::once(0)).collect());
        let wide_dir: Option<Vec<u16>> = Path::new(&final_path)
            .parent()
            .filter(|d| d.is_dir())
            .map(|d| d.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect());

        let res = ShellExecuteW(
            None,
            windows::core::PCWSTR(wide_open.as_ptr()),
            windows::core::PCWSTR(wide_path.as_ptr()),
            wide_args.as_ref().map_or(windows::core::PCWSTR::null(), |a| windows::core::PCWSTR(a.as_ptr())),
            wide_dir.as_ref().map_or(windows::core::PCWSTR::null(), |d| windows::core::PCWSTR(d.as_ptr())),
            SW_SHOWNORMAL,
        );
        
        if res.0 as usize <= 32 {
            let msg = format!("launch_path: failed to open app {} (error {})", final_path, res.0 as usize);
            eprintln!("{msg}");
            crate::diagnostics::log(&msg);
        }
    }
}

#[tauri::command]
pub fn save_pinned_apps(app: AppHandle, apps: Vec<AppInfo>) -> Result<(), String> {
    let path = app.path().app_config_dir().map_err(|e| e.to_string())?.join("pinned_apps.json");
    if let Some(parent) = path.parent() { let _ = std::fs::create_dir_all(parent); }
    let content = serde_json::to_string(&apps).map_err(|e| e.to_string())?;
    std::fs::write(path, content).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn load_pinned_apps(app: AppHandle) -> Vec<AppInfo> {
    let cfg_dir = app.path().app_config_dir().unwrap_or_default();
    let path = cfg_dir.join("pinned_apps.json");
    let synced_marker = cfg_dir.join("taskbar_pins_synced");

    if !synced_marker.exists() {
        let imported = tauri::async_runtime::spawn_blocking(import_taskbar_pins).await.ok().flatten();
        if let Some(imported) = imported {
            if !imported.is_empty() {
                let _ = save_pinned_apps(app.clone(), imported.clone());
                let _ = std::fs::write(&synced_marker, b"1");
                return imported;
            }
        }
    }

    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(apps) = serde_json::from_str(&content) { return apps; }
    }

    vec![
        AppInfo { name: crate::taskbar_pins::shell_app_name("Microsoft.Windows.Explorer", "File Explorer"), path: "C:\\Windows\\explorer.exe".into(), icon: None, is_running: false, hwnd: None, executable: Some("explorer.exe".into()), all_hwnds: None, args: None },
        AppInfo { name: "Microsoft Edge".into(), path: "msedge.exe".into(), icon: None, is_running: false, hwnd: None, executable: Some("msedge.exe".into()), all_hwnds: None, args: None },
        AppInfo { name: crate::taskbar_pins::shell_app_name("Microsoft.WindowsNotepad_8wekyb3d8bbwe!App", "Notepad"), path: "notepad.exe".into(), icon: None, is_running: false, hwnd: None, executable: Some("notepad.exe".into()), all_hwnds: None, args: None },
        AppInfo { name: "Settings".into(), path: "fern-settings".into(), icon: None, is_running: false, hwnd: None, executable: None, all_hwnds: None, args: None },
    ]
}

fn import_taskbar_pins() -> Option<Vec<AppInfo>> {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
    unsafe {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let pins = crate::taskbar_pins::read_pins();
        if com { CoUninitialize(); }
        pins
    }
}

#[tauri::command]
pub async fn get_installed_apps() -> Vec<AppInfo> {
    let cache = INSTALLED_APPS_CACHE.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let is_empty = if let Ok(lock) = cache.lock() { lock.is_empty() } else { true };
    if is_empty && !IS_SCANNING.load(Ordering::Relaxed) { crate::services::trigger_app_scan(); }
    let start = std::time::Instant::now();
    while IS_SCANNING.load(Ordering::Relaxed) && start.elapsed() < std::time::Duration::from_secs(5) { tokio::time::sleep(std::time::Duration::from_millis(100)).await; }
    if let Ok(cache_lock) = cache.lock() { cache_lock.clone() } else { Vec::new() }
}

pub fn format_picked_program_name(stem: Option<&str>) -> String {
    let raw = stem.filter(|s| !s.trim().is_empty()).unwrap_or("App");
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) if first.is_lowercase() => first.to_uppercase().chain(chars).collect(),
        _ => raw.to_string(),
    }
}

#[tauri::command]
pub async fn pick_program(app: AppHandle) -> Result<Option<(String, String)>, String> {
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Choose a program")
        .add_filter("Programs", &["exe", "lnk", "bat", "cmd", "url"]);
    if let Some(dock) = app.get_webview_window("dock") {
        dialog = dialog.set_parent(&dock);
    }
    let Some(path) = dialog.blocking_pick_file() else {
        return Ok(None);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    let name = format_picked_program_name(path.file_stem().and_then(|s| s.to_str()));
    Ok(Some((name, path.to_string_lossy().into_owned())))
}
