//! Works out which desktop the user is in, how a keybinding is added there, and whether one for
//! Tokativ already exists, so setup can give directions that fit.

use std::fs;
use std::path::{Path, PathBuf};

/// The compositor or desktop environment of this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    Niri,
    Hyprland,
    Sway,
    River,
    Plasma,
    Gnome,
    Unknown,
}

/// A keybinding for Tokativ found in the desktop's config.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    /// The keys, as the config spells them.
    pub keys: String,
    /// The file the binding is in.
    pub file: PathBuf,
}

impl Desktop {
    /// Reads the session from the variables each compositor sets for its clients.
    pub fn detect() -> Self {
        let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
        if set("NIRI_SOCKET") {
            return Self::Niri;
        }
        if set("HYPRLAND_INSTANCE_SIGNATURE") {
            return Self::Hyprland;
        }
        if set("SWAYSOCK") {
            return Self::Sway;
        }
        let current = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .to_lowercase();
        if current.split(':').any(|part| part == "river") {
            Self::River
        } else if current.contains("kde") {
            Self::Plasma
        } else if current.contains("gnome") {
            Self::Gnome
        } else {
            Self::Unknown
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Niri => "niri",
            Self::Hyprland => "Hyprland",
            Self::Sway => "Sway",
            Self::River => "river",
            Self::Plasma => "KDE Plasma",
            Self::Gnome => "GNOME",
            Self::Unknown => "your desktop",
        }
    }

    /// Whether the pill can show here. GNOME has no layer shell.
    pub fn supported(self) -> bool {
        self != Self::Gnome
    }

    /// The config file a binding goes in, for desktops configured by file.
    pub fn config_file(self) -> Option<PathBuf> {
        let config = dirs::config_dir()?;
        match self {
            Self::Niri => Some(config.join("niri/config.kdl")),
            Self::Hyprland => Some(config.join("hypr/hyprland.conf")),
            Self::Sway => Some(config.join("sway/config")),
            Self::River => Some(config.join("river/init")),
            Self::Plasma | Self::Gnome | Self::Unknown => None,
        }
    }

    /// What to do, in a sentence.
    pub fn directions(self) -> &'static str {
        match self {
            Self::Niri => {
                "Add these lines to the binds section of your niri config. niri picks the change up on save."
            }
            Self::Hyprland => {
                "Add these lines to your Hyprland config. Hyprland reloads it on save."
            }
            Self::Sway => "Add these lines to your Sway config, then reload Sway with Mod+Shift+C.",
            Self::River => {
                "Add these lines to your river init script and run them once, or restart river."
            }
            Self::Plasma => {
                "Open System Settings, then Keyboard, Shortcuts, Add New, Command or Script. Use this command and pick a key."
            }
            Self::Gnome => {
                "GNOME has no layer shell, so the pill cannot show there. Tokativ needs niri, Hyprland, Sway, river or Plasma."
            }
            Self::Unknown => "Bind a key in your compositor to run this command.",
        }
    }

    /// The text to add, in the config's own syntax.
    pub fn snippet(self) -> &'static str {
        match self {
            Self::Niri => {
                "Mod+D { spawn \"tokativ\" \"toggle\"; }"
            }
            Self::Hyprland => {
                "bind = SUPER, D, exec, tokativ toggle"
            }
            Self::Sway => {
                "bindsym $mod+d exec tokativ toggle"
            }
            Self::River => {
                "riverctl map normal Super D spawn 'tokativ toggle'"
            }
            Self::Plasma | Self::Gnome | Self::Unknown => "tokativ toggle",
        }
    }

    /// Looks through the desktop's config files for a line that runs `tokativ toggle`.
    pub fn bound(self) -> Option<Bound> {
        let file = self.config_file()?;
        let folder = file.parent()?.to_path_buf();
        let mut files = vec![file];
        // niri and Sway configs often include sibling files, so those are read as well.
        if let Ok(entries) = fs::read_dir(&folder) {
            let mut siblings: Vec<PathBuf> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_file() && !files.contains(path))
                .collect();
            siblings.sort();
            files.extend(siblings);
        }
        files.into_iter().find_map(|file| {
            let text = fs::read_to_string(&file).ok()?;
            let line = text.lines().find(|line| {
                let line = line.trim_start();
                !line.starts_with("//") && !line.starts_with('#') && runs_toggle(line)
            })?;
            Some(Bound {
                keys: keys(self, line)?,
                file,
            })
        })
    }
}

fn runs_toggle(line: &str) -> bool {
    line.contains("tokativ") && line.contains("toggle")
}

/// The key combination at the start of a binding line, in each config's syntax.
fn keys(desktop: Desktop, line: &str) -> Option<String> {
    let line = line.trim();
    match desktop {
        Desktop::Niri => line.split_whitespace().next().map(str::to_owned),
        Desktop::Sway => line.split_whitespace().nth(1).map(str::to_owned),
        Desktop::Hyprland => {
            let (_, rest) = line.split_once('=')?;
            let mut fields = rest.split(',').map(str::trim);
            let modifiers = fields.next()?;
            let key = fields.next()?;
            Some(match modifiers.is_empty() {
                true => key.to_owned(),
                false => format!("{} + {key}", modifiers.replace(' ', " + ")),
            })
        }
        Desktop::River => {
            let words: Vec<&str> = line.split_whitespace().collect();
            let at = words.iter().position(|word| *word == "map")?;
            Some(format!("{}+{}", words.get(at + 2)?, words.get(at + 3)?))
        }
        Desktop::Plasma | Desktop::Gnome | Desktop::Unknown => None,
    }
}

/// Opens a config file in the user's editor, creating its folder first if needed.
pub fn open(file: &Path) {
    if let Some(folder) = file.parent() {
        fs::create_dir_all(folder).ok();
    }
    if let Err(error) = std::process::Command::new("xdg-open").arg(file).spawn() {
        log::error!("setup: cannot open {}: {error}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_come_out_of_each_syntax() {
        assert_eq!(
            keys(
                Desktop::Niri,
                "\tMod+D hotkey-overlay-title=\"x\" { spawn \"tokativ\" \"toggle\"; }"
            )
            .as_deref(),
            Some("Mod+D")
        );
        assert_eq!(
            keys(
                Desktop::Hyprland,
                "bind = SUPER SHIFT, D, exec, tokativ toggle"
            )
            .as_deref(),
            Some("SUPER + SHIFT + D")
        );
        assert_eq!(
            keys(Desktop::Sway, "bindsym $mod+d exec tokativ toggle").as_deref(),
            Some("$mod+d")
        );
        assert_eq!(
            keys(
                Desktop::River,
                "riverctl map normal Super D spawn 'tokativ toggle'"
            )
            .as_deref(),
            Some("Super+D")
        );
    }
}
