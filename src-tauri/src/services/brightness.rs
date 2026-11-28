use std::sync::atomic::Ordering;
use std::sync::mpsc::channel;
use std::time::Duration;
use windows::Win32::Foundation::LPARAM;
use windows::core::BOOL;
use crate::types::*;
use crate::state::*;
use crate::utils::*;

static DDC_FOUND: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static DDC_APPLIED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Returns how many external monitors were found and how many took the new value.
fn set_physical_monitors_brightness(brightness: u32) -> (u32, u32) {
    DDC_FOUND.store(0, Ordering::Relaxed);
    DDC_APPLIED.store(0, Ordering::Relaxed);
    unsafe {
        use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
        use windows::Win32::Foundation::{RECT, LPARAM};
        use windows::core::BOOL;

        unsafe extern "system" fn monitor_enum_proc(
            hmonitor: HMONITOR,
            _: HDC,
            _: *mut RECT,
            lparam: LPARAM,
        ) -> BOOL {
            let brightness = lparam.0 as u32;
            #[repr(C)]
            #[derive(Clone, Copy)]
            struct PHYSICAL_MONITOR {
                h_physical_monitor: usize,
                sz_physical_monitor_description: [u16; 128],
            }
            #[link(name = "dxva2")]
            extern "system" {
                fn GetNumberOfPhysicalMonitorsFromHMONITOR(
                    hMonitor: HMONITOR,
                    pdwNumberOfPhysicalMonitors: *mut u32,
                ) -> BOOL;
                fn GetPhysicalMonitorsFromHMONITOR(
                    hMonitor: HMONITOR,
                    dwPhysicalMonitorArraySize: u32,
                    pPhysicalMonitorArray: *mut PHYSICAL_MONITOR,
                ) -> BOOL;
                fn DestroyPhysicalMonitors(
                    dwPhysicalMonitorArraySize: u32,
                    pPhysicalMonitorArray: *mut PHYSICAL_MONITOR,
                ) -> BOOL;
                fn SetMonitorBrightness(
                    hMonitor: usize,
                    dwNewBrightness: u32,
                ) -> BOOL;
                fn GetMonitorBrightness(
                    hMonitor: usize,
                    pdwMinimumBrightness: *mut u32,
                    pdwCurrentBrightness: *mut u32,
                    pdwMaximumBrightness: *mut u32,
                ) -> BOOL;
                fn GetVCPFeatureAndVCPFeatureReply(
                    hMonitor: usize,
                    bVCPCode: u8,
                    pvct: *mut u32,
                    pdwCurrentValue: *mut u32,
                    pdwMaximumValue: *mut u32,
                ) -> BOOL;
                fn SetVCPFeature(
                    hMonitor: usize,
                    bVCPCode: u8,
                    dwNewValue: u32,
                ) -> BOOL;
            }

            let mut count = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count).as_bool() && count > 0 {
                let mut monitors = vec![std::mem::zeroed::<PHYSICAL_MONITOR>(); count as usize];
                if GetPhysicalMonitorsFromHMONITOR(hmonitor, count, monitors.as_mut_ptr()).as_bool() {
                    for mon in &monitors {
                        // 0 is a valid handle here, the first monitor gets it.
                        let handle = mon.h_physical_monitor;
                        DDC_FOUND.fetch_add(1, Ordering::Relaxed);
                        // VCP 0x10 is brightness. Unlike the high-level call it needs no capabilities string, which many DisplayPort monitors lack.
                        let (mut kind, mut cur, mut vcp_max) = (0u32, 0u32, 0u32);
                        if !GetVCPFeatureAndVCPFeatureReply(handle, 0x10, &mut kind, &mut cur, &mut vcp_max).as_bool() || vcp_max == 0 {
                            vcp_max = 100;
                        }
                        let target = vcp_max * brightness / 100;
                        let mut done = false;
                        for attempt in 0..3 {
                            if SetVCPFeature(handle, 0x10, target).as_bool() {
                                done = true;
                                break;
                            }
                            if attempt < 2 {
                                std::thread::sleep(std::time::Duration::from_millis(60));
                            }
                        }
                        if done {
                            DDC_APPLIED.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let (mut min, mut now, mut max) = (0u32, 0u32, 0u32);
                        if GetMonitorBrightness(handle, &mut min, &mut now, &mut max).as_bool() && max > min
                            && SetMonitorBrightness(handle, min + (max - min) * brightness / 100).as_bool()
                        {
                            DDC_APPLIED.fetch_add(1, Ordering::Relaxed);
                            continue;
                        }
                        let code = windows::Win32::Foundation::GetLastError().0;
                        let name = String::from_utf16_lossy(&mon.sz_physical_monitor_description).trim_end_matches('\0').to_string();
                        crate::diagnostics::log(&format!("brightness: monitor \"{name}\" did not accept a brightness change over DDC/CI, error {code}"));
                    }
                    let _ = DestroyPhysicalMonitors(count, monitors.as_mut_ptr());
                }
            }
            true.into()
        }

        let _ = EnumDisplayMonitors(
            None, None,
            Some(monitor_enum_proc),
            LPARAM(brightness as isize),
        );
    }
    (DDC_FOUND.load(Ordering::Relaxed), DDC_APPLIED.load(Ordering::Relaxed))
}

/// Keeps external monitors working when the laptop WMI setup fails.
fn external_brightness_only(rx: std::sync::mpsc::Receiver<u32>) {
    crate::diagnostics::log("brightness: WMI unavailable, controlling external monitors only");
    while let Ok(mut brightness) = rx.recv() {
        while let Ok(newer) = rx.try_recv() {
            brightness = newer;
        }
        let (found, applied) = set_physical_monitors_brightness(brightness.min(100));
        if found > 0 && applied == 0 {
            crate::diagnostics::log(&format!("brightness: {found} external monitor(s) found, none accepted {brightness}"));
        }
    }
}

pub fn setup_brightness_worker() {
    let (tx, rx) = channel::<u32>();
    let _ = BRIGHTNESS_SENDER.set(tx);
    std::thread::spawn(move || unsafe {
        // Direct WMI COM + DXVA2 implementation (zero child processes spawned).
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED, CoCreateInstance, CoSetProxyBlanket, CLSCTX_ALL,
            EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE,
        };
        use windows::Win32::System::Wmi::{IWbemLocator, IWbemClassObject, WbemLocator, WBEM_GENERIC_FLAG_TYPE};
        use windows::Win32::System::Variant::{VARIANT, VariantClear, VARENUM};

        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let locator: IWbemLocator = match CoCreateInstance(&WbemLocator, None, CLSCTX_ALL) {
            Ok(l) => l,
            Err(_) => {
                let _ = CoUninitialize();
                external_brightness_only(rx);
                return;
            }
        };
        let ns = windows::core::BSTR::from("root\\WMI");
        let empty_bstr = windows::core::BSTR::new();
        let services = match locator.ConnectServer(&ns, &empty_bstr, &empty_bstr, &empty_bstr, 0, &empty_bstr, None) {
            Ok(s) => s,
            Err(_) => {
                let _ = CoUninitialize();
                external_brightness_only(rx);
                return;
            }
        };

        // Without impersonation on the proxy WMI answers ACCESS_DENIED and every write was silently dropped.
        // 10 = RPC_C_AUTHN_WINNT, 0 = RPC_C_AUTHZ_NONE.
        let _ = CoSetProxyBlanket(
            &services, 10, 0, windows::core::PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE,
        );

        // GetMethod only works on the class definition, not an instance. Resolve the input signature once.
        let set_brightness_params = {
            let mut class_def: Option<IWbemClassObject> = None;
            let mut in_cls: Option<IWbemClassObject> = None;
            if services
                .GetObject(&windows::core::BSTR::from("WmiMonitorBrightnessMethods"), WBEM_GENERIC_FLAG_TYPE(0), None, Some(&mut class_def), None)
                .is_ok()
            {
                if let Some(ref class_def) = class_def {
                    let _ = class_def.GetMethod(windows::core::w!("WmiSetBrightness"), 0i32, &mut in_cls, std::ptr::null_mut());
                }
            }
            in_cls
        };

        while let Ok(mut brightness) = rx.recv() {
            // A drag queues a value per event and each write is slow, only the latest matters.
            while let Ok(newer) = rx.try_recv() {
                brightness = newer;
            }
            let brightness = brightness.min(100);
            // 1. Laptop internal panel via WMI WmiMonitorBrightnessMethods
            let wql = windows::core::BSTR::from("WQL");
            let q = windows::core::BSTR::from("SELECT * FROM WmiMonitorBrightnessMethods");
            if let Ok(enum_obj) = services.ExecQuery(&wql, &q, WBEM_GENERIC_FLAG_TYPE(0), None) {
                let mut row = [None::<IWbemClassObject>; 1];
                let mut returned = 0u32;
                while enum_obj.Next(-1i32, &mut row, &mut returned).is_ok() && returned > 0 {
                    if let Some(obj) = row[0].take() {
                        let mut var = VARIANT::default();
                        if obj.Get(windows::core::w!("__RELPATH"), 0i32, &mut var, None, None).is_ok() {
                            let relpath_str = var.Anonymous.Anonymous.Anonymous.bstrVal.to_string();
                            let _ = VariantClear(&mut var);
                            if !relpath_str.is_empty() {
                                let obj_path = windows::core::BSTR::from(relpath_str.as_str());
                                let method_name = windows::core::BSTR::from("WmiSetBrightness");

                                {
                                    if let Some(ref in_cls) = set_brightness_params {
                                        if let Ok(in_params) = in_cls.SpawnInstance(0i32) {
                                            let mut b_var = VARIANT::default();
                                            let b_anon = &mut b_var.Anonymous.Anonymous;
                                            b_anon.vt = VARENUM(17); // VT_UI1
                                            b_anon.Anonymous.bVal = brightness as u8;
                                            let _ = in_params.Put(windows::core::w!("Brightness"), 0i32, &b_var, 0);

                                            let mut t_var = VARIANT::default();
                                            let t_anon = &mut t_var.Anonymous.Anonymous;
                                            t_anon.vt = VARENUM(3); // VT_I4
                                            t_anon.Anonymous.lVal = 0i32;
                                            let _ = in_params.Put(windows::core::w!("Timeout"), 0i32, &t_var, 0);

                                            let _ = services.ExecMethod(
                                                &obj_path, &method_name, WBEM_GENERIC_FLAG_TYPE(0), None,
                                                Some(&in_params), None, None,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 2. Desktop external monitor via Physical Monitor API (DXVA2 DDC/CI)
            let (found, applied) = set_physical_monitors_brightness(brightness);
            if found > 0 && applied == 0 {
                crate::diagnostics::log(&format!("brightness: {found} external monitor(s) found, none accepted {brightness}"));
            }


        }
        let _ = CoUninitialize();
    });
}
