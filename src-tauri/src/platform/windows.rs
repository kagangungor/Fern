//! Windows platform backend delegating to Fern's existing Win32 modules.

use std::ffi::c_void;
use std::path::PathBuf;
use windows::Win32::Foundation::HWND;

use super::traits::{
    PlatformBackend, PlatformCapabilities, PlatformKind, SystemBackend, TaskbarBackend,
    WindowBackend,
};

/// Win32 implementation of [`PlatformBackend`].
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsPlatform;

impl WindowsPlatform {
    /// Converts an OS-neutral window handle (`isize`) into a Win32 [`HWND`].
    #[inline]
    pub(crate) fn hwnd_from_raw(handle: isize) -> HWND {
        HWND(handle as *mut c_void)
    }
}

impl TaskbarBackend for WindowsPlatform {
    fn set_visible(&self, visible: bool, always_on_top: bool) {
        crate::utils::taskbar::set_taskbar_visibility(visible, always_on_top);
    }

    fn restore_after_crash(&self) {
        crate::utils::taskbar::restore_taskbar_after_crash();
    }

    fn needs_restore(&self) -> bool {
        crate::utils::taskbar::needs_taskbar_restore()
    }
}

impl WindowBackend for WindowsPlatform {
    fn set_click_through(&self, handle: isize, ignore: bool) {
        if handle == 0 {
            return;
        }
        crate::utils::window::set_click_through(Self::hwnd_from_raw(handle), ignore);
    }

    fn re_assert_topmost(&self, handle: isize) {
        if handle == 0 {
            return;
        }
        crate::utils::window::re_assert_topmost(Self::hwnd_from_raw(handle));
    }

    fn is_fullscreen(&self, handle: isize) -> bool {
        if handle == 0 {
            return false;
        }
        crate::utils::window::is_window_fullscreen(Self::hwnd_from_raw(handle))
    }
}

impl SystemBackend for WindowsPlatform {
    fn platform_kind(&self) -> PlatformKind {
        PlatformKind::Windows
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            native_taskbar_hide: true,
            appbar_reserve_space: true,
            dwm_live_thumbnails: true,
            system_audio_mixer: true,
            global_hooks: true,
            media_controls: true,
            brightness_control: true,
        }
    }

    fn accent_color(&self) -> Option<String> {
        crate::commands::get_windows_accent_color()
    }

    fn data_dir(&self) -> Option<PathBuf> {
        crate::agent_api::data_dir()
    }
}

impl PlatformBackend for WindowsPlatform {}
