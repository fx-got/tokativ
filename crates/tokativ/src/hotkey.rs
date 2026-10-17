//! Global hotkeys on macOS and Windows, where no compositor runs `tokativ toggle` on a keypress.
//! The toggle hotkey is registered for as long as the daemon runs. Escape and Enter are
//! registered only while a session runs, since they are taken from every other app meanwhile.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{Context as _, Result};
use global_hotkey::hotkey::{Code, HotKey};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use tokativ_input::Key;

/// A hotkey press the daemon acts on.
#[derive(Clone, Copy, Debug)]
pub enum Pressed {
    Toggle,
    Key(Key),
}

/// The registered hotkeys. Made on the main thread, where the platform delivers the presses.
pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    toggle: Option<HotKey>,
    toggle_id: Arc<AtomicU32>,
    escape: HotKey,
    enter: HotKey,
    held: bool,
    /// The toggle is let go while a window records a new one.
    paused: bool,
}

impl Hotkeys {
    /// Registers `toggle` and calls `pressed` for every press of a registered key. A toggle that
    /// cannot be registered is logged and can be fixed later through [`Self::set_toggle`]. The
    /// handler can only be installed once per process, so there is a single `Hotkeys`.
    pub fn new(toggle: &str, pressed: impl Fn(Pressed) + Send + Sync + 'static) -> Result<Self> {
        let manager = GlobalHotKeyManager::new().context("starting the global hotkeys")?;
        let escape = HotKey::new(None, Code::Escape);
        let enter = HotKey::new(None, Code::Enter);
        let toggle_id = Arc::new(AtomicU32::new(0));
        let current = toggle_id.clone();
        let (escape_id, enter_id) = (escape.id(), enter.id());
        GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
            if event.state() != HotKeyState::Pressed {
                return;
            }
            let id = event.id();
            let press = match id {
                _ if id == current.load(Ordering::Relaxed) => Pressed::Toggle,
                _ if id == escape_id => Pressed::Key(Key::Escape),
                _ if id == enter_id => Pressed::Key(Key::Enter),
                _ => return,
            };
            pressed(press);
        }));
        let mut hotkeys = Self {
            manager,
            toggle: None,
            toggle_id,
            escape,
            enter,
            held: false,
            paused: false,
        };
        if let Err(error) = hotkeys.set_toggle(toggle) {
            log::error!("hotkey: {error:#}");
        }
        Ok(hotkeys)
    }

    /// Replaces the toggle hotkey with `text`, such as `alt+space`. The old one stays when
    /// `text` does not parse or another app holds those keys.
    pub fn set_toggle(&mut self, text: &str) -> Result<()> {
        let hotkey: HotKey = text
            .parse()
            .with_context(|| format!("{text:?} is not a hotkey"))?;
        if self.toggle == Some(hotkey) {
            return Ok(());
        }
        if !self.paused {
            self.manager
                .register(hotkey)
                .with_context(|| format!("registering {text}"))?;
        }
        if let Some(old) = self.toggle.replace(hotkey)
            && !self.paused
            && let Err(error) = self.manager.unregister(old)
        {
            log::warn!("hotkey: cannot release {old}: {error}");
        }
        self.toggle_id.store(hotkey.id(), Ordering::Relaxed);
        log::info!("hotkey: {text} toggles dictation");
        Ok(())
    }

    /// Lets go of the toggle while `paused`, so a window recording a new hotkey sees the keys of
    /// the current one, and takes it back after. A toggle set meanwhile is registered then.
    pub fn pause(&mut self, paused: bool) {
        if self.paused == paused {
            return;
        }
        self.paused = paused;
        let Some(toggle) = self.toggle else {
            return;
        };
        let changed = match paused {
            true => self.manager.unregister(toggle),
            false => self.manager.register(toggle),
        };
        if let Err(error) = changed {
            log::warn!("hotkey: cannot change {toggle}: {error}");
        }
    }

    /// Takes Escape and Enter from every app while `held`, and gives them back when not.
    pub fn hold_session_keys(&mut self, held: bool) {
        if self.held == held {
            return;
        }
        let keys = [self.escape, self.enter];
        let changed = match held {
            true => self.manager.register_all(&keys),
            false => self.manager.unregister_all(&keys),
        };
        match changed {
            Ok(()) => self.held = held,
            Err(error) => log::warn!("hotkey: cannot change Escape and Enter: {error}"),
        }
    }
}

impl Drop for Hotkeys {
    fn drop(&mut self) {
        self.hold_session_keys(false);
        if let Some(toggle) = self.toggle.take()
            && !self.paused
        {
            self.manager.unregister(toggle).ok();
        }
    }
}
