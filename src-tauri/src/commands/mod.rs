mod agent;
mod audio;
mod dock;
mod icons;
mod lifecycle;
mod media;
mod notch;
mod settings;
mod shelf;
mod system;
mod windows;

pub use agent::*;
pub use audio::*;
pub use dock::*;
pub use icons::*;
pub use lifecycle::*;
pub use media::*;
pub use notch::*;
pub use settings::*;
pub use shelf::*;
pub use system::*;
pub use windows::*;

pub(crate) use icons::is_browser_pwa_aumid;
pub(crate) use notch::{bring_window_to_front, wait_for_splash_with};
pub(crate) use shelf::extract_shelf_text_preview;
pub(crate) use windows::{is_start_menu_window, thumbar_command_wparam};
