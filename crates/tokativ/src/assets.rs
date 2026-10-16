//! The assets GPUI loads: Sonora's icon packs and the Inter faces the UI is set in.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{App, AssetSource, SharedString};

const FONTS: &[&[u8]] = &[
    include_bytes!("../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/Inter-Bold.ttf"),
];

pub struct Assets;

impl Assets {
    pub fn load_fonts(cx: &App) -> Result<()> {
        cx.text_system()
            .add_fonts(FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
    }
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(icons::asset(path).map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        icons::Assets.list(path)
    }
}
