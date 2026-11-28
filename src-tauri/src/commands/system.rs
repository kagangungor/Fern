use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

use crate::types::{BrightnessChangeEvent, WifiStatus};

// ── Radio helpers — Windows Runtime Windows.Devices.Radios ───────────────────
// Replaces PowerShell -ExecutionPolicy Bypass scripts for WiFi/Bluetooth state.

fn get_radio_state_sync(kind: windows::Devices::Radios::RadioKind) -> Result<bool, String> {
    use windows::Devices::Radios::{Radio, RadioState};
    let _com = crate::utils::ComGuard::mta();
    let radios = Radio::GetRadiosAsync()
        .and_then(|op| op.get())
        .map_err(|e| e.to_string())?;
    for i in 0..radios.Size().unwrap_or(0) {
        if let Ok(radio) = radios.GetAt(i) {
            if radio.Kind().ok() == Some(kind) {
                return Ok(radio.State().ok() == Some(RadioState::On));
            }
        }
    }
    Ok(false)
}

fn set_radio_state_sync(kind: windows::Devices::Radios::RadioKind, enabled: bool) -> Result<(), String> {
    use windows::Devices::Radios::{Radio, RadioState};
    let _com = crate::utils::ComGuard::mta();
    let radios = Radio::GetRadiosAsync()
        .and_then(|op| op.get())
        .map_err(|e| e.to_string())?;
    for i in 0..radios.Size().unwrap_or(0) {
        if let Ok(radio) = radios.GetAt(i) {
            if radio.Kind().ok() == Some(kind) {
                let target = if enabled { RadioState::On } else { RadioState::Off };
                let _ = radio.SetStateAsync(target).and_then(|op| op.get());
                return Ok(());
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub fn get_brightness() -> u32 {
    crate::state::CURRENT_BRIGHTNESS.load(std::sync::atomic::Ordering::Relaxed)
}

// Wi-Fi status: the radio state alone can't tell on from connected, so ask the WLAN API too.

fn is_wlan_connected_sync() -> bool {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::WiFi::{
        wlan_intf_opcode_current_connection, WlanCloseHandle, WlanEnumInterfaces, WlanFreeMemory,
        WlanOpenHandle, WlanQueryInterface, WLAN_INTERFACE_INFO_LIST,
    };

    unsafe {
        let mut negotiated_version = 0u32;
        let mut client_handle = HANDLE::default();
        if WlanOpenHandle(2, None, &mut negotiated_version, &mut client_handle) != 0 {
            return false;
        }

        let mut interface_list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let mut connected = false;
        if WlanEnumInterfaces(client_handle, None, &mut interface_list) == 0
            && !interface_list.is_null()
        {
            let interfaces = std::slice::from_raw_parts(
                (*interface_list).InterfaceInfo.as_ptr(),
                (*interface_list).dwNumberOfItems as usize,
            );
            for interface in interfaces {
                let mut data_size = 0u32;
                let mut data: *mut std::ffi::c_void = std::ptr::null_mut();
                // Errors with ERROR_INVALID_STATE when not associated, so success means connected.
                let result = WlanQueryInterface(
                    client_handle,
                    &interface.InterfaceGuid,
                    wlan_intf_opcode_current_connection,
                    None,
                    &mut data_size,
                    &mut data,
                    None,
                );
                if !data.is_null() {
                    WlanFreeMemory(data);
                }
                if result == 0 {
                    connected = true;
                    break;
                }
            }
            WlanFreeMemory(interface_list as *const _);
        }

        WlanCloseHandle(client_handle, None);
        connected
    }
}

/// Wi-Fi radio and connection state for the quick settings tile.
#[tauri::command]
pub async fn get_wifi_status() -> Result<WifiStatus, String> {
    tauri::async_runtime::spawn_blocking(|| -> Result<WifiStatus, String> {
        let enabled = get_radio_state_sync(windows::Devices::Radios::RadioKind::WiFi)?;
        let connected = enabled && is_wlan_connected_sync();
        Ok(WifiStatus { enabled, connected })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_wifi_state(enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_radio_state_sync(windows::Devices::Radios::RadioKind::WiFi, enabled)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn get_bluetooth_state() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(|| {
        get_radio_state_sync(windows::Devices::Radios::RadioKind::Bluetooth)
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn set_bluetooth_state(enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        set_radio_state_sync(windows::Devices::Radios::RadioKind::Bluetooth, enabled)
    }).await.map_err(|e| e.to_string())?
}

// Settings openers — use ShellExecuteA directly instead of spawning powershell

#[tauri::command]
pub fn open_bluetooth_settings() {
    unsafe {
        use windows::Win32::UI::Shell::ShellExecuteA;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let _ = ShellExecuteA(None, windows::core::PCSTR(c"open".as_ptr() as *const u8), windows::core::PCSTR(c"ms-settings:bluetooth".as_ptr() as *const u8), windows::core::PCSTR::null(), windows::core::PCSTR::null(), SW_SHOWNORMAL);
    }
}

#[tauri::command]
pub fn open_airplane_mode_settings() {
    unsafe {
        use windows::Win32::UI::Shell::ShellExecuteA;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let _ = ShellExecuteA(None, windows::core::PCSTR(c"open".as_ptr() as *const u8), windows::core::PCSTR(c"ms-settings:network-airplanemode".as_ptr() as *const u8), windows::core::PCSTR::null(), windows::core::PCSTR::null(), SW_SHOWNORMAL);
    }
}

#[tauri::command]
pub fn set_brightness(app: AppHandle, brightness: u32) {
    let val = brightness.min(100);
    crate::state::CURRENT_BRIGHTNESS.store(val, Ordering::Relaxed);
    crate::state::LAST_BRIGHTNESS_CHANGE.store(crate::utils::get_now_ms(), Ordering::Relaxed);
    if let Ok(dir) = app.path().app_config_dir() {
        let _ = std::fs::write(dir.join("last_brightness.txt"), val.to_string());
    }
    let _ = app.emit("brightness-change", BrightnessChangeEvent { brightness: val });
    if let Some(tx) = crate::state::BRIGHTNESS_SENDER.get() {
        let _ = tx.send(val);
    }
}

#[tauri::command]
pub async fn get_battery_saver_state() -> Result<bool, String> {
    // Uses Windows Runtime PowerManager — no PowerShell required.
    tauri::async_runtime::spawn_blocking(|| {
        let _com = crate::utils::ComGuard::mta();
        use windows::System::Power::{EnergySaverStatus, PowerManager};
        match PowerManager::EnergySaverStatus() {
            Ok(status) => Ok(status == EnergySaverStatus::On),
            Err(_) => Ok(false), // No battery / not supported on this device
        }
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn has_battery() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let _com = crate::utils::ComGuard::mta();
        use windows::System::Power::{BatteryStatus, PowerManager};
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut power = SYSTEM_POWER_STATUS::default();
        let power_ok = unsafe { GetSystemPowerStatus(&mut power) }.is_ok();
        let manager = PowerManager::BatteryStatus();
        crate::diagnostics::log(&format!(
            "battery: power_ok={power_ok} flag={} percent={} manager={:?}",
            power.BatteryFlag,
            power.BatteryLifePercent,
            manager.as_ref().map(|s| s.0)
        ));
        if power_ok && (power.BatteryFlag == 128 || (power.BatteryFlag == 255 && power.BatteryLifePercent == 255)) {
            return Ok(false);
        }
        match manager {
            Ok(status) => Ok(status != BatteryStatus::NotPresent),
            Err(_) => Ok(true),
        }
    }).await.map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn open_battery_saver_settings() {
    unsafe {
        use windows::Win32::UI::Shell::ShellExecuteA;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let _ = ShellExecuteA(None, windows::core::PCSTR(c"open".as_ptr() as *const u8), windows::core::PCSTR(c"ms-settings:batterysaver".as_ptr() as *const u8), windows::core::PCSTR::null(), windows::core::PCSTR::null(), SW_SHOWNORMAL);
    }
}

// --- System metrics for status widgets ---

fn compute_cpu_usage_percent(prev: (u64, u64, u64), curr: (u64, u64, u64)) -> u32 {
    let idle_delta = curr.0.saturating_sub(prev.0);
    let kernel_delta = curr.1.saturating_sub(prev.1);
    let user_delta = curr.2.saturating_sub(prev.2);
    let total = kernel_delta.saturating_add(user_delta);
    if total == 0 {
        return 0;
    }
    ((total.saturating_sub(idle_delta) * 100) / total).min(100) as u32
}

#[tauri::command]
pub async fn get_ping_ms(app: AppHandle) -> Option<u32> {
    let target = crate::utils::get_setting_str(&app, "fern-ping-host")
        .unwrap_or_else(|| "1.1.1.1:443".to_string());
    tauri::async_runtime::spawn_blocking(move || {
        use std::net::{TcpStream, ToSocketAddrs};
        let addr = target.to_socket_addrs().ok()?.next()?;
        let start = std::time::Instant::now();
        TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(1500)).ok()?;
        Some(start.elapsed().as_millis() as u32)
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
pub fn get_cpu_usage() -> Result<u32, String> {
    static LAST_TIMES: std::sync::OnceLock<std::sync::Mutex<(u64, u64, u64)>> = std::sync::OnceLock::new();
    let lock = LAST_TIMES.get_or_init(|| std::sync::Mutex::new((0, 0, 0)));

    unsafe {
        let mut idle = windows::Win32::Foundation::FILETIME::default();
        let mut kernel = windows::Win32::Foundation::FILETIME::default();
        let mut user = windows::Win32::Foundation::FILETIME::default();
        windows::Win32::System::Threading::GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user))
            .map_err(|e| format!("GetSystemTimes failed: {e}"))?;

        let curr = (
            ((idle.dwHighDateTime as u64) << 32) | idle.dwLowDateTime as u64,
            ((kernel.dwHighDateTime as u64) << 32) | kernel.dwLowDateTime as u64,
            ((user.dwHighDateTime as u64) << 32) | user.dwLowDateTime as u64,
        );

        let mut prev = lock.lock().unwrap_or_else(|e| e.into_inner());
        let usage = compute_cpu_usage_percent(*prev, curr);
        *prev = curr;
        Ok(usage)
    }
}

#[tauri::command]
pub fn get_ram_usage() -> Result<f32, String> {
    unsafe {
        let mut mem = windows::Win32::System::SystemInformation::MEMORYSTATUSEX::default();
        mem.dwLength = std::mem::size_of::<windows::Win32::System::SystemInformation::MEMORYSTATUSEX>() as u32;
        windows::Win32::System::SystemInformation::GlobalMemoryStatusEx(&mut mem)
            .map_err(|e| format!("GlobalMemoryStatusEx failed: {e}"))?;
        Ok(mem.dwMemoryLoad as f32)
    }
}

#[tauri::command]
pub fn get_disk_space() -> Result<u64, String> {
    unsafe {
        let mut free_bytes = 0u64;
        let mut total_bytes = 0u64;
        let mut total_free = 0u64;
        windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            windows::core::w!("C:\\"),
            Some(&mut free_bytes),
            Some(&mut total_bytes),
            Some(&mut total_free),
        ).map_err(|e| format!("GetDiskFreeSpaceEx failed: {e}"))?;
        Ok(free_bytes / (1024 * 1024 * 1024))
    }
}

#[derive(serde::Serialize, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub exe_path: Option<String>,
    pub memory_kb: u64,
}

const PROTECTED_PROCESS_NAMES: &[&str] = &[
    "system",
    "system idle process",
    "registry",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "smss.exe",
    "explorer.exe",
    "dwm.exe",
    "fern.exe",
];

#[tauri::command]
pub fn get_running_processes() -> Result<Vec<ProcessInfo>, String> {
    unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
        };
        use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        };

        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(|e| e.to_string())?;
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut results = Vec::new();

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name = String::from_utf16_lossy(&entry.szExeFile)
                    .trim_end_matches('\0')
                    .to_string();
                let pid = entry.th32ProcessID;

                if pid != 0 && !name.is_empty() && !PROTECTED_PROCESS_NAMES.contains(&name.to_lowercase().as_str()) {
                    let mut exe_path = None;
                    let mut memory_kb = 0u64;
                    if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                        let mut buf = [0u16; 512];
                        let mut size = buf.len() as u32;
                        if QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut size).is_ok() {
                            exe_path = Some(String::from_utf16_lossy(&buf[..size as usize]));
                        }
                        let mut counters = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
                        if GetProcessMemoryInfo(handle, &mut counters, counters.cb).is_ok() {
                            memory_kb = (counters.WorkingSetSize as u64) / 1024;
                        }
                        let _ = CloseHandle(handle);
                    }
                    results.push(ProcessInfo { pid, name, exe_path, memory_kb });
                }

                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);

        results.sort_by(|a, b| b.memory_kb.cmp(&a.memory_kb));
        Ok(results)
    }
}

#[tauri::command]
pub fn kill_process(pid: u32, _name: Option<String>) -> Result<(), String> {
    unsafe {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
            PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        };

        // Resolve the real image name from the PID instead of trusting the client-supplied name
        let query_handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|e| format!("Cannot inspect process {}: {}", pid, e))?;

        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let res = QueryFullProcessImageNameW(query_handle, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(query_handle);

        // Fail closed: a process whose identity can't be verified is never terminated
        res.map_err(|e| format!("Cannot verify process {}: {}", pid, e))?;

        let full_path = String::from_utf16_lossy(&buf[..len as usize]);
        let exe_name = std::path::Path::new(&full_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if exe_name.is_empty() || PROTECTED_PROCESS_NAMES.contains(&exe_name.as_str()) {
            return Err(format!("Refusing to end protected process: {}", exe_name));
        }

        let term_handle = OpenProcess(PROCESS_TERMINATE, false, pid)
            .map_err(|e| format!("Cannot open process {} for termination: {}", pid, e))?;
        let result = TerminateProcess(term_handle, 1).map_err(|e| e.to_string());
        let _ = CloseHandle(term_handle);
        result
    }
}

fn compute_rate_per_sec(prev_bytes: u64, curr_bytes: u64, elapsed_ms: u64) -> u64 {
    let delta = curr_bytes.saturating_sub(prev_bytes);
    ((delta as u128 * 1000) / elapsed_ms.max(1) as u128) as u64
}

#[tauri::command]
pub fn get_network_speed() -> Result<(u64, u64), String> {
    static LAST_SAMPLE: std::sync::OnceLock<std::sync::Mutex<Option<(std::time::Instant, u64, u64)>>> =
        std::sync::OnceLock::new();
    let lock = LAST_SAMPLE.get_or_init(|| std::sync::Mutex::new(None));

    let (mut total_in, mut total_out) = (0u64, 0u64);
    unsafe {
        let mut table_ptr: *mut windows::Win32::NetworkManagement::IpHelper::MIB_IF_TABLE2 = std::ptr::null_mut();
        let ret = windows::Win32::NetworkManagement::IpHelper::GetIfTable2(&mut table_ptr);
        if ret.is_ok() && !table_ptr.is_null() {
            let table = &*table_ptr;
            let num_entries = table.NumEntries;
            let rows = table.Table.as_ptr();
            for i in 0..num_entries as usize {
                let row = &*rows.add(i);
                total_in = total_in.saturating_add(row.InOctets);
                total_out = total_out.saturating_add(row.OutOctets);
            }
            windows::Win32::NetworkManagement::IpHelper::FreeMibTable(table_ptr as *const _);
        }
    }

    let now = std::time::Instant::now();
    let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let result = if let Some((last_time, prev_sent, prev_recv)) = *guard {
        let elapsed_ms = now.saturating_duration_since(last_time).as_millis().max(1) as u64;
        (
            compute_rate_per_sec(prev_sent, total_out, elapsed_ms),
            compute_rate_per_sec(prev_recv, total_in, elapsed_ms),
        )
    } else {
        (0, 0)
    };
    *guard = Some((now, total_out, total_in));

    Ok(result)
}

#[tauri::command]
pub fn get_windows_accent_color() -> Option<String> {
    unsafe {
        use windows::Win32::System::Registry::{RegOpenKeyExW, RegQueryValueExW, RegCloseKey, HKEY_CURRENT_USER, KEY_READ, REG_BINARY, REG_DWORD};
        
        // 1. Try reading AccentPalette from Explorer\Accent for the true system accent color
        let subkey = windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent");
        let mut hkey = windows::Win32::System::Registry::HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey, Some(0), KEY_READ, &mut hkey).is_ok() {
            let mut value_type = REG_BINARY;
            let mut palette = [0u8; 32];
            let mut value_size = 32u32;
            let val_name = windows::core::w!("AccentPalette");
            if RegQueryValueExW(
                hkey,
                val_name,
                None,
                Some(&mut value_type),
                Some(palette.as_mut_ptr()),
                Some(&mut value_size)
            ).is_ok() {
                let _ = RegCloseKey(hkey);
                if value_size >= 15 {
                    // bytes 12, 13, 14 are R, G, B of the active accent color
                    let r = palette[12];
                    let g = palette[13];
                    let b = palette[14];
                    return Some(format!("#{:02x}{:02x}{:02x}", r, g, b));
                }
            } else {
                let _ = RegCloseKey(hkey);
            }
        }

        // 2. Fallback to DWM\AccentColor
        let subkey = windows::core::w!("Software\\Microsoft\\Windows\\DWM");
        let mut hkey = windows::Win32::System::Registry::HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey, Some(0), KEY_READ, &mut hkey).is_ok() {
            let mut value_type = REG_DWORD;
            let mut color_val = 0u32;
            let mut value_size = std::mem::size_of::<u32>() as u32;
            let val_name = windows::core::w!("AccentColor");
            if RegQueryValueExW(
                hkey,
                val_name,
                None,
                Some(&mut value_type),
                Some(&mut color_val as *mut u32 as *mut u8),
                Some(&mut value_size)
            ).is_ok() {
                let _ = RegCloseKey(hkey);
                // color_val is AABBGGRR (ABGR format)
                let r = (color_val & 0xff) as u8;
                let g = ((color_val >> 8) & 0xff) as u8;
                let b = ((color_val >> 16) & 0xff) as u8;
                return Some(format!("#{:02x}{:02x}{:02x}", r, g, b));
            }
            let _ = RegCloseKey(hkey);
        }
    }
    None
}

#[tauri::command]
pub fn get_system_accent_color() -> Result<String, String> {
    // Try reading registry first for exact Windows accent color
    if let Some(color) = get_windows_accent_color() {
        return Ok(color);
    }

    // Fallback to DwmGetColorizationColor
    unsafe {
        let mut color = 0u32;
        let mut opaque = windows::core::BOOL(0);
        if windows::Win32::Graphics::Dwm::DwmGetColorizationColor(&mut color, &mut opaque).is_ok() {
            let r = ((color >> 16) & 0xff) as u8;
            let g = ((color >> 8) & 0xff) as u8;
            let b = (color & 0xff) as u8;
            let hex = format!("#{:02x}{:02x}{:02x}", r, g, b);
            Ok(hex)
        } else {
            Err("Failed to query colorization color".into())
        }
    }
}

#[tauri::command]
pub async fn get_tray_app_names() -> Vec<String> {
    tauri::async_runtime::spawn_blocking(crate::tray_icons::list_tray_app_names)
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn trigger_tray_app(name: String, context_menu: bool) -> bool {
    tauri::async_runtime::spawn_blocking(move || crate::tray_icons::trigger_tray_app(&name, context_menu))
        .await
        .unwrap_or(false)
}

#[cfg(test)]
mod metrics_tests {
    use super::*;
    use super::super::{
        extract_shelf_text_preview, format_picked_program_name, thumbar_command_wparam,
        wait_for_splash_with,
    };
    use super::super::windows::on_screen_restore_rect;

    #[test]
    fn cpu_usage_saturates_instead_of_underflowing() {
        assert_eq!(compute_cpu_usage_percent((100, 100, 100), (100, 100, 100)), 0);
        assert_eq!(compute_cpu_usage_percent((0, 0, 0), (500, 200, 100)), 0);
        assert_eq!(compute_cpu_usage_percent((100, 100, 50), (150, 200, 150)), 75);
    }

    #[test]
    fn network_rate_handles_counter_reset_and_zero_elapsed() {
        assert_eq!(compute_rate_per_sec(1000, 500, 1000), 0);
        assert_eq!(compute_rate_per_sec(0, 2048, 0), 2_048_000);
        assert_eq!(compute_rate_per_sec(1000, 6000, 500), 10_000);
    }

    #[test]
    fn wait_for_splash_returns_immediately_when_finished() {
        let flag = std::sync::atomic::AtomicBool::new(true);
        let started = std::time::Instant::now();
        let finished = tauri::async_runtime::block_on(wait_for_splash_with(&flag, std::time::Duration::from_secs(2)));
        assert!(finished);
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn wait_for_splash_respects_timeout_when_unfinished() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        let finished = tauri::async_runtime::block_on(wait_for_splash_with(&flag, std::time::Duration::from_millis(60)));
        assert!(!finished);
    }

    #[test]
    fn format_picked_program_name_capitalizes_lowercase_first_letter() {
        assert_eq!(format_picked_program_name(Some("spotify")), "Spotify");
        assert_eq!(format_picked_program_name(Some("VSCode")), "VSCode");
        assert_eq!(format_picked_program_name(None), "App");
        assert_eq!(format_picked_program_name(Some("")), "App");
    }

    #[test]
    fn shelf_text_preview_reads_text_and_rejects_binary_or_disallowed() {
        let dir = std::env::temp_dir().join(format!("fern_shelf_preview_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);

        let txt = dir.join("notes.md");
        std::fs::write(&txt, "\n  # Title  \n\n  Line two\n  Line three\n").unwrap();
        assert_eq!(extract_shelf_text_preview(&txt).as_deref(), Some("# Title\nLine two\nLine three"));

        let bin_ext = dir.join("photo.png");
        std::fs::write(&bin_ext, "hello").unwrap();
        assert_eq!(extract_shelf_text_preview(&bin_ext), None);

        let nul_txt = dir.join("nul.txt");
        std::fs::write(&nul_txt, b"abc\0def").unwrap();
        assert_eq!(extract_shelf_text_preview(&nul_txt), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replaces_a_stub_size_with_two_thirds_of_the_work_area() {
        use windows::Win32::Foundation::RECT;
        let rect = |left, top, width, height| RECT { left, top, right: left + width, bottom: top + height };
        let work = rect(0, 0, 2880, 1800);
        assert_eq!(
            on_screen_restore_rect(rect(-32000, -32000, 945, 200), work),
            rect(480, 300, 1920, 1200)
        );
    }

    #[test]
    fn shrinks_an_oversized_window_to_the_work_area_of_its_monitor() {
        use windows::Win32::Foundation::RECT;
        let rect = |left, top, width, height| RECT { left, top, right: left + width, bottom: top + height };
        let work = rect(2880, 100, 1920, 980);
        assert_eq!(
            on_screen_restore_rect(rect(-32000, -32000, 3000, 2000), work),
            work
        );
    }

    #[test]
    fn thumbar_command_wparam_encodes_electron_ids_and_rejects_overflow() {
        assert_eq!(thumbar_command_wparam(0), Some((0x1800 << 16) | 40001));
        assert_eq!(thumbar_command_wparam(3), Some((0x1800 << 16) | 40004));
        assert_eq!(thumbar_command_wparam(6), Some((0x1800 << 16) | 40007));
        assert_eq!(thumbar_command_wparam(7), None);
    }
}
