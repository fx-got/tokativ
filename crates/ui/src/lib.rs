//! The windows: the pill overlay, the settings window and the first-run setup window, all drawn
//! with Sonora's `ui` crate.

mod chrome;
// Off Linux there is no compositor to give binding directions for, so most of this goes unused.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod desktop;
mod host;
mod hotkey;
mod models;
pub mod pill;
mod radius;
pub mod settings;
pub mod setup;
mod usage;

use gpui::App;
use ui::{Look, Rounding, Theme, ThemeKind};

pub use host::{Action, Files, Host, Library, ModelState, Status, Store, entities};
pub use pill::{Phase, Pill};
pub use radius::nested_radius;

/// The path the download icon is drawn from. Sonora's packs have none, so the app ships its own.
pub const DOWNLOAD_ICON: &str = "tokativ/download.svg";

/// Builds the theme from the desktop's light or dark preference. Called again before each
/// window opens and whenever the preference changes, since the portal reports it some time
/// after startup.
pub fn refresh_theme(cx: &mut App) {
    Theme::init(
        Look {
            kind: ThemeKind::System,
            rounding: Rounding::Rounded,
            font: 14.,
            transparent: false,
            transparency: 0.,
            blur: false,
            blur_window: false,
            tint: None,
            tint_secondary: None,
        },
        &Default::default(),
        cx,
    );
    cx.refresh_windows();
}
