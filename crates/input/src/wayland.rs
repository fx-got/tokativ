//! The insertion worker: one Wayland connection that drives the input method and the virtual
//! keyboard on behalf of [`crate::Inserter`].

use std::io::{ErrorKind, Write as _};
use std::os::fd::AsFd as _;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use rustix::event::{PollFd, PollFlags, Timespec};
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_keyboard::KeymapFormat;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2;
use wayland_protocols_misc::zwp_input_method_v2::client::zwp_input_method_v2::{
    self, ZwpInputMethodV2,
};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

use crate::keymap::{self, KEY_LEFTCTRL, KEY_LEFTSHIFT, KEY_V, MOD_CTRL, MOD_SHIFT};
use crate::{Backend, Method, Request, Status, clipboard, focus};

/// What the placeholder preedit shows while a segment is transcribed.
const PLACEHOLDER: &str = "…";
/// The input method protocol rejects longer strings.
const MAX_STRING: usize = 4000;
/// How long the first insertion of a session waits for the focused field to activate the input
/// method before typing instead. Apps answer the compositor's enter late, often near a second.
const ACTIVATION: Duration = Duration::from_millis(1500);
/// The gap between virtual key events, as wtype leaves it. Some clients drop keys sent faster.
const KEY_GAP: Duration = Duration::from_millis(2);
/// The most distinct characters one virtual keymap carries before a fresh one is uploaded.
const KEYMAP_CHARS: usize = 64;
/// How long the app gets to read the clipboard after the paste chord before it is restored.
const PASTE_SETTLE: Duration = Duration::from_millis(400);
const TICK: Duration = Duration::from_millis(10);

/// The protocol state the dispatch callbacks write into.
struct State {
    pending_active: bool,
    active: bool,
    /// How many `done` events the current input method has seen, which is the serial every
    /// commit must carry.
    serial: u32,
    unavailable: bool,
    before: Arc<Mutex<Option<String>>>,
}

struct Worker {
    connection: Connection,
    queue: EventQueue<State>,
    handle: QueueHandle<State>,
    state: State,
    seat: WlSeat,
    input_methods: Option<ZwpInputMethodManagerV2>,
    keyboards: Option<ZwpVirtualKeyboardManagerV1>,
    method: Method,
    terminals: Vec<String>,
    input_method: Option<ZwpInputMethodV2>,
    /// Whether this session already waited out [`ACTIVATION`] once.
    waited: bool,
    started: Instant,
}

pub(crate) fn run(
    preferred: Backend,
    terminals: Vec<String>,
    requests: Receiver<Request>,
    ready: SyncSender<Result<Status>>,
    before: Arc<Mutex<Option<String>>>,
) {
    let mut worker = match Worker::connect(preferred, terminals, before) {
        Ok((worker, status)) => {
            ready.send(Ok(status)).ok();
            worker
        }
        Err(error) => {
            ready.send(Err(error)).ok();
            return;
        }
    };
    loop {
        let request = match requests.recv_timeout(TICK) {
            Ok(request) => Some(request),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let handled = match request {
            Some(request) => worker.handle(request),
            None => worker.pump(Duration::ZERO),
        };
        if let Err(error) = handled {
            log::error!("insert: {error:#}");
            if worker.connection.protocol_error().is_some() {
                return;
            }
        }
    }
}

impl Worker {
    fn connect(
        preferred: Backend,
        terminals: Vec<String>,
        before: Arc<Mutex<Option<String>>>,
    ) -> Result<(Self, Status)> {
        let connection = Connection::connect_to_env().context("connecting to Wayland")?;
        let (globals, queue) =
            registry_queue_init::<State>(&connection).context("listing Wayland globals")?;
        let handle = queue.handle();
        let seat: WlSeat = globals
            .bind(&handle, 1..=7, ())
            .context("the compositor has no seat")?;
        let input_methods = bind::<ZwpInputMethodManagerV2>(&globals, &handle);
        let keyboards = bind::<ZwpVirtualKeyboardManagerV1>(&globals, &handle);
        let status = choose(
            preferred,
            input_methods.is_some(),
            keyboards.is_some(),
            focus::other_input_method(),
        );
        let worker = Self {
            connection,
            queue,
            handle,
            state: State {
                pending_active: false,
                active: false,
                serial: 0,
                unavailable: false,
                before,
            },
            seat,
            input_methods,
            keyboards,
            method: status.method,
            terminals,
            input_method: None,
            waited: false,
            started: Instant::now(),
        };
        Ok((worker, status))
    }

    fn handle(&mut self, request: Request) -> Result<()> {
        match request {
            Request::Begin => self.begin(),
            Request::Placeholder => self.placeholder(),
            Request::Insert { text, more } => self.insert(&text, more),
            Request::End => self.end(),
            Request::Terminals(terminals) => {
                self.terminals = terminals;
                Ok(())
            }
        }
    }

    fn begin(&mut self) -> Result<()> {
        self.end()?;
        self.waited = false;
        if self.method != Method::InputMethod {
            return Ok(());
        }
        let Some(manager) = &self.input_methods else {
            return Ok(());
        };
        self.state.pending_active = false;
        self.state.active = false;
        self.state.serial = 0;
        self.state.unavailable = false;
        self.input_method = Some(manager.get_input_method(&self.seat, &self.handle, ()));
        self.pump(Duration::ZERO)
    }

    /// The input method, when it is bound, usable and active in a text field. The first call in
    /// a session waits for activation, since it usually arrives after the session starts.
    fn active_input_method(&mut self) -> Result<Option<ZwpInputMethodV2>> {
        if self.input_method.is_none() {
            return Ok(None);
        }
        if !self.state.active && !self.waited {
            self.waited = true;
            let deadline = Instant::now() + ACTIVATION;
            while !self.state.active && !self.state.unavailable && Instant::now() < deadline {
                self.pump(TICK)?;
            }
        }
        if self.state.unavailable {
            log::warn!("insert: another input method holds the seat, typing instead");
            if let Some(input_method) = self.input_method.take() {
                input_method.destroy();
            }
            return Ok(None);
        }
        Ok(self
            .state
            .active
            .then(|| self.input_method.clone())
            .flatten())
    }

    fn placeholder(&mut self) -> Result<()> {
        if !self.state.active {
            return Ok(());
        }
        if let Some(input_method) = &self.input_method {
            input_method.set_preedit_string(PLACEHOLDER.into(), -1, -1);
            input_method.commit(self.state.serial);
        }
        self.pump(Duration::ZERO)
    }

    fn insert(&mut self, text: &str, more: bool) -> Result<()> {
        if let Some(input_method) = self.active_input_method()? {
            log::debug!("insert: {} bytes through the input method", text.len());
            for chunk in chunks(text, MAX_STRING) {
                input_method.commit_string(chunk.into());
            }
            if more {
                input_method.set_preedit_string(PLACEHOLDER.into(), -1, -1);
            }
            input_method.commit(self.state.serial);
            return self.pump(Duration::ZERO);
        }
        log::debug!("insert: {} bytes, no active input method", text.len());
        match (self.method, self.keyboards.is_some()) {
            (Method::Clipboard, true) => self.paste(text),
            (_, true) => self.type_text(text),
            (_, false) => {
                clipboard::offer(text)?;
                log::warn!("insert: no way to send keys, the text is on the clipboard");
                Ok(())
            }
        }
    }

    fn end(&mut self) -> Result<()> {
        let Some(input_method) = self.input_method.take() else {
            return Ok(());
        };
        if self.state.active {
            input_method.set_preedit_string(String::new(), 0, 0);
            input_method.commit(self.state.serial);
        }
        input_method.destroy();
        self.state.active = false;
        *self.state.before.lock().expect("before lock") = None;
        self.queue
            .roundtrip(&mut self.state)
            .context("ending the input method")?;
        Ok(())
    }

    /// Types `text` through a fresh virtual keyboard whose keymap holds exactly the characters
    /// needed. The keyboard is destroyed afterwards so the compositor restores the real keymap.
    fn type_text(&mut self, text: &str) -> Result<()> {
        let keyboard = self.virtual_keyboard()?;
        let characters: Vec<char> = text.chars().collect();
        let mut start = 0;
        while start < characters.len() {
            let mut distinct: Vec<char> = Vec::new();
            let mut end = start;
            while end < characters.len() {
                let c = characters[end];
                if !distinct.contains(&c) {
                    if distinct.len() == KEYMAP_CHARS {
                        break;
                    }
                    distinct.push(c);
                }
                end += 1;
            }
            let chunk: String = characters[start..end].iter().collect();
            let typing = keymap::typing(&chunk);
            self.upload(&keyboard, &typing.text)?;
            for code in typing.codes {
                self.key(&keyboard, code, true)?;
                self.key(&keyboard, code, false)?;
            }
            start = end;
        }
        keyboard.destroy();
        self.queue
            .roundtrip(&mut self.state)
            .context("finishing typing")?;
        Ok(())
    }

    /// Offers `text` on the clipboard, sends the paste chord, and restores the clipboard.
    fn paste(&mut self, text: &str) -> Result<()> {
        let terminal = focus::focused_app_id()
            .is_some_and(|app| self.terminals.iter().any(|known| known == &app));
        let previous = clipboard::save();
        clipboard::offer(text)?;
        let keyboard = self.virtual_keyboard()?;
        self.upload(&keyboard, &keymap::paste())?;
        let mut mods = MOD_CTRL;
        self.key(&keyboard, KEY_LEFTCTRL, true)?;
        if terminal {
            mods |= MOD_SHIFT;
            self.key(&keyboard, KEY_LEFTSHIFT, true)?;
        }
        keyboard.modifiers(mods, 0, 0, 0);
        self.key(&keyboard, KEY_V, true)?;
        self.key(&keyboard, KEY_V, false)?;
        keyboard.modifiers(0, 0, 0, 0);
        if terminal {
            self.key(&keyboard, KEY_LEFTSHIFT, false)?;
        }
        self.key(&keyboard, KEY_LEFTCTRL, false)?;
        keyboard.destroy();
        self.queue
            .roundtrip(&mut self.state)
            .context("sending the paste chord")?;
        thread::sleep(PASTE_SETTLE);
        clipboard::restore(previous);
        Ok(())
    }

    fn virtual_keyboard(&self) -> Result<ZwpVirtualKeyboardV1> {
        let manager = self
            .keyboards
            .as_ref()
            .ok_or_else(|| anyhow!("the compositor has no virtual keyboard"))?;
        Ok(manager.create_virtual_keyboard(&self.seat, &self.handle, ()))
    }

    fn upload(&mut self, keyboard: &ZwpVirtualKeyboardV1, keymap: &str) -> Result<()> {
        let fd = rustix::fs::memfd_create("tokativ-keymap", rustix::fs::MemfdFlags::CLOEXEC)
            .context("creating the keymap file")?;
        let mut file = std::fs::File::from(fd);
        file.write_all(keymap.as_bytes())?;
        file.write_all(&[0])?;
        keyboard.keymap(
            KeymapFormat::XkbV1.into(),
            file.as_fd(),
            keymap.len() as u32 + 1,
        );
        self.queue
            .roundtrip(&mut self.state)
            .context("uploading the keymap")?;
        Ok(())
    }

    fn key(&mut self, keyboard: &ZwpVirtualKeyboardV1, code: u32, pressed: bool) -> Result<()> {
        let time = self.started.elapsed().as_millis() as u32;
        keyboard.key(time, code, pressed as u32);
        self.queue
            .roundtrip(&mut self.state)
            .context("sending a key")?;
        thread::sleep(KEY_GAP);
        Ok(())
    }

    /// Sends what is queued and handles whatever the compositor has sent, waiting up to
    /// `timeout` for something to arrive.
    fn pump(&mut self, timeout: Duration) -> Result<()> {
        self.queue.dispatch_pending(&mut self.state)?;
        self.connection.flush()?;
        if let Some(guard) = self.queue.prepare_read() {
            let readable = {
                let fd = guard.connection_fd();
                let mut fds = [PollFd::new(&fd, PollFlags::IN)];
                let timeout = Timespec::try_from(timeout).unwrap_or_default();
                rustix::event::poll(&mut fds, Some(&timeout)).context("polling Wayland")? > 0
            };
            if readable {
                match guard.read() {
                    Ok(_) => {}
                    Err(wayland_client::backend::WaylandError::Io(error))
                        if error.kind() == ErrorKind::WouldBlock => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        self.queue.dispatch_pending(&mut self.state)?;
        Ok(())
    }
}

/// Picks the method for `preferred`, stepping down the chain when a protocol is missing or the
/// input method belongs to someone else.
fn choose(
    preferred: Backend,
    input_method: bool,
    virtual_keyboard: bool,
    other: Option<String>,
) -> Status {
    let input_method_note = match (input_method, &other) {
        (false, _) => Some("the compositor has no input method protocol".to_string()),
        (true, Some(other)) => Some(format!("{other} already owns the input method")),
        (true, None) => None,
    };
    let keyboard_note =
        (!virtual_keyboard).then(|| "the compositor has no virtual keyboard protocol".to_string());
    let chain: &[Method] = match preferred {
        Backend::Auto | Backend::InputMethod => &[
            Method::InputMethod,
            Method::VirtualKeyboard,
            Method::Clipboard,
        ],
        Backend::VirtualKeyboard => &[Method::VirtualKeyboard, Method::Clipboard],
        Backend::Clipboard => &[Method::Clipboard],
    };
    let mut notes = Vec::new();
    for method in chain {
        let missing = match method {
            Method::InputMethod => &input_method_note,
            Method::VirtualKeyboard => &keyboard_note,
            Method::Clipboard => &None,
        };
        match missing {
            None => {
                return Status {
                    method: *method,
                    note: (!notes.is_empty()).then(|| notes.join(", ")),
                };
            }
            Some(note) => notes.push(note.clone()),
        }
    }
    unreachable!("the clipboard is always available")
}

/// Splits `text` into pieces of at most `limit` bytes on character boundaries.
fn chunks(text: &str, limit: usize) -> impl Iterator<Item = &str> {
    let mut rest = text;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let mut end = rest.len().min(limit);
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let (chunk, tail) = rest.split_at(end);
        rest = tail;
        Some(chunk)
    })
}

fn bind<I>(globals: &GlobalList, handle: &QueueHandle<State>) -> Option<I>
where
    I: Proxy + 'static,
    State: Dispatch<I, ()>,
{
    globals.bind::<I, State, ()>(handle, 1..=1, ()).ok()
}

impl Dispatch<ZwpInputMethodV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodV2,
        event: zwp_input_method_v2::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwp_input_method_v2::Event::Activate => state.pending_active = true,
            zwp_input_method_v2::Event::Deactivate => state.pending_active = false,
            zwp_input_method_v2::Event::SurroundingText { text, cursor, .. } => {
                let cursor = (cursor as usize).min(text.len());
                let before = text.get(..cursor).map(str::to_owned);
                *state.before.lock().expect("before lock") = before;
            }
            zwp_input_method_v2::Event::Done => {
                state.serial = state.serial.wrapping_add(1);
                state.active = state.pending_active;
            }
            zwp_input_method_v2::Event::Unavailable => state.unavailable = true,
            _ => {}
        }
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(State: ignore WlSeat);
wayland_client::delegate_noop!(State: ignore ZwpInputMethodManagerV2);
wayland_client::delegate_noop!(State: ignore ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(State: ignore ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_respect_character_boundaries() {
        let text = "приветhello";
        let pieces: Vec<&str> = chunks(text, 5).collect();
        assert_eq!(pieces.concat(), text);
        assert!(pieces.iter().all(|piece| piece.len() <= 5));
    }

    #[test]
    fn chain_steps_down_with_reasons() {
        let status = choose(Backend::Auto, true, true, Some("fcitx5".into()));
        assert_eq!(status.method, Method::VirtualKeyboard);
        assert_eq!(
            status.note.as_deref(),
            Some("fcitx5 already owns the input method")
        );
        let status = choose(Backend::Auto, true, true, None);
        assert_eq!(
            status,
            Status {
                method: Method::InputMethod,
                note: None
            }
        );
        assert_eq!(
            choose(Backend::VirtualKeyboard, false, false, None).method,
            Method::Clipboard
        );
    }
}
