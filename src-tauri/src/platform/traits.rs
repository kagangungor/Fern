//! OS-neutral traits and capability descriptors for Fern's platform abstraction layer.
//!
//! Each supported operating system implements [`PlatformBackend`] (composed of
//! [`TaskbarBackend`], [`WindowBackend`], and [`SystemBackend`]) so that higher-level
//! application code can query capabilities and invoke platform behavior without
//! depending directly on Win32 or Linux-specific APIs.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Identifies the runtime operating system family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlatformKind {
    Windows,
    Linux,
    MacOS,
    Other,
}

impl PlatformKind {
    /// Returns a lowercase string identifier for this platform.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::MacOS => "macos",
            Self::Other => "other",
        }
    }
}

impl fmt::Display for PlatformKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Feature flags describing which OS-integrated capabilities are available on the
/// active platform backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    /// Whether the backend can hide/restore the native OS taskbar/panel.
    pub native_taskbar_hide: bool,
    /// Whether the backend can reserve exclusive desktop work-area space (e.g. Win32 AppBar).
    pub appbar_reserve_space: bool,
    /// Whether the backend supports compositor-accelerated live window thumbnails (e.g. DWM).
    pub dwm_live_thumbnails: bool,
    /// Whether per-application system audio mixing is available.
    pub system_audio_mixer: bool,
    /// Whether global keyboard/mouse/window hooks are supported.
    pub global_hooks: bool,
    /// Whether OS media transport controls (SMTC / MPRIS) are supported.
    pub media_controls: bool,
    /// Whether display brightness control is supported.
    pub brightness_control: bool,
}

/// Controls visibility and crash-recovery of the native OS taskbar or panel.
pub trait TaskbarBackend {
    /// Shows or hides the native OS taskbar.
    ///
    /// When `visible` is `true`, `always_on_top` is used as the fallback taskbar
    /// state if the original pre-hide state could not be captured.
    fn set_visible(&self, visible: bool, always_on_top: bool);

    /// Restores the native taskbar using persisted crash-recovery state, if any.
    fn restore_after_crash(&self);

    /// Returns `true` if the taskbar is currently hidden or has pending restore state.
    fn needs_restore(&self) -> bool;
}

/// Low-level native window operations using an OS-neutral integer handle (`isize`).
pub trait WindowBackend {
    /// Toggles pointer click-through on the native window identified by `handle`.
    fn set_click_through(&self, handle: isize, ignore: bool);

    /// Re-asserts topmost z-order for the window without stealing keyboard focus.
    fn re_assert_topmost(&self, handle: isize);

    /// Returns `true` if the window identified by `handle` is currently fullscreen.
    fn is_fullscreen(&self, handle: isize) -> bool;
}

/// System-level metadata, theming, and filesystem path resolution.
pub trait SystemBackend {
    /// Returns the operating system family represented by this backend.
    fn platform_kind(&self) -> PlatformKind;

    /// Returns the capability matrix supported by this backend.
    fn capabilities(&self) -> PlatformCapabilities;

    /// Returns the active OS accent color as a `#rrggbb` hex string, if available.
    fn accent_color(&self) -> Option<String>;

    /// Returns Fern's per-user configuration/data directory for this OS.
    fn data_dir(&self) -> Option<PathBuf>;
}

/// Super-trait combining all platform subsystems into a single thread-safe backend.
pub trait PlatformBackend: TaskbarBackend + WindowBackend + SystemBackend + Send + Sync {}
