//! Platform Abstraction Layer isolating OS-specific capabilities behind traits.
//!
//! Callers should access platform functionality via [`current()`], [`capabilities()`],
//! and [`platform_info()`] rather than invoking Win32 APIs directly.

#![allow(dead_code)]

pub mod linux;
pub mod traits;
#[cfg(target_os = "windows")]
pub mod windows;

use serde::Serialize;

pub use traits::{
    PlatformBackend, PlatformCapabilities, PlatformKind, SystemBackend, TaskbarBackend,
    WindowBackend,
};

/// Serializable snapshot of the active platform for diagnostics and frontend queries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformInfo {
    pub os: PlatformKind,
    pub arch: &'static str,
    pub capabilities: PlatformCapabilities,
}

#[cfg(target_os = "windows")]
static ACTIVE_PLATFORM: windows::WindowsPlatform = windows::WindowsPlatform;

#[cfg(not(target_os = "windows"))]
static ACTIVE_PLATFORM: linux::LinuxPlatform = linux::LinuxPlatform;

/// Returns the active [`PlatformBackend`] singleton for the current operating system.
pub fn current() -> &'static dyn PlatformBackend {
    &ACTIVE_PLATFORM
}

/// Returns the [`PlatformCapabilities`] supported by the active platform backend.
pub fn capabilities() -> PlatformCapabilities {
    current().capabilities()
}

/// Returns a serializable [`PlatformInfo`] summary for diagnostics and frontend consumption.
pub fn platform_info() -> PlatformInfo {
    let backend = current();
    PlatformInfo {
        os: backend.platform_kind(),
        arch: std::env::consts::ARCH,
        capabilities: backend.capabilities(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::path::PathBuf;

    #[test]
    fn current_returns_expected_backend_for_target_os() {
        let backend = current();
        #[cfg(target_os = "windows")]
        {
            assert_eq!(backend.platform_kind(), PlatformKind::Windows);
            assert_eq!(backend.platform_kind().as_str(), "windows");
            assert_eq!(backend.platform_kind().to_string(), "windows");
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert_eq!(backend.platform_kind(), PlatformKind::Linux);
            assert_eq!(backend.platform_kind().as_str(), "linux");
        }
        assert_eq!(capabilities(), backend.capabilities());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_platform_exposes_full_capabilities_and_safe_null_handle() {
        let win = windows::WindowsPlatform;
        assert_eq!(win.platform_kind(), PlatformKind::Windows);
        assert_eq!(
            win.capabilities(),
            PlatformCapabilities {
                native_taskbar_hide: true,
                appbar_reserve_space: true,
                dwm_live_thumbnails: true,
                system_audio_mixer: true,
                global_hooks: true,
                media_controls: true,
                brightness_control: true,
            }
        );

        // Null handle (0) must be handled safely without panicking or calling Win32 on NULL.
        win.set_click_through(0, true);
        win.re_assert_topmost(0);
        assert!(!win.is_fullscreen(0));
        assert!(windows::WindowsPlatform::hwnd_from_raw(0).0.is_null());
        assert_eq!(windows::WindowsPlatform::hwnd_from_raw(0x1234).0 as isize, 0x1234);
    }

    #[test]
    fn linux_platform_capabilities_and_safe_noops() {
        let linux_backend = linux::LinuxPlatform;
        assert_eq!(linux_backend.platform_kind(), PlatformKind::Linux);
        assert_eq!(
            linux_backend.capabilities(),
            PlatformCapabilities {
                native_taskbar_hide: false,
                appbar_reserve_space: false,
                dwm_live_thumbnails: false,
                system_audio_mixer: false,
                global_hooks: false,
                media_controls: false,
                brightness_control: false,
            }
        );

        // Safe no-op calls must not panic or claim fullscreen/restore needed.
        linux_backend.set_visible(false, false);
        linux_backend.set_visible(true, true);
        linux_backend.restore_after_crash();
        assert!(!linux_backend.needs_restore());

        linux_backend.set_click_through(42, true);
        linux_backend.re_assert_topmost(42);
        assert!(!linux_backend.is_fullscreen(42));
        assert_eq!(linux_backend.accent_color(), None);
    }

    #[test]
    fn linux_platform_resolves_xdg_data_dir_with_fallback() {
        let xdg = OsStr::new("/tmp/custom-config");
        let home = OsStr::new("/home/alice");

        // XDG_CONFIG_HOME takes precedence over HOME.
        assert_eq!(
            linux::LinuxPlatform::resolve_xdg_data_dir(Some(xdg), Some(home)),
            Some(PathBuf::from("/tmp/custom-config").join("fern"))
        );

        // Empty XDG_CONFIG_HOME falls back to $HOME/.config/fern.
        assert_eq!(
            linux::LinuxPlatform::resolve_xdg_data_dir(Some(OsStr::new("")), Some(home)),
            Some(PathBuf::from("/home/alice").join(".config").join("fern"))
        );

        // Missing XDG_CONFIG_HOME falls back to $HOME/.config/fern.
        assert_eq!(
            linux::LinuxPlatform::resolve_xdg_data_dir(None, Some(home)),
            Some(PathBuf::from("/home/alice").join(".config").join("fern"))
        );

        // Neither set returns None.
        assert_eq!(
            linux::LinuxPlatform::resolve_xdg_data_dir(None, None),
            None
        );
        assert_eq!(
            linux::LinuxPlatform::resolve_xdg_data_dir(Some(OsStr::new("")), Some(OsStr::new(""))),
            None
        );
    }

    #[test]
    fn platform_info_serializes_expected_json_shape() {
        let info = platform_info();
        let json = serde_json::to_value(&info).expect("PlatformInfo should serialize to JSON");

        assert_eq!(json["os"], serde_json::json!(info.os.as_str()));
        assert_eq!(json["arch"], serde_json::json!(std::env::consts::ARCH));
        assert_eq!(
            json["capabilities"]["native_taskbar_hide"],
            serde_json::json!(info.capabilities.native_taskbar_hide)
        );
        assert_eq!(
            json["capabilities"]["appbar_reserve_space"],
            serde_json::json!(info.capabilities.appbar_reserve_space)
        );
        assert_eq!(
            json["capabilities"]["dwm_live_thumbnails"],
            serde_json::json!(info.capabilities.dwm_live_thumbnails)
        );
        assert_eq!(
            json["capabilities"]["system_audio_mixer"],
            serde_json::json!(info.capabilities.system_audio_mixer)
        );
        assert_eq!(
            json["capabilities"]["global_hooks"],
            serde_json::json!(info.capabilities.global_hooks)
        );
        assert_eq!(
            json["capabilities"]["media_controls"],
            serde_json::json!(info.capabilities.media_controls)
        );
        assert_eq!(
            json["capabilities"]["brightness_control"],
            serde_json::json!(info.capabilities.brightness_control)
        );
    }
}
