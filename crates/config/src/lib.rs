//! The settings file at `$XDG_CONFIG_HOME/tokativ/config.toml` and the paths the app keeps its
//! data under. The daemon, the settings window and the setup window all read and write it
//! through this crate.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

pub const APP_ID: &str = "tokativ";

/// Which transcription model a session uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EngineKind {
    #[default]
    Parakeet,
    Gigaam,
}

impl EngineKind {
    pub const ALL: [Self; 2] = [Self::Parakeet, Self::Gigaam];

    pub fn id(self) -> &'static str {
        match self {
            Self::Parakeet => "parakeet",
            Self::Gigaam => "gigaam",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Parakeet => "Parakeet TDT 0.6B v3",
            Self::Gigaam => "GigaAM v3",
        }
    }
}

/// The language a session is dictated in. Each mode maps to an engine in [`Engines`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LanguageMode {
    #[default]
    Auto,
    Russian,
}

impl LanguageMode {
    pub const ALL: [Self; 2] = [Self::Auto, Self::Russian];

    /// The short name the CLI and IPC use, `auto` or `ru`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Russian => "ru",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "auto" => Some(Self::Auto),
            "ru" | "russian" => Some(Self::Russian),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Russian => "Russian",
        }
    }
}

/// How dictated text reaches the focused app. `Auto` tries them in the order listed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    #[default]
    Auto,
    InputMethod,
    VirtualKeyboard,
    Clipboard,
}

impl Backend {
    pub const ALL: [Self; 4] = [
        Self::Auto,
        Self::InputMethod,
        Self::VirtualKeyboard,
        Self::Clipboard,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::InputMethod => "Input method",
            Self::VirtualKeyboard => "Virtual keyboard",
            Self::Clipboard => "Clipboard and paste",
        }
    }
}

/// Which edge of the focused output the pill sits on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PillEdge {
    #[default]
    Bottom,
    Top,
}

impl PillEdge {
    pub const ALL: [Self; 2] = [Self::Bottom, Self::Top];

    pub fn name(self) -> &'static str {
        match self {
            Self::Bottom => "Bottom",
            Self::Top => "Top",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// The capture device by its CPAL name. `None` follows the system default.
    pub input_device: Option<String>,
    /// The mode a session starts in until the hotkey or the tray picks another.
    pub language: LanguageMode,
    pub engines: Engines,
    pub vad: Vad,
    pub insertion: Insertion,
    pub pill: Pill,
    pub models: Models,
    pub launch_at_login: bool,
    pub tray: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            input_device: None,
            language: LanguageMode::Auto,
            engines: Engines::default(),
            vad: Vad::default(),
            insertion: Insertion::default(),
            pill: Pill::default(),
            models: Models::default(),
            launch_at_login: false,
            tray: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Engines {
    pub auto: EngineKind,
    pub russian: EngineKind,
}

impl Default for Engines {
    fn default() -> Self {
        Self {
            auto: EngineKind::Parakeet,
            russian: EngineKind::Gigaam,
        }
    }
}

impl Engines {
    pub fn for_mode(&self, mode: LanguageMode) -> EngineKind {
        match mode {
            LanguageMode::Auto => self.auto,
            LanguageMode::Russian => self.russian,
        }
    }
}

/// How speech is cut into the segments that get transcribed one at a time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Vad {
    /// The silence in milliseconds that ends a segment.
    pub silence_ms: u32,
    /// The Silero speech probability above which a frame counts as speech.
    pub threshold: f32,
    /// The longest a segment may run, in seconds, before it is cut mid-speech.
    pub max_segment_s: u32,
    /// The audio in milliseconds kept from before speech starts, so the first syllable is whole.
    pub preroll_ms: u32,
}

impl Default for Vad {
    fn default() -> Self {
        Self {
            silence_ms: 500,
            threshold: 0.5,
            max_segment_s: 20,
            preroll_ms: 200,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Insertion {
    pub backend: Backend,
    /// App ids that paste with Ctrl+Shift+V instead of Ctrl+V.
    pub terminals: Vec<String>,
}

impl Default for Insertion {
    fn default() -> Self {
        Self {
            backend: Backend::Auto,
            terminals: [
                "Alacritty",
                "com.mitchellh.ghostty",
                "foot",
                "footclient",
                "kitty",
                "org.wezfurlong.wezterm",
                "org.gnome.Console",
                "org.gnome.Ptyxis",
                "org.kde.konsole",
                "xterm",
                "rio",
            ]
            .map(String::from)
            .to_vec(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Pill {
    pub edge: PillEdge,
    /// The gap in logical pixels between the pill and the edge of the output.
    pub margin: f32,
}

impl Default for Pill {
    fn default() -> Self {
        Self {
            edge: PillEdge::Bottom,
            margin: 48.,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Models {
    /// Minutes without a session after which loaded models are dropped. Zero keeps them loaded.
    pub unload_after_minutes: u32,
}

impl Config {
    /// Reads the config file. A missing file gives the defaults, a malformed one is an error so
    /// a typo never silently resets the user's settings.
    pub fn load() -> Result<Self> {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Writes the config file through a temporary file and a rename, so the daemon's watcher
    /// never reads a half-written file.
    pub fn save(&self) -> Result<()> {
        let path = config_path();
        let directory = path.parent().context("config path has no parent")?;
        fs::create_dir_all(directory)
            .with_context(|| format!("creating {}", directory.display()))?;
        let text = toml::to_string_pretty(self).context("serializing config")?;
        let temporary = path.with_extension("toml.tmp");
        fs::write(&temporary, text).with_context(|| format!("writing {}", temporary.display()))?;
        fs::rename(&temporary, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home().join(".config"))
        .join(APP_ID)
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Where downloaded models live, one folder per engine.
pub fn models_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| home().join(".local/share"))
        .join(APP_ID)
        .join("models")
}

/// The daemon's control socket. Falls back to the temp dir when there is no runtime dir.
pub fn socket_path() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{APP_ID}.sock"))
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&text).unwrap(), config);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let config: Config =
            toml::from_str("language = \"russian\"\n[vad]\nsilence_ms = 700\n").unwrap();
        assert_eq!(config.language, LanguageMode::Russian);
        assert_eq!(config.vad.silence_ms, 700);
        assert_eq!(config.vad.max_segment_s, 20);
    }
}
