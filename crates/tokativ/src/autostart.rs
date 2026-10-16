//! Launch at login through an XDG autostart entry.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use tokativ_config::APP_ID;

fn entry() -> Option<PathBuf> {
    Some(
        dirs::config_dir()?
            .join("autostart")
            .join(format!("{APP_ID}.desktop")),
    )
}

/// Writes or removes the autostart entry so it matches `enabled`.
pub fn apply(enabled: bool) -> Result<()> {
    let path = entry().context("no config directory")?;
    if !enabled {
        return match fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                Err(error).with_context(|| format!("removing {}", path.display()))
            }
            _ => Ok(()),
        };
    }
    let exe = std::env::current_exe().context("finding the running binary")?;
    let text = format!(
        "[Desktop Entry]\nType=Application\nName=Tokativ\nComment=Local dictation\n\
         Exec={} daemon\nTerminal=false\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    );
    fs::create_dir_all(path.parent().context("autostart path has no parent")?)?;
    fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
}
