//! The hotkey Tokativ registers itself on macOS and Windows: recorded by pressing it, stored in
//! the config's syntax and shown the way the platform writes it. Linux leaves the key to the
//! compositor, so only the conversions are built there, for their tests.

#![cfg_attr(target_os = "linux", allow(dead_code))]

use gpui::Keystroke;

/// Keys that can end a hotkey besides letters, digits and punctuation, in GPUI's names. The
/// config stores them the same way, and the hotkey parser reads them back.
const NAMED: &[&str] = &[
    "space",
    "enter",
    "tab",
    "backspace",
    "delete",
    "insert",
    "home",
    "end",
    "pageup",
    "pagedown",
    "up",
    "down",
    "left",
    "right",
];

/// The hotkey a keystroke stands for, such as `control+alt+space`. `None` for a key that cannot
/// be a hotkey, or for one without a modifier, which would take a plain key from every app.
/// Function keys are allowed on their own.
pub fn from_keystroke(keystroke: &Keystroke) -> Option<String> {
    let key = keystroke.key.to_lowercase();
    let function = key
        .strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=24).contains(&number));
    let single = key.chars().count() == 1
        && key
            .chars()
            .all(|key| key.is_ascii_alphanumeric() || "`\\[],=-.';/".contains(key));
    if !(function || single || NAMED.contains(&key.as_str())) {
        return None;
    }
    let modifiers = &keystroke.modifiers;
    let held = [
        (modifiers.control, "control"),
        (modifiers.alt, "alt"),
        (modifiers.shift, "shift"),
        (modifiers.platform, "super"),
    ];
    let mut parts: Vec<&str> = held
        .iter()
        .filter(|(down, _)| *down)
        .map(|(_, name)| *name)
        .collect();
    if parts.is_empty() && !function {
        return None;
    }
    parts.push(&key);
    Some(parts.join("+"))
}

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
                ("pageup", _) => "Page Up",
                ("pagedown", _) => "Page Down",
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

#[cfg(not(target_os = "linux"))]
pub use recorder::Recorder;

#[cfg(not(target_os = "linux"))]
mod recorder {
    use gpui::prelude::*;
    use gpui::{Context, Entity, FocusHandle, KeyDownEvent, Subscription, Window, div};
    use ui::{ActiveTheme as _, Button, Text};

    use crate::host::{Action, Host, Store};

    /// A button showing the hotkey that records a new one when clicked. While it records, the
    /// daemon lets go of the current hotkey so pressing it again reaches this window. Escape or
    /// clicking elsewhere stops without a change.
    pub struct Recorder {
        store: Entity<Store>,
        focus: FocusHandle,
        recording: bool,
        /// Set when a pressed combination cannot be a hotkey, until the next press.
        refused: bool,
        _blur: Option<Subscription>,
    }

    impl Recorder {
        pub fn new(cx: &mut Context<Self>) -> Self {
            let store = Host::global(cx).store;
            cx.observe(&store, |_, _, cx| cx.notify()).detach();
            Self {
                store,
                focus: cx.focus_handle(),
                recording: false,
                refused: false,
                _blur: None,
            }
        }

        fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
            if self.recording {
                return;
            }
            self.recording = true;
            self.refused = false;
            Host::global(cx).dispatch(Action::Capturing(true), cx);
            self.focus.focus(window, cx);
            self._blur = Some(cx.on_blur(&self.focus, window, |this, _, cx| this.stop(cx)));
            cx.notify();
        }

        fn stop(&mut self, cx: &mut Context<Self>) {
            if !self.recording {
                return;
            }
            self.recording = false;
            self.refused = false;
            self._blur = None;
            Host::global(cx).dispatch(Action::Capturing(false), cx);
            cx.notify();
        }

        fn pressed(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
            if !self.recording || event.is_held {
                return;
            }
            cx.stop_propagation();
            let keystroke = &event.keystroke;
            let modifiers = &keystroke.modifiers;
            let bare =
                !(modifiers.control || modifiers.alt || modifiers.shift || modifiers.platform);
            if keystroke.key == "escape" && bare {
                self.stop(cx);
                return;
            }
            match super::from_keystroke(keystroke) {
                Some(hotkey) => {
                    self.store.update(cx, |store, cx| {
                        store.update(|config| config.hotkey = hotkey, cx)
                    });
                    self.stop(cx);
                }
                None => {
                    self.refused = true;
                    cx.notify();
                }
            }
        }
    }

    impl Render for Recorder {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let theme = cx.theme();
            let current = super::label(&self.store.read(cx).config().hotkey);
            let label = match (self.recording, self.refused) {
                (false, _) => current,
                (true, false) => "Press a shortcut".to_string(),
                (true, true) => "Add a modifier".to_string(),
            };
            let button = Button::new("record-hotkey")
                .label(label)
                .small()
                .outline()
                .selected(self.recording)
                .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)));
            div()
                .flex()
                .items_center()
                .gap_2()
                .track_focus(&self.focus)
                .on_key_down(
                    cx.listener(|this, event: &KeyDownEvent, _, cx| this.pressed(event, cx)),
                )
                .when(self.recording, |this| {
                    this.child(
                        div()
                            .text_size(theme.text(Text::Small))
                            .text_color(theme.muted_foreground)
                            .child("Esc to cancel"),
                    )
                })
                .child(button)
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::Modifiers;

    use super::*;

    fn keystroke(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.into(),
            key_char: None,
        }
    }

    #[test]
    fn keystrokes_become_hotkeys() {
        let control_alt = Modifiers {
            control: true,
            alt: true,
            ..Default::default()
        };
        assert_eq!(
            from_keystroke(&keystroke("space", control_alt)).as_deref(),
            Some("control+alt+space")
        );
        let command_shift = Modifiers {
            shift: true,
            platform: true,
            ..Default::default()
        };
        assert_eq!(
            from_keystroke(&keystroke("D", command_shift)).as_deref(),
            Some("shift+super+d")
        );
        assert_eq!(
            from_keystroke(&keystroke("f9", Modifiers::default())).as_deref(),
            Some("f9")
        );
    }

    #[test]
    fn plain_keys_and_modifiers_alone_are_refused() {
        assert_eq!(from_keystroke(&keystroke("a", Modifiers::default())), None);
        let alt = Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(from_keystroke(&keystroke("shift", alt)), None);
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
                assert_eq!(label("alt+shift+pageup"), "Alt+Shift+Page Up");
            }
        }
    }
}
