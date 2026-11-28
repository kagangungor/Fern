use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM, CloseHandle};
use windows::core::BOOL;
use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible, GetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW};
use windows::Win32::Graphics::Dwm::DwmGetWindowAttribute;
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_NAME_WIN32};
use std::path::Path;
use crate::types::*;
use crate::state::*;
use crate::utils::*;
use super::mouse_hit::{MH_LAST_DOCK_IGNORE, MH_LAST_MAIN_IGNORE, MH_LAST_OV_IGNORE};
use super::app_scan::{RESOLVED_TARGET_NAMES, normalize_exe_path};
use super::appbar::{position_notch_window, register_appbar, register_dock_appbar};

pub fn sync_overlays(app: &AppHandle) {
    // Don't reposition while splash is playing
    if crate::state::OVERLAY_IN_SPLASH.load(Ordering::Relaxed) {
        return;
    }
    // Full-screen overlay — notches render at left/right edges via CSS
    // The window covers the entire primary monitor so it never needs repositioning
    if let Some(ov_win) = app.get_webview_window("overlay") {
        let hwnd_res = ov_win.hwnd();
        if let Ok(Some(monitor)) = ov_win.primary_monitor() {
            let size = monitor.size();
            let pos = monitor.position();
            let already_positioned = hwnd_res.as_ref().ok().is_some_and(|&hwnd| unsafe {
                use windows::Win32::Foundation::RECT;
                use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
                let mut current_rect = RECT::default();
                GetWindowRect(hwnd, &mut current_rect).is_ok()
                    && current_rect.left == pos.x
                    && current_rect.top == pos.y
                    && (current_rect.right - current_rect.left) == size.width as i32
                    && (current_rect.bottom - current_rect.top) == size.height as i32
            });
            if !already_positioned {
                let _ = ov_win.set_position(tauri::PhysicalPosition::new(pos.x, pos.y));
                let _ = ov_win.set_size(tauri::PhysicalSize::new(size.width, size.height));
            }
        }
        if !crate::direct_flip::is_hidden("overlay") {
            if let Ok(hwnd) = hwnd_res { re_assert_topmost(hwnd); }
        }
    }
}

/// True for a property sheet dialog (has a tab control). Works in any locale, unlike matching the title.
unsafe fn dialog_has_tab_control(hwnd: HWND) -> bool {
    unsafe extern "system" fn child_proc(child: HWND, lparam: LPARAM) -> BOOL {
        let found = &mut *(lparam.0 as *mut bool);
        let mut class_name = [0u8; 64];
        let len = windows::Win32::UI::WindowsAndMessaging::GetClassNameA(child, &mut class_name);
        if std::str::from_utf8(&class_name[..len as usize]).unwrap_or("") == "SysTabControl32" {
            *found = true;
            return BOOL(0);
        }
        BOOL(1)
    }

    let mut found = false;
    let _ = windows::Win32::UI::WindowsAndMessaging::EnumChildWindows(
        Some(hwnd),
        Some(child_proc),
        LPARAM(&mut found as *mut bool as isize),
    );
    found
}

// "Fern" is an ordinary word, so a window is only ours if our process owns it. Other apps' windows
// with the same title, or a program of the same name, stay in the dock.
fn hides_own_window(window_pid: u32, own_pid: u32, title: &str) -> bool {
    if window_pid != own_pid {
        return false;
    }
    let lower = title.to_lowercase();
    let is_settings = lower.contains("settings") || lower.contains("ayarlar") || title == "Settings";
    !is_settings
}

fn should_include_window_title(title_len: i32, window_class: &str, console_windows_enabled: bool) -> bool {
    title_len > 0 || (window_class == "ConsoleWindowClass" && console_windows_enabled)
}

pub unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let apps = &mut *(lparam.0 as *mut Vec<AppInfo>);

    if IsWindowVisible(hwnd).as_bool() {
        // DWM-cloaked windows aren't on screen (suspended UWP apps, other virtual desktops)
        // but IsWindowVisible is still true, so check cloaking.
        {
            let mut cloaked = 0u32;
            let size = std::mem::size_of::<u32>() as u32;
            if windows::Win32::Graphics::Dwm::DwmGetWindowAttribute(
                hwnd,
                windows::Win32::Graphics::Dwm::DWMWA_CLOAKED,
                &mut cloaked as *mut _ as *mut _,
                size,
            ).is_ok() && cloaked != 0 {
                return true.into();
            }
        }

        let mut text = [0u16; 512];
        let len = windows::Win32::UI::WindowsAndMessaging::GetWindowTextW(hwnd, &mut text);
        let mut class_name = [0u8; 256];
        let class_len = windows::Win32::UI::WindowsAndMessaging::GetClassNameA(hwnd, &mut class_name);
        let window_class = std::str::from_utf8(&class_name[..class_len.max(0) as usize]).unwrap_or("");
        let console_windows_enabled = crate::utils::setting_flag("fern-dock-console-windows");
        if should_include_window_title(len, window_class, console_windows_enabled) {
            let title = if len > 0 { String::from_utf16_lossy(&text[..len as usize]) } else { String::new() };
            
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
            
            // Basic filter for top-level app windows
            // We include windows without captions if they don't have the ToolWindow style,
            // as many games and modern apps (like Spotify/Valorant) lack WS_CAPTION.
            if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
                return true.into();
            }

            // Filter out system containers and background stuff
            if title == "Program Manager" {
                return true.into();
            }

            let mut process_id = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut process_id));
            
            if let Ok(process_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) {
                let mut path_buf = [0u16; 1024];
                let mut path_len = path_buf.len() as u32;
                if QueryFullProcessImageNameW(process_handle, PROCESS_NAME_WIN32, windows::core::PWSTR(path_buf.as_mut_ptr()), &mut path_len).is_ok() {
                    let path = String::from_utf16_lossy(&path_buf[..path_len as usize]);
                    let lowercase_path = path.to_lowercase();
                    let is_console_window = window_class == "ConsoleWindowClass";

                    // Explorer folder windows and property sheets (#32770 with a tab control) both count,
                    // so Properties shows under File Explorer.
                    let is_explorer_window = window_class == "CabinetWClass"
                        || window_class == "ExploreWClass"
                        || (window_class == "#32770" && dialog_has_tab_control(hwnd));

                    let has_visible_owner = windows::Win32::UI::WindowsAndMessaging::GetWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::GW_OWNER)
                        .map(|owner| !owner.0.is_null() && IsWindowVisible(owner).as_bool())
                        .unwrap_or(false);
                    if has_visible_owner && !is_explorer_window && !is_console_window {
                        let _ = CloseHandle(process_handle);
                        return true.into();
                    }

                    if hides_own_window(process_id, std::process::id(), &title) ||
                       (lowercase_path.contains("conhost.exe")
                           && !(is_console_window && console_windows_enabled)) ||
                       (lowercase_path.contains("explorer.exe") && !is_explorer_window) ||
                       lowercase_path.contains("shellexperiencehost.exe") ||
                       lowercase_path.contains("searchhost.exe") ||
                       lowercase_path.contains("textinputhost.exe") ||
                       lowercase_path.ends_with("gamingservicesui.exe") ||
                       (lowercase_path.contains("applicationframehost.exe") && window_class != "ApplicationFrameWindow") {
                        let _ = CloseHandle(process_handle);
                        return true.into();
                    }

                    let name = Path::new(&path).file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(&title)
                        .replace(".exe", "");

                    let exe_name = Path::new(&path).file_name()
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string());

                    // A UWP app's real window is the ApplicationFrameWindow from ApplicationFrameHost, with the
                    // title and package id. Skip the inner CoreWindow so there's no duplicate item.
                    let is_uwp_core_window = window_class == "Windows.UI.Core.CoreWindow";
                    let is_uwp_frame = window_class == "ApplicationFrameWindow";

                    let is_browser_host = lowercase_path.contains("msedge.exe")
                        || lowercase_path.contains("chrome.exe")
                        || lowercase_path.contains("brave.exe");

                    // A browser window and a PWA in the same browser can share a title. The AUMID tells them apart:
                    // PWAs carry a web app id or package id, plain browser windows just the browser.
                    // No id falls back to the title.
                    let window_aumid = if is_uwp_frame
                        || lowercase_path.contains("\\windowsapps\\")
                        || is_browser_host
                    {
                        crate::utils::get_window_app_user_model_id(hwnd)
                    } else {
                        None
                    };
                    let is_browser_pwa = is_browser_host
                        && window_aumid.as_deref().map_or(false, crate::commands::is_browser_pwa_aumid);

                    let final_name = if name.eq_ignore_ascii_case("conhost") {
                        if !title.trim().is_empty() { title.clone() } else { "Console".to_string() }
                    } else if (name.eq_ignore_ascii_case("javaw") || name.eq_ignore_ascii_case("java")) && !title.is_empty() {
                        title.clone()
                    } else if ((is_browser_host && (is_browser_pwa || window_aumid.is_none()))
                        || name == "ApplicationFrameHost"
                        || name == "SystemSettings")
                        && !title.is_empty()
                    {
                        // Extract a cleaner name from the window title for host processes (PWAs, UWP apps)
                        title.split(" - ").next().map(|s| s.trim()).unwrap_or(&title).to_string()
                    } else if name.eq_ignore_ascii_case("explorer") {
                        crate::taskbar_pins::shell_app_name("Microsoft.Windows.Explorer", "File Explorer")
                    } else {
                        let from_start_menu = INSTALLED_APPS_CACHE
                            .get()
                            .and_then(|c| c.lock().ok())
                            .and_then(|apps| {
                                apps.iter().find(|a| a.path.eq_ignore_ascii_case(&path)).map(|a| a.name.clone())
                            })
                            .or_else(|| {
                                let norm = normalize_exe_path(&path);
                                RESOLVED_TARGET_NAMES
                                    .get()
                                    .and_then(|m| m.read().ok())
                                    .and_then(|m| m.get(&norm).cloned())
                            });
                        from_start_menu
                            .or_else(|| crate::utils::exe_description(&path))
                            .unwrap_or_else(|| name.clone())
                    };

                    // Only avoid adding the exact same window handle (HWND) multiple times
                    let already_exists = apps.iter().any(|a| a.hwnd == Some(hwnd.0 as isize));

                    if !already_exists && !is_uwp_core_window {
                        // Packaged apps expose their package id on the window. Using it as the path makes pins
                        // match running windows and gives the icon code an exact AUMID.
                        let path = if is_uwp_frame
                            || lowercase_path.contains("\\windowsapps\\")
                            || is_browser_host
                        {
                            match window_aumid {
                                Some(aumid) if aumid.contains('!') => aumid,
                                _ => path,
                            }
                        } else {
                            path
                        };
                        let is_control_panel = window_class == "CabinetWClass"
                            && lowercase_path.ends_with("\\explorer.exe")
                            && crate::utils::get_window_app_user_model_id(hwnd)
                                .map_or(false, |id| id.eq_ignore_ascii_case("Microsoft.Windows.ControlPanel"));
                        let (final_name, path, exe_name) = if process_id == std::process::id() {
                            ("Settings".to_string(), "fern-settings".to_string(), None)
                        } else if is_control_panel {
                            let control = format!(r"{}\System32\control.exe", std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into()));
                            (crate::taskbar_pins::shell_app_name("Microsoft.Windows.ControlPanel", "Control Panel"), control, Some("control.exe".to_string()))
                        } else {
                            (final_name, path, exe_name)
                        };
                        let window_title = if title.trim().is_empty() { final_name.clone() } else { title.clone() };
                        apps.push(AppInfo {
                            name: final_name,
                            path,
                            icon: None,
                            is_running: true,
                            hwnd: Some(hwnd.0 as isize),
                            executable: exe_name,
                            all_hwnds: Some(vec![(hwnd.0 as isize, window_title)]),
                            args: None,
                        });
                    }
                }
                let _ = CloseHandle(process_handle);
            } else if process_exe_name(process_id).is_some_and(|exe| exe.eq_ignore_ascii_case("consent.exe")) {
                // Parked UAC prompt (consent.exe runs as SYSTEM so OpenProcess fails).
                if !apps.iter().any(|a| a.hwnd == Some(hwnd.0 as isize)) {
                    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
                    let window_title = if title.trim().is_empty() { "User Account Control".to_string() } else { title.clone() };
                    apps.push(AppInfo {
                        name: window_title.clone(),
                        path: format!(r"{system_root}\System32\consent.exe"),
                        icon: None,
                        is_running: true,
                        hwnd: Some(hwnd.0 as isize),
                        executable: Some("consent.exe".into()),
                        all_hwnds: Some(vec![(hwnd.0 as isize, window_title)]),
                        args: None,
                    });
                }
            }
        }
    }
    true.into()
}

/// Returns the executable file name of a process (including elevated SYSTEM processes
/// that cannot be opened with `OpenProcess`) via `CreateToolhelp32Snapshot`.
unsafe fn process_exe_name(process_id: u32) -> Option<String> {
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = None;
    if Process32FirstW(snapshot, &mut entry).is_ok() {
        loop {
            if entry.th32ProcessID == process_id {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                found = Some(String::from_utf16_lossy(&entry.szExeFile[..len]));
                break;
            }
            if Process32NextW(snapshot, &mut entry).is_err() {
                break;
            }
        }
    }
    let _ = CloseHandle(snapshot);
    found
}

pub fn wire_window_events(app: &AppHandle, window: &tauri::WebviewWindow, kind: crate::types::WindowKind, is_dynamic: bool) {
    let label = window.label().to_string();

    if let Ok(hwnd) = window.hwnd() {
        crate::active_look::prevent_minimize(hwnd);
    }

    let win_for_rect = window.clone();
    let label_for_rect = label.clone();
    let update_rect = move || {
        if let (Ok(p), Ok(s)) = (win_for_rect.outer_position(), win_for_rect.outer_size()) {
            let map = match kind {
                crate::types::WindowKind::Dock => crate::state::dock_window_rects(),
                crate::types::WindowKind::Notch => crate::state::main_window_rects(),
            };
            if let Ok(mut m) = map.lock() {
                m.insert(label_for_rect.clone(), (p, s));
            }
        }
    };
    update_rect();

    let u = update_rect.clone();
    let win_for_events = window.clone();
    let app_for_events = app.clone();
    let label_for_events = label.clone();
    window.on_window_event(move |e| {
        match e {
            tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                u();
                sync_overlays(&app_for_events);
            }
            tauri::WindowEvent::ScaleFactorChanged { .. } => {
                u();
                match kind {
                    crate::types::WindowKind::Notch => {
                        let w = win_for_events.clone();
                        let h = app_for_events.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                            register_appbar(w);
                            sync_overlays(&h);
                        });
                    }
                    crate::types::WindowKind::Dock => {
                        let h = app_for_events.clone();
                        if crate::state::get_flag(crate::state::dock_appbar_registered(), &label_for_events) {
                            let w = win_for_events.clone();
                            tauri::async_runtime::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                register_dock_appbar(w);
                                sync_overlays(&h);
                            });
                        } else {
                            sync_overlays(&h);
                        }
                    }
                }
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                if is_dynamic {
                    let _ = win_for_events.hide();
                } else {
                    crate::commands::restore_taskbar_and_exit(&app_for_events);
                }
            }
            _ => {}
        }
    });
}

pub fn create_monitor_window(app: &AppHandle, kind: crate::types::WindowKind, label: &str, monitor: &tauri::Monitor) {
    let app = app.clone();
    let label = label.to_string();
    let monitor = monitor.clone();
    let base_label = match kind {
        crate::types::WindowKind::Dock => "dock",
        crate::types::WindowKind::Notch => "main",
    };

    tauri::async_runtime::spawn(async move {
        let base_config = match app.config().app.windows.iter().find(|c| c.label == base_label) {
            Some(c) => c.clone(),
            None => return,
        };

        let mut conf = base_config;
        conf.label = label.clone();

        let pos = monitor.position();
        let scale = monitor.scale_factor();
        conf.x = Some(pos.x as f64 / scale);
        conf.y = Some(pos.y as f64 / scale);
        conf.visible = false;

        let builder = match tauri::WebviewWindowBuilder::from_config(&app, &conf) {
            Ok(b) => b,
            Err(_) => return,
        };
        let window = match builder.build() {
            Ok(w) => w,
            Err(_) => return,
        };

        let _ = window.set_ignore_cursor_events(true);
        wire_window_events(&app, &window, kind, true);
        watch_webview_process(&app, &window);

        // Extra monitors wait for the splash
        if !crate::state::SPLASH_FINISHED.load(Ordering::Relaxed) {
            crate::commands::wait_for_splash().await;
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }

        match kind {
            crate::types::WindowKind::Dock => {
                let mode = crate::utils::get_setting_str(&app, "fern-dock-mode").unwrap_or_else(|| "smart".to_string());
                crate::commands::init_dock_window(&app, window, &mode).await;
            }
            crate::types::WindowKind::Notch => {
                let mode = crate::utils::get_setting_str(&app, "fern-notch-mode").unwrap_or_else(|| "fixed".to_string());
                crate::commands::init_notch_window(&app, window, &mode).await;
            }
        }
    });
}

fn reposition_all_windows(app_handle: &AppHandle) {
    if SHUTTING_DOWN.load(Ordering::Relaxed) {
        return;
    }
    crate::monitors::sync_monitor_windows(app_handle);

    for (label, win) in app_handle.webview_windows() {
        if crate::state::is_notch_label(&label) {
            if crate::state::get_flag(crate::state::main_appbar_registered(), &label) {
                register_appbar(win);
            } else if crate::utils::setting_flag("fern-notch-respect-workarea") {
                position_notch_window(&win);
            }
        } else if crate::state::is_dock_label(&label) {
            // Only reposition the dock if it's enabled in settings.
            // Without this guard, power events (plug/unplug, wake) would re-show
            // a dock that the user had previously disabled.
            let dock_enabled = get_setting_str(app_handle, "fern-dock-enabled")
                .unwrap_or_else(|| "true".to_string());
            let waiting_for_splash = !crate::state::SPLASH_FINISHED.load(Ordering::Relaxed);
            if dock_enabled == "true" && !waiting_for_splash {
                if crate::state::get_flag(crate::state::dock_appbar_registered(), &label) {
                    register_dock_appbar(win);
                } else {
                    reposition_autohide_dock(app_handle, win);
                }
            }
        }
    }
    if NATIVE_TASKBAR_HIDDEN.load(Ordering::Relaxed) {
        set_taskbar_visibility(false, false);
    }
    sync_overlays(app_handle);
    // Re-assert topmost on all windows after repositioning to recover from any z-order loss
    for (label, win) in app_handle.webview_windows() {
        if crate::state::is_notch_label(&label) || crate::state::is_dock_label(&label) {
            if let Ok(hwnd) = win.hwnd() { re_assert_topmost(hwnd); }
        }
    }
}

fn reposition_autohide_dock(app_handle: &AppHandle, dock_win: tauri::WebviewWindow) {
    let dock_clone = dock_win.clone();
    let ah = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        for _attempt in 0..5 {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let hwnd_val = match dock_clone.hwnd() {
                Ok(h) => h.0 as isize,
                Err(_) => continue,
            };
            let ph = match dock_clone.outer_size() {
                Ok(s) => s.height as i32,
                Err(_) => continue,
            };
            if ph <= 10 {
                continue;
            }
            let monitor_info = crate::monitors::monitor_for_window_label(dock_clone.app_handle(), crate::types::WindowKind::Dock, dock_clone.label()).map(|m| {
                let s = m.size();
                let p = m.position();
                (s.width as i32, s.height as i32, p.x, p.y)
            });
            if let Some((m_w, m_h, m_x, m_y)) = monitor_info {
                let final_y = m_y + m_h - ph;
                unsafe {
                    use windows::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOZORDER, SWP_NOACTIVATE, SWP_FRAMECHANGED};
                    use windows::Win32::Foundation::HWND;
                    let _ = SetWindowPos(
                        HWND(hwnd_val as *mut _), None,
                        m_x, final_y, m_w, ph,
                        SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    );
                }
                if let Ok(hwnd) = dock_clone.hwnd() { re_assert_topmost(hwnd); }
                let _ = dock_clone.show();
                break;
            }
        }
        if NATIVE_TASKBAR_HIDDEN.load(Ordering::Relaxed) {
            set_taskbar_visibility(false, false);
        }
        sync_overlays(&ah);
    });
}

pub fn setup_display_change_monitor(app_handle: AppHandle) {
    let _ = crate::state::DISPLAY_MONITOR_HANDLE.set(app_handle);

    std::thread::spawn(|| {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::*;
            use windows::Win32::System::LibraryLoader::GetModuleHandleW;

            let class_name = windows::core::PCSTR(c"FernDisplayMonitor".as_ptr() as *const u8);
            let h_inst = GetModuleHandleW(None).unwrap_or_default().into();

            let wnd_class = WNDCLASSEXA {
                cbSize: std::mem::size_of::<WNDCLASSEXA>() as u32,
                lpfnWndProc: Some(display_monitor_proc),
                hInstance: h_inst,
                lpszClassName: class_name,
                ..Default::default()
            };

            if RegisterClassExA(&wnd_class) == 0 {
                return;
            }

            let hwnd = CreateWindowExA(
                WINDOW_EX_STYLE(WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0),
                class_name,
                windows::core::PCSTR::null(),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                Some(h_inst),
                None,
            );

            let hwnd = match hwnd {
                Ok(h) => h,
                Err(_) => return,
            };

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, Some(hwnd), 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
}

unsafe extern "system" fn display_monitor_proc(
    hwnd: HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::*;
    use windows::Win32::Foundation::LRESULT;

    const WM_DISPLAYCHANGE: u32 = 0x007E;
    const WM_POWERBROADCAST: u32 = 0x0218;
    const PBT_APMPOWERSTATUSCHANGE: u32 = 0x000A;
    const PBT_APMRESUMESUSPEND: u32 = 0x0007;
    const WM_DWMCOLORIZATIONCOLORCHANGED: u32 = 0x0320;
    const WM_SETTINGCHANGE: u32 = 0x001A;

    match msg {
        WM_DISPLAYCHANGE => {
            let now = get_now_ms();
            let last = LAST_DISPLAY_CHANGE_MS.load(Ordering::Relaxed);
            if now - last > 500 {
                LAST_DISPLAY_CHANGE_MS.store(now, Ordering::Relaxed);
                if let Some(app_handle) = DISPLAY_MONITOR_HANDLE.get() {
                    let ah = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        reposition_all_windows(&ah);
                    });
                }
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            let pw = wparam.0 as u32;
            if pw == PBT_APMPOWERSTATUSCHANGE || pw == PBT_APMRESUMESUSPEND {
                let now = get_now_ms();
                let last = LAST_DISPLAY_CHANGE_MS.load(Ordering::Relaxed);
                if now - last > 1000 {
                    LAST_DISPLAY_CHANGE_MS.store(now, Ordering::Relaxed);
                    if let Some(app_handle) = DISPLAY_MONITOR_HANDLE.get() {
                        let ah = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            // Longer delay for wake from sleep to allow display to fully initialize
                            let delay = if pw == PBT_APMRESUMESUSPEND { 500 } else { 300 };
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            reposition_all_windows(&ah);
                        });
                    }
                }
            }
            LRESULT(0)
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => {
            if !crate::windows_accent::is_recent_self_broadcast() {
                emit_system_accent_change();
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_SETTINGCHANGE => {
            const SPI_SETWORKAREA: usize = 0x002F;
            static LAST_WORKAREA_CHANGE_MS: AtomicI64 = AtomicI64::new(0);
            if wparam.0 == SPI_SETWORKAREA && crate::utils::setting_flag("fern-notch-respect-workarea") {
                let now = get_now_ms();
                let last = LAST_WORKAREA_CHANGE_MS.load(Ordering::Relaxed);
                if now - last > 300 {
                    LAST_WORKAREA_CHANGE_MS.store(now, Ordering::Relaxed);
                    if let Some(app_handle) = DISPLAY_MONITOR_HANDLE.get() {
                        let ah = app_handle.clone();
                        tauri::async_runtime::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                            reposition_all_windows(&ah);
                        });
                    }
                }
            }
            let area = if lparam.0 != 0 {
                unsafe { windows::core::PCWSTR(lparam.0 as *const u16).to_string() }.unwrap_or_default()
            } else {
                String::new()
            };
            if should_emit_setting_change(wparam.0, &area, crate::windows_accent::is_recent_self_broadcast()) {
                emit_system_accent_change();
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn should_emit_setting_change(wparam: usize, area: &str, recent_self_broadcast: bool) -> bool {
    const SPI_SETDESKWALLPAPER: usize = 0x0014;
    if wparam == SPI_SETDESKWALLPAPER {
        return true;
    }
    match area {
        "ImmersiveColorSet" => !recent_self_broadcast,
        "WindowsThemeElement" | "Wallpaper" => true,
        _ => false,
    }
}

fn emit_system_accent_change() {
    if let Some(app_handle) = DISPLAY_MONITOR_HANDLE.get() {
        let color_hex = if let Some(c) = crate::commands::get_windows_accent_color() {
            c
        } else {
            let mut color = 0u32;
            let mut opaque = windows::core::BOOL(0);
            if unsafe { windows::Win32::Graphics::Dwm::DwmGetColorizationColor(&mut color, &mut opaque).is_ok() } {
                let r = ((color >> 16) & 0xff) as u8;
                let g = ((color >> 8) & 0xff) as u8;
                let b = (color & 0xff) as u8;
                format!("#{:02x}{:02x}{:02x}", r, g, b)
            } else {
                "#0078d4".to_string()
            }
        };
        let _ = app_handle.emit("system-accent-changed", color_hex);
    }
}

pub fn start_window_style_guard(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(crate::memory_saver::poll_interval(std::time::Duration::from_millis(300)));
        for (label, win) in app.webview_windows() {
            let is_notch = is_notch_label(&label);
            let is_dock = is_dock_label(&label);
            if !(is_notch || is_dock || label == "overlay") { continue; }
            let Ok(hwnd) = win.hwnd() else { continue };
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{
                    GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
                };
                let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as usize;
                let keyboard = crate::state::KEYBOARD_FOCUS_LABEL.lock().ok().is_some_and(|l| l.as_deref() == Some(label.as_str()));
                let want = if keyboard { WS_EX_TOOLWINDOW.0 as usize } else { (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) as usize };
                if ex & want != want {
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (ex | want) as isize);
                }

                let extra = crate::state::monitor_suffix(&label).is_some();
                let last = if label == "overlay" {
                    MH_LAST_OV_IGNORE.load(Ordering::Relaxed)
                } else if is_notch && !extra {
                    MH_LAST_MAIN_IGNORE.load(Ordering::Relaxed)
                } else if is_dock && !extra {
                    MH_LAST_DOCK_IGNORE.load(Ordering::Relaxed)
                } else if is_notch {
                    crate::state::get_i32(crate::state::notch_extra_last_ignore(), &label, -1)
                } else {
                    crate::state::get_i32(crate::state::dock_extra_last_ignore(), &label, -1)
                };
                // tao rewrites the extended style on its own flag changes. Restore what the hit test
                // last decided (see set_click_through), the cursor may not move again to trigger it.
                if last != -1 {
                    crate::utils::set_click_through(hwnd, last == 1);
                } else if ex & WS_EX_LAYERED.0 as usize == 0 {
                    crate::utils::set_click_through(hwnd, ex & WS_EX_TRANSPARENT.0 as usize != 0);
                }
            }
        }
    });
}

static WEBVIEWS_WATCHED_AT: OnceLock<Instant> = OnceLock::new();
static RECOVERING_FROM_WEBVIEW_FAILURE: AtomicBool = AtomicBool::new(false);

/// If a webview's browser or renderer process dies, the windows stay up with dead content and keep
/// swallowing clicks. Relaunch when that happens.
pub fn watch_webview_processes(app: &AppHandle) {
    let _ = WEBVIEWS_WATCHED_AT.set(Instant::now());
    for (_, window) in app.webview_windows() {
        watch_webview_process(app, &window);
    }
}

/// Also called for each on-demand window, those don't exist yet when the app starts.
pub fn watch_webview_process(app: &AppHandle, window: &tauri::WebviewWindow) {
    let handle = app.clone();
    let _ = window.with_webview(move |webview| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_PROCESS_FAILED_KIND,
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED,
        };
        use webview2_com::ProcessFailedEventHandler;

        let Ok(core) = webview.controller().CoreWebView2() else {
            return;
        };
        let mut token = 0i64;
        let _ = core.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                args.ProcessFailedKind(&mut kind)?;
                crate::diagnostics::log(&format!("webview process failed: kind={}", kind.0));
                if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
                    || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                {
                    recover_from_webview_failure(&handle);
                }
                Ok(())
            })),
            &mut token,
        );
    });
}

fn recover_from_webview_failure(handle: &AppHandle) {
    // Quitting destroys the webviews too, and every window reports the same exit, so only act once.
    if SHUTTING_DOWN.load(Ordering::Relaxed)
        || RECOVERING_FROM_WEBVIEW_FAILURE.swap(true, Ordering::SeqCst)
    {
        return;
    }
    let started = WEBVIEWS_WATCHED_AT.get().map(Instant::elapsed);
    let handle = handle.clone();
    // Get out of the WebView2 callback before touching windows.
    tauri::async_runtime::spawn(async move {
        // A webview dying right after launch would die again after a relaunch, so give the taskbar back and exit.
        if started.is_some_and(|elapsed| elapsed < Duration::from_secs(30)) {
            crate::commands::restore_taskbar_and_exit(&handle);
        } else {
            crate::diagnostics::log("webview process failed, relaunching");
            // Give the leftover webview processes a moment to let go of the profile, relaunching
            // straight away left the new webviews without their images.
            SHUTTING_DOWN.store(true, Ordering::Relaxed);
            tokio::time::sleep(Duration::from_millis(1500)).await;
            crate::commands::relaunch_fern(&handle);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_our_own_process_is_hidden_from_the_dock() {
        assert!(hides_own_window(10, 10, "fern"));
        assert!(hides_own_window(10, 10, "Fern Dock"));
        assert!(!hides_own_window(10, 10, "Settings"), "our Settings window shows in the dock");
        assert!(!hides_own_window(10, 10, "Fern Settings"), "Fern Settings shows in the dock");
        assert!(!hides_own_window(10, 10, "Ayarlar"), "Ayarlar shows in the dock");
        assert!(!hides_own_window(20, 10, "fern"), "another app's window with that title stays");
        assert!(!hides_own_window(20, 10, "Fern Dock"));
    }

    #[test]
    fn setting_change_filter_suppresses_tray_echo_and_unrelated_areas() {
        use super::should_emit_setting_change;

        assert!(!should_emit_setting_change(0, "TraySettings", false));
        assert!(!should_emit_setting_change(0, "intl", false));
        assert!(!should_emit_setting_change(0, "", false));
        assert!(!should_emit_setting_change(0, "ImmersiveColorSet", true));
        assert!(should_emit_setting_change(0, "ImmersiveColorSet", false));
        assert!(should_emit_setting_change(0, "WindowsThemeElement", false));
        assert!(should_emit_setting_change(0x0014, "Wallpaper", true));
    }

    #[test]
    fn names_a_process_from_the_process_list() {
        let own = std::env::current_exe().unwrap();
        let expected = own.file_name().unwrap().to_string_lossy().to_string();
        let found = unsafe { super::process_exe_name(std::process::id()) };
        assert_eq!(found.as_deref(), Some(expected.as_str()));
    }

    #[test]
    fn empty_title_console_windows_show_only_when_enabled() {
        use super::should_include_window_title;

        assert!(should_include_window_title(5, "Chrome_WidgetWin_1", false));
        assert!(!should_include_window_title(0, "Chrome_WidgetWin_1", true));
        assert!(!should_include_window_title(0, "ConsoleWindowClass", false));
        assert!(should_include_window_title(0, "ConsoleWindowClass", true));
    }
}
