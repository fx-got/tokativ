//! The windows: the pill overlay, the settings window and the first-run setup window, all drawn
//! with Sonora's `ui` crate.

mod chrome;
mod host;
pub mod pill;
mod radius;
pub mod settings;
pub mod setup;

use gpui::App;
use ui::{Look, Rounding, Theme, ThemeKind};

pub use host::{Action, Host, Library, ModelState, Status, Store, entities};
pub use pill::{Phase, Pill};
pub use radius::nested_radius;

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
