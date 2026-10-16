//! The clipboard half of the paste fallback: remember what was there, offer the dictated text,
//! and put the old contents back once the app has pasted.

use std::io::Read as _;

use anyhow::{Context as _, Result};
use wl_clipboard_rs::copy::{self, MimeType as CopyMime, Source};
use wl_clipboard_rs::paste::{self, ClipboardType, MimeType, Seat};

/// The clipboard's text before the paste, so it can be restored. `None` when it was empty or
/// held something other than text.
pub fn save() -> Option<Vec<u8>> {
    let (mut pipe, _) =
        paste::get_contents(ClipboardType::Regular, Seat::Unspecified, MimeType::Text).ok()?;
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

/// Offers `text` on the clipboard. The serving happens on a thread wl-clipboard-rs owns, which
/// exits once another selection replaces this one.
pub fn offer(text: &str) -> Result<()> {
    let mut options = copy::Options::new();
    options.clipboard(copy::ClipboardType::Regular);
    options
        .copy(Source::Bytes(text.as_bytes().into()), CopyMime::Text)
        .context("setting the clipboard")
}

/// Puts back what [`save`] read, or clears the dictated text if the clipboard was empty.
pub fn restore(previous: Option<Vec<u8>>) {
    let restored = match previous {
        Some(bytes) => {
            let mut options = copy::Options::new();
            options.clipboard(copy::ClipboardType::Regular);
            options.copy(Source::Bytes(bytes.into()), CopyMime::Text)
        }
        None => copy::clear(copy::ClipboardType::Regular, copy::Seat::All),
    };
    if let Err(error) = restored {
        log::warn!("insert: cannot restore the clipboard: {error}");
    }
}
