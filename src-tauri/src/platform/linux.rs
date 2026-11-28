//! Linux platform backend (XDG-compliant paths and safe no-op stubs).
//!
//! Compiled on all targets so that non-Windows builds have a safe fallback and
//! unit tests can verify Linux behavior on Windows developer machines.

use std::ffi::OsStr;
use std::path::PathBuf;

use super::traits::{
    PlatformBackend, PlatformCapabilities, PlatformKind, SystemBackend, TaskbarBackend,
    WindowBackend,
};

/// Linux implementation of [`PlatformBackend`].
///
/// Desktop-environment integrations (X11 EWMH struts, Wayland `wlr-layer-shell`,
/// PipeWire audio/thumbnails, MPRIS media controls, sysfs/DDC-CI brightness) are
/// intentionally stubbed as safe no-ops until their respective Linux backends land.
#[derive(Debug, Default, Clone, Copy)]
pub struct LinuxPlatform;

impl LinuxPlatform {
    /// Resolves Fern's XDG-compliant config/data directory from explicit env values.
    ///
    /// Follows the XDG Base Directory Specification:
    /// 1. `$XDG_CONFIG_HOME/fern` when `XDG_CONFIG_HOME` is set and non-empty.
    /// 2. `$HOME/.config/fern` when `HOME` is set and non-empty.
    /// 3. `None` otherwise.
    pub(crate) fn resolve_xdg_data_dir(
        xdg_config_home: Option<&OsStr>,
        home: Option<&OsStr>,
    ) -> Option<PathBuf> {
        if let Some(xdg) = xdg_config_home.filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(xdg).join("fern"));
        }
        if let Some(home_dir) = home.filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(home_dir).join(".config").join("fern"));
        }
        None
    }
}

impl TaskbarBackend for LinuxPlatform {
    fn set_visible(&self, _visible: bool, _always_on_top: bool) {}

    fn restore_after_crash(&self) {}

    fn needs_restore(&self) -> bool {
        false
    }
}

impl WindowBackend for LinuxPlatform {
    fn set_click_through(&self, _handle: isize, _ignore: bool) {}

    fn re_assert_topmost(&self, _handle: isize) {}

    fn is_fullscreen(&self, _handle: isize) -> bool {
        false
    }
}

impl SystemBackend for LinuxPlatform {
    fn platform_kind(&self) -> PlatformKind {
        PlatformKind::Linux
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            native_taskbar_hide: false,
            appbar_reserve_space: false,
            dwm_live_thumbnails: false,
            system_audio_mixer: false,
            global_hooks: false,
            media_controls: false,
            brightness_control: false,
        }
    }

    fn accent_color(&self) -> Option<String> {
        None
    }

    fn data_dir(&self) -> Option<PathBuf> {
        let xdg = std::env::var_os("XDG_CONFIG_HOME");
        let home = std::env::var_os("HOME");
        Self::resolve_xdg_data_dir(xdg.as_deref(), home.as_deref())
    }
}

impl PlatformBackend for LinuxPlatform {}
