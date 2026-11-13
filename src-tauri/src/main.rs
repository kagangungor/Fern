#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod startup;
mod types;
mod state;
mod utils;
mod capture;
mod attention;
mod agent_api;
mod services;
mod windows_accent;
mod hotkeys;
mod live_preview;
mod commands;
mod monitors;
mod updater;
mod uninstall_registry;
mod native_uninstall;
mod diagnostics;
mod taskbar_pins;
mod tray_icons;
mod caldav;
mod calendar_sync;
mod ics;
mod active_look;
mod connections;
mod audio_devices;
mod wallpaper;
mod i18n;
mod shelf;
mod on_demand_windows;
mod memory_saver;
mod webview_args;
mod window_region;
mod buds;
mod usb;
mod jack;
mod bluetooth;
mod battery;
mod dock_extras;
mod launchers;
mod privacy_hud;
mod app_audio;
mod lyrics;

use tauri::Manager;
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT};
use windows::core::BOOL;
use std::sync::atomic::Ordering;

use crate::state::*;
use crate::utils::*;
use crate::services::*;
use crate::commands::*;

unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> BOOL {
    if ctrl_type == CTRL_C_EVENT || ctrl_type == CTRL_BREAK_EVENT || ctrl_type == CTRL_CLOSE_EVENT {
        set_taskbar_visibility(true, true);
        NATIVE_TASKBAR_HIDDEN.store(false, Ordering::Relaxed);

    }
    BOOL(0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = agent_api::run_cli(&args) {
        std::process::exit(code);
    }

    if native_uninstall::requested() {
        native_uninstall::run();
        return;
    }

    webview_args::apply();
    diagnostics::init();

    startup::begin();

    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), true);
    }

    // Single-instance enforcement
    unsafe {
        use windows::Win32::System::Threading::{CreateMutexW, CreateEventW, OpenEventW, SetEvent, SYNCHRONIZATION_ACCESS_RIGHTS};
        use windows::Win32::Foundation::{GetLastError, CloseHandle};

        let mutex_name: Vec<u16> = "FernSingleInstance".encode_utf16().chain(std::iter::once(0)).collect();
        let event_name: Vec<u16> = "FernOpenSettings".encode_utf16().chain(std::iter::once(0)).collect();

        let h_mutex = CreateMutexW(None, true, windows::core::PCWSTR(mutex_name.as_ptr())).ok();
        let err = GetLastError();

        if err.0 == 183 {
            // Another instance is already running — signal it to open settings
            if let Ok(h_event) = OpenEventW(SYNCHRONIZATION_ACCESS_RIGHTS(0x00100002), false, windows::core::PCWSTR(event_name.as_ptr())) {
                let _ = SetEvent(h_event);
                let _ = CloseHandle(h_event);
            }
            if let Some(h) = h_mutex { let _ = CloseHandle(h); }
            return;
        }

        if let Ok(h_event) = CreateEventW(None, false, false, windows::core::PCWSTR(event_name.as_ptr())) {
            let _ = SINGLE_INSTANCE_EVENT_HANDLE.set(h_event.0 as isize);
        }
        if let Some(h) = h_mutex { let _ = SINGLE_INSTANCE_MUTEX_HANDLE.set(h.0 as isize); }
    }

    setup_brightness_worker();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_drag::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .invoke_handler(tauri::generate_handler![
            startup::get_autostart,
            startup::set_autostart,
            hide_native_osd,
            setup_shelf_drop,
            shelf_thumbnail,
            shelf_open,
            shelf_prune,
            shelf_drag_icon,
            shelf_text_file,
            pointer_over_notch,
            open_settings_window,
            sync_windows_accent,
            restore_windows_accent,
            has_windows_accent_backup,
            windows_accent_backup_exists,
            reload_hotkeys,
            set_live_thumbnails,
            set_live_thumbnails_opacity,
            clear_live_thumbnails,
            pick_program,
            close_app,
            get_target_app_volume,
            set_target_app_volume,
            adjust_target_app_volume,
            toggle_target_app_mute,
            is_dwm_composition_enabled,
            on_demand_windows::on_demand_window_ready,
            close_settings_window,
            take_settings_page,
            open_calendar_window,
            close_calendar_window,
            window_region::set_window_regions,
            open_tray_window,
            open_tray_at_cursor,
            open_tray_from_button,
            show_desktop,
            open_wifi_settings,
            open_sound_settings,
            open_notification_center,
            set_ignore_cursor_events,
            set_window_height,
            resize_settings_window,
            hide_overlay,
            set_splash_fullscreen,
            sync_overlay_position,
            select_media_session,
            media_play_pause,
            media_next,
            media_previous,
            media_seek,
            get_app_media_sessions,
            app_media_control,
            open_media_source_app,

            init_dock,
            toggle_dock,
            change_dock_mode,
            change_notch_mode,
            sync_appbar,
            open_app,
            launch_new_instance,
            update_dock_rect,
            update_notch_rect,
            focus_agent_window,
            agent_answer,
            pick_agent_sound,
            reset_agent_sound,
            agent_sound_data,
            agent_sound_custom,
            set_dock_hovered,
            set_notch_hovered,
            set_notch_hidden,
            set_dock_dragging,
            get_active_windows,
            get_app_icon,
            get_installed_apps,
            save_pinned_apps,
            load_pinned_apps,
            clear_icon_cache,
            set_custom_icon,
            remove_custom_icon,
            get_custom_icons,
            set_menu_open,
            set_window_keyboard_focus,
            get_drives,
            open_dock_extra,
            dock_extras::get_recycle_bin_status,
            dock_extras::empty_recycle_bin,
            dock_extras::eject_drive,
            dock_extras::shelf_read_text_preview,
            launchers::get_search_launchers,
            launchers::trigger_search_launcher,
            privacy_hud::get_microphone_mute,
            privacy_hud::set_microphone_mute,
            privacy_hud::toggle_microphone_mute,
            privacy_hud::get_privacy_status,
            audio_devices::get_audio_devices,
            audio_devices::set_default_audio_device,
            attention::get_attention_windows,
            wallpaper::get_wallpaper_backdrop,
            wallpaper::is_backdrop_covered,
            connections::get_lan_status,
            connections::open_ethernet_settings,
            connections::wifi_scan,
            connections::wifi_connect,
            connections::wifi_disconnect,
            connections::wifi_forget,
            bluetooth::bluetooth_devices,
            bluetooth::bluetooth_list_devices,
            bluetooth::bluetooth_set_connected,
            bluetooth::bluetooth_refresh_buds,
            bluetooth::bluetooth_discovery,
            bluetooth::bluetooth_pair,
            bluetooth::bluetooth_pairing_respond,
            bluetooth::bluetooth_unpair,
            usb::usb_list_devices,
            jack::jack_list_devices,
            battery::get_battery_details,
            focus_window,
            bring_hwnd_to_front,
            focus_app_windows,
            close_window,
            end_task,
            quit_fern,
            restart_fern,
            uninstall_fern,
            get_diagnostics,
            open_log_folder,
            set_hitbox_logging,
            get_hitbox_logging,
            get_volume,
            get_volume_state,
            get_brightness,
            set_volume,
            get_audio_sessions,
            set_app_volume,
            set_app_mute,
            set_volume_mixer_rect,
            clear_volume_mixer_rect,
            save_setting,
            load_settings,
            reset_settings,
            capture_window_thumbnail,
            get_wifi_status,
            set_wifi_state,
            get_bluetooth_state,
            set_bluetooth_state,
            open_bluetooth_settings,
            open_airplane_mode_settings,
            set_brightness,
            get_battery_saver_state,
            has_battery,
            open_battery_saver_settings,
            get_system_accent_color,
            get_monitors,
            get_dock_mode_for_window,
            get_notch_mode_for_window,
            set_dock_mode_for_window,
            set_notch_mode_for_window,
            get_cpu_usage,
            get_ram_usage,
            get_disk_space,
            get_network_speed,
            get_running_processes,
            kill_process,
            export_settings,
            import_settings,
            export_settings_dialog,
            import_settings_dialog,
            updater::check_for_updates,
            updater::install_update,
            updater::get_update_state,
            caldav::caldav_connect,
            caldav::caldav_disconnect,
            calendar_sync::caldav_sync_now,
            calendar_sync::caldav_get_cache,
            ics::ics_add_link,
            ics::ics_remove_link,
            ics::ics_detect,
            get_lyrics
        ])
        .setup(|app| {
            init_taskbar_marker(app.handle());
            // Crash-recovery: if a previous session was force-killed while the native
            // taskbar was hidden, restore it now. Runs before the frontend re-hides it
            // (init_dock fires after a delay), so the flag must be removed first.
            if std::env::var_os("FERN_RESTARTING").is_some() {
                std::env::remove_var("FERN_RESTARTING");
                RELAUNCHED.store(true, Ordering::Relaxed);
            } else if taskbar_marker_exists() {
                RELAUNCHED.store(true, Ordering::Relaxed);
                restore_taskbar_after_crash();
                NATIVE_TASKBAR_HIDDEN.store(false, Ordering::Relaxed);
            }

            crate::utils::init_settings_cache(app.handle());
            startup::migrate(app.handle());
            agent_api::start(app.handle().clone());
            crate::hotkeys::start(app.handle().clone());
            privacy_hud::start(app.handle().clone());

            #[cfg(not(debug_assertions))]
            std::thread::spawn(crate::uninstall_registry::heal_uninstall_registration);

            // Update check on startup (non-blocking). Always runs so the UI can
            // show an update badge; auto-install only happens when the user
            // enabled it and the release has aged past the rollout gate.
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    updater::run_startup_check(app_handle).await;
                });
            }

            crate::calendar_sync::start_background_sync(app.handle().clone());

            let window = app.get_webview_window("main").unwrap();
            let dock_win = app.get_webview_window("dock").unwrap();

            crate::services::wire_window_events(app.handle(), &window, crate::types::WindowKind::Notch, false);
            crate::services::wire_window_events(app.handle(), &dock_win, crate::types::WindowKind::Dock, false);

            for label in ["main", "dock", "overlay"] {
                if let Some(w) = app.get_webview_window(label) {
                    let _ = w.set_ignore_cursor_events(true);
                }
            }

            sync_overlays(app.handle());

            {
                let ah = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    crate::monitors::sync_monitor_windows(&ah);
                });
            }

            // Initialize the overlay window — on Windows, set_position doesn't
            // take effect on a window that has never been shown. Show it once
            // to register it with the compositor, then hide immediately.
            if let Some(ov_win) = app.get_webview_window("overlay") {
                let _ = ov_win.show();
                let _ = ov_win.hide();
                let overlay_handle = app.handle().clone();
                ov_win.on_window_event(move |e| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                        api.prevent_close();
                        restore_taskbar_and_exit(&overlay_handle);
                    }
                });
            }

            watch_webview_processes(app.handle());
            setup_mouse_hook(app.handle().clone());
            setup_display_change_monitor(app.handle().clone());
            setup_window_change_hook(app.handle().clone());
            attention::start(app.handle().clone());
            {
                let _ = crate::state::THUMBNAIL_CACHE.set(std::sync::Mutex::new(std::collections::HashMap::new()));
                let _ = crate::state::FOCUS_TIMESTAMPS.set(std::sync::Mutex::new(std::collections::HashMap::new()));
                // Set before the scan so it can store results.
                let _ = crate::state::INSTALLED_APPS_CACHE.set(std::sync::Mutex::new(Vec::new()));
            }
            setup_thumbnail_capture(app.handle().clone());
            trigger_app_scan();
            let tx = setup_system_worker(app.handle().clone());
            let _ = COMMAND_SENDER.set(tx.clone());
            let _hook = services::setup_keyboard_hook(app.handle().clone());
            setup_taskbar_hook();
            start_window_style_guard(app.handle().clone());
            setup_audio_visualization(app.handle().clone());
            setup_settings_watcher(app.handle().clone());
            memory_saver::start(app.handle().clone());
            bluetooth::setup_bluetooth_notifications(app.handle().clone());
            usb::setup_usb_notifications(app.handle().clone());
            jack::setup_jack_notifications(app.handle().clone());

            // Listen for second-instance signal to open settings
            if let Some(&h_event) = SINGLE_INSTANCE_EVENT_HANDLE.get() {
                if h_event != 0 {
                    let app_handle = app.handle().clone();
                    std::thread::spawn(move || {
                        use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
                        use windows::Win32::Foundation::{HANDLE, WAIT_FAILED, WAIT_OBJECT_0};
                        let h_event = HANDLE(h_event as *mut _);
                        loop {
                            let result = unsafe { WaitForSingleObject(h_event, INFINITE) };
                            if result == WAIT_OBJECT_0 {
                                crate::on_demand_windows::open(&app_handle, crate::on_demand_windows::SETTINGS);
                            } else if result == WAIT_FAILED {
                                break;
                            }
                        }
                    });
                }
            }

            {
                use tauri::menu::{Menu, MenuItem};
                use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    crate::tray_icons::warm_up();
                });
                let handle = app.handle();
                let make = |id: &str, key: &'static str| -> tauri::Result<MenuItem<tauri::Wry>> {
                    let item = MenuItem::with_id(app, id, crate::i18n::tr(handle, key), true, None::<&str>)?;
                    crate::i18n::register_tray_item(item.clone(), key);
                    Ok(item)
                };
                let quit_item = make("quit", "general.quit")?;
                let restart_item = make("restart", "general.restart")?;
                let settings_item = make("settings", "dockmenu.openSettings")?;
                let calendar_item = make("calendar", "dockmenu.openCalendar")?;
                let menu = Menu::with_items(app, &[&calendar_item, &settings_item, &restart_item, &quit_item])?;
                let ah = app.handle().clone();
                TrayIconBuilder::new()
                    .icon(app.default_window_icon().unwrap().clone())
                    .tooltip("Fern")
                    .menu(&menu)
                    .on_menu_event(move |_, event| match event.id().as_ref() {
                        "quit" => {
                            crate::commands::restore_taskbar_and_exit(&ah);
                        }
                        "restart" => {
                            crate::commands::relaunch_fern(&ah);
                        }
                        "settings" => {
                            crate::on_demand_windows::open(&ah, crate::on_demand_windows::SETTINGS);
                        }
                        "calendar" => {
                            crate::commands::open_calendar_window(ah.clone());
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } = event
                        {
                            crate::on_demand_windows::open(tray.app_handle(), crate::on_demand_windows::SETTINGS);
                        }
                    })
                    .build(app)?;
            }

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");
    app.run(|_, event| {
        if let tauri::RunEvent::Exit = event {
            set_taskbar_visibility(true, true);
            NATIVE_TASKBAR_HIDDEN.store(false, Ordering::Relaxed);

        }
    });
}
