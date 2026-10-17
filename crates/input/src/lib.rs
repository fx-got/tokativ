//! Puts dictated text into the focused app. On Wayland three ways are tried in order: the input
//! method protocol, a virtual keyboard that types each character, and the clipboard followed by
//! a paste chord. macOS and Windows have no input method to hook into, so text is typed as
//! keyboard events or pasted.

#[cfg(target_os = "linux")]
mod clipboard;
#[cfg(target_os = "linux")]
mod focus;
#[cfg(target_os = "linux")]
mod keymap;
#[cfg(not(target_os = "linux"))]
mod native;
#[cfg(target_os = "linux")]
mod wayland;

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context as _, Result, anyhow};
pub use tokativ_config::Backend;

#[cfg(target_os = "linux")]
pub use focus::{focused_app_id, focused_output, other_input_method};
/// The worker that serves [`Request`]s on this platform.
#[cfg(not(target_os = "linux"))]
use native as backend;
#[cfg(not(target_os = "linux"))]
pub use native::{focused_app_id, focused_output, other_input_method};
#[cfg(target_os = "linux")]
use wayland as backend;

/// The way text actually reaches apps, after the startup probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    InputMethod,
    VirtualKeyboard,
    Clipboard,
}

impl Method {
    pub fn name(self) -> &'static str {
        match self {
            Self::InputMethod => "Input method",
            Self::VirtualKeyboard => "Virtual keyboard",
            Self::Clipboard => "Clipboard and paste",
        }
    }
}

/// What the probe chose and, when it had to settle for less than asked, why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub method: Method,
    pub note: Option<String>,
}

/// A key the session answers to while it holds the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Escape,
    Enter,
}

/// Runs on the insertion thread when Escape or Enter is pressed during a session.
pub(crate) type KeyHandler = Box<dyn Fn(Key) + Send>;

#[cfg_attr(
    not(target_os = "linux"),
    allow(
        dead_code,
        reason = "only the Wayland worker shows drafts and pastes per app"
    )
)]
pub(crate) enum Request {
    Begin,
    Preedit(String),
    Insert { text: String, preedit: String },
    End,
    Terminals(Vec<String>),
}

/// The handle the daemon talks to. Requests run in order on a worker thread with its own
/// Wayland connection, so a slow paste never holds up the UI.
pub struct Inserter {
    requests: Sender<Request>,
    status: Arc<Mutex<Status>>,
    before: Arc<Mutex<Option<String>>>,
}

impl Inserter {
    /// Connects to the compositor, picks a method starting from `preferred`, and starts the
    /// worker. `terminals` are the app ids that paste with Ctrl+Shift+V. `keys` runs on the worker
    /// thread when Escape or Enter is pressed during a session.
    pub fn start(
        preferred: Backend,
        terminals: Vec<String>,
        keys: impl Fn(Key) + Send + 'static,
    ) -> Result<Self> {
        let (requests, receiver) = mpsc::channel();
        let (ready, probed) = mpsc::sync_channel(1);
        let before = Arc::new(Mutex::new(None));
        let shared = before.clone();
        thread::Builder::new()
            .name("insert".into())
            .spawn(move || {
                backend::run(
                    preferred,
                    terminals,
                    receiver,
                    ready,
                    shared,
                    Box::new(keys),
                )
            })
            .context("spawning the insertion thread")?;
        let status = probed
            .recv()
            .map_err(|_| anyhow!("the insertion thread exited during startup"))??;
        log::info!(
            "insert: using {}{}",
            status.method.name(),
            status
                .note
                .as_deref()
                .map(|note| format!(" ({note})"))
                .unwrap_or_default()
        );
        Ok(Self {
            requests,
            status: Arc::new(Mutex::new(status)),
            before,
        })
    }

    pub fn status(&self) -> Status {
        self.status.lock().expect("status lock").clone()
    }

    /// Starts a dictation session. With the input method this binds it, so the focused text
    /// field can activate it before the first segment is ready, and takes the keyboard so Escape
    /// and Enter reach the session until it ends. Every other key goes on to the focused app.
    pub fn begin(&self) {
        *self.before.lock().expect("before lock") = None;
        self.send(Request::Begin);
    }

    /// Shows `text` at the cursor as unconfirmed text, or clears it when empty. Only the input
    /// method can do this. The other methods ignore it.
    pub fn preedit(&self, text: String) {
        self.send(Request::Preedit(text));
    }

    /// Inserts `text` at the cursor and then shows `preedit` after it, which may be empty.
    pub fn insert(&self, text: String, preedit: String) {
        self.send(Request::Insert { text, preedit });
    }

    /// Ends the session and clears any placeholder.
    pub fn end(&self) {
        self.send(Request::End);
    }

    pub fn set_terminals(&self, terminals: Vec<String>) {
        self.send(Request::Terminals(terminals));
    }

    /// The text left of the cursor in the focused field, when the input method is active and
    /// the app reports it.
    pub fn before_cursor(&self) -> Option<String> {
        self.before.lock().expect("before lock").clone()
    }

    fn send(&self, request: Request) {
        if self.requests.send(request).is_err() {
            log::error!("insert: the insertion thread is gone");
        }
    }
}
