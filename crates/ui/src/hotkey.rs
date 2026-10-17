//! The hotkeys Tokativ registers itself on macOS and Windows, as the settings offer them and as
//! people read them. Linux leaves the key to the compositor and never shows these.

/// The hotkeys the settings offer, in the syntax the config stores. The first is the default.
pub const PRESETS: &[&str] = match cfg!(target_os = "macos") {
    true => &[
        "alt+space",
        "control+space",
        "control+alt+space",
        "alt+shift+d",
    ],
    false => &[
        "control+alt+space",
        "control+shift+space",
        "control+alt+d",
        "alt+shift+d",
    ],
};

/// A hotkey as the platform writes it: `⌥ Space` on macOS, `Ctrl+Alt+Space` on Windows.
pub fn label(hotkey: &str) -> String {
    let mac = cfg!(target_os = "macos");
    let keys: Vec<String> = hotkey
        .split('+')
        .map(|key| {
            let named = match (key.to_lowercase().as_str(), mac) {
                ("alt" | "option", true) => "⌥",
                ("alt" | "option", false) => "Alt",
                ("control" | "ctrl", true) => "⌃",
                ("control" | "ctrl", false) => "Ctrl",
                ("shift", true) => "⇧",
                ("shift", false) => "Shift",
                ("command" | "cmd" | "super" | "meta", true) => "⌘",
                ("command" | "cmd" | "super" | "meta", false) => "Win",
                _ => return capitalized(key),
            };
            named.to_owned()
        })
        .collect();
    match mac {
        true => {
            let (key, modifiers) = keys
                .split_last()
                .map_or((String::new(), &[][..]), |(key, rest)| (key.clone(), rest));
            format!("{} {key}", modifiers.concat()).trim().to_owned()
        }
        false => keys.join("+"),
    }
}

fn capitalized(key: &str) -> String {
    let mut letters = key.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_offered_first() {
        assert_eq!(PRESETS[0], tokativ_config::DEFAULT_HOTKEY);
    }

    #[test]
    fn labels_read_as_the_platform_writes_them() {
        match cfg!(target_os = "macos") {
            true => {
                assert_eq!(label("alt+space"), "⌥ Space");
                assert_eq!(label("control+alt+d"), "⌃⌥ D");
            }
            false => {
                assert_eq!(label("control+alt+space"), "Ctrl+Alt+Space");
                assert_eq!(label("alt+shift+d"), "Alt+Shift+D");
            }
        }
    }
}
