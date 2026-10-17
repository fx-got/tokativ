//! The background process: it owns the GPUI app, answers the control socket and the tray, and
//! runs one dictation session at a time.

use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use gpui::{App, AppContext as _, AsyncApp, Context, Entity, KeyBinding, QuitMode, WindowHandle};
use notify::{RecursiveMode, Watcher as _};
use tokativ_audio::{Levels, Options, Recorder, Silero, SpeechDetector, Timing};
use tokativ_config::{Capitals, Config, config_dir, config_path, socket_path};
use tokativ_engine::{Joiner, ModelId, Transcriber, restyle};
use tokativ_input::{Inserter, Key, Method};
use tokativ_ui::{Action, Host, Library, Phase, Pill, Status, Store};

use crate::assets::Assets;
use crate::autostart;
use crate::ipc::{self, Command};
use crate::tray::{Shown, Tray, TrayEvent};

/// What sits at the cursor while a segment is transcribed and no draft of it is ready.
const PLACEHOLDER: &str = "…";
/// How much new speech triggers another draft transcript.
const DRAFT_EVERY: Duration = Duration::from_millis(600);
/// How often the daemon looks whether a model finished loading on some device.
const DEVICE_POLL: Duration = Duration::from_secs(2);

/// Everything that reaches the daemon from another thread.
enum Event {
    Command(Command),
    Tray(TrayEvent),
    Recorder {
        session: u64,
        event: tokativ_audio::Event,
    },
    Transcribed {
        session: u64,
        result: Result<String>,
    },
    Drafted {
        session: u64,
        segment: usize,
        result: Result<String>,
    },
    Key(Key),
    ConfigFile,
}

/// The pill window and where it was put, which decides whether the next session can reuse it.
struct Placed {
    handle: WindowHandle<Pill>,
    output: Option<String>,
    pill: tokativ_config::Pill,
}

impl Placed {
    fn fits(&self, output: &Option<String>, config: &Config) -> bool {
        &self.output == output && self.pill == config.pill
    }
}

/// One press of the hotkey to the pill closing.
struct Session {
    id: u64,
    engine: ModelId,
    /// The microphone, once it has opened.
    recorder: Option<Recorder>,
    /// Created at the first insertion, once the input method may have reported the text
    /// already left of the cursor.
    joiner: Option<Joiner>,
    /// Segments handed to the transcriber whose text has not come back yet.
    pending: usize,
    /// How many segments the recorder has finished, which is also the index of the one being
    /// spoken now.
    spoken: usize,
    /// How many segments have come back and been inserted.
    inserted: usize,
    /// The draft shown at the cursor and the segment it belongs to.
    draft: Option<(usize, String)>,
    /// The recorder may still deliver segments.
    capturing: bool,
    /// Confirm was pressed, so the session ends once the last segment is inserted.
    stopping: bool,
    /// The hotkey was pressed again while finishing, so a new session follows in the same pill.
    restart: bool,
    /// Speech was heard, so confirming has something to wait for.
    heard: bool,
}

pub struct Daemon {
    store: Entity<Store>,
    library: Entity<Library>,
    status: Entity<Status>,
    applied: Config,
    events: UnboundedSender<Event>,
    transcriber: Transcriber,
    inserter: Option<Inserter>,
    session: Option<Session>,
    next_session: u64,
    /// The pill window, kept while it is on screen so a new session can unfold it again
    /// instead of opening another.
    pill: Option<Placed>,
    tray: Option<Tray>,
    tray_events: UnboundedSender<TrayEvent>,
    _watcher: Option<notify::RecommendedWatcher>,
}

/// Starts the daemon and blocks until it quits.
pub fn run(config: Config) -> Result<()> {
    // A second start is not a failure, so a service manager does not keep retrying it.
    if ipc::send(&socket_path(), &Command::Ping).is_ok() {
        log::info!("tokativ is already running");
        return Ok(());
    }
    let (events, receiver) = unbounded();
    let commands = events.clone();
    ipc::serve(&socket_path(), move |command| {
        commands.unbounded_send(Event::Command(command)).ok();
    })?;

    let application = gpui_platform::application().with_assets(Assets);
    application.run(move |cx: &mut App| {
        cx.set_quit_mode(QuitMode::Explicit);
        if let Err(error) = Assets::load_fonts(cx) {
            log::error!("cannot load the UI fonts: {error:#}");
        }
        tokativ_ui::refresh_theme(cx);
        cx.bind_keys(bindings());

        let daemon = match Daemon::new(config, events, cx) {
            Ok(daemon) => daemon,
            Err(error) => {
                log::error!("{error:#}");
                cx.quit();
                return;
            }
        };
        listen(daemon, receiver, cx);
    });
    std::fs::remove_file(socket_path()).ok();
    Ok(())
}

fn listen(daemon: Entity<Daemon>, mut receiver: UnboundedReceiver<Event>, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        while let Some(event) = receiver.next().await {
            daemon.update(cx, |daemon, cx| daemon.handle(event, cx));
        }
    })
    .detach();
}

impl Daemon {
    fn new(config: Config, events: UnboundedSender<Event>, cx: &mut App) -> Result<Entity<Self>> {
        let (store, library, status) = tokativ_ui::entities(config.clone(), cx);
        let transcriber = Transcriber::start(idle_limit(&config), config.processor)?;
        let inserter = start_inserter(&config, &events);

        let (tray_events, mut tray_receiver) = unbounded();
        let forwarded = events.clone();
        cx.background_spawn(async move {
            while let Some(event) = tray_receiver.next().await {
                forwarded.unbounded_send(Event::Tray(event)).ok();
            }
        })
        .detach();

        let watched = events.clone();
        let watcher = watch_config(move || {
            watched.unbounded_send(Event::ConfigFile).ok();
        })
        .map_err(|error| log::warn!("config: not watching for edits: {error:#}"))
        .ok();

        let daemon = cx.new(|cx| {
            let mut daemon = Self {
                store: store.clone(),
                library: library.clone(),
                status,
                applied: config.clone(),
                events,
                transcriber,
                inserter,
                session: None,
                next_session: 0,
                pill: None,
                tray: None,
                tray_events,
                _watcher: watcher,
            };
            cx.observe(
                &store,
                |this: &mut Self, store: Entity<Store>, cx: &mut Context<Self>| {
                    let config = store.read(cx).config().clone();
                    this.apply(config, cx);
                },
            )
            .detach();
            cx.observe(&library, |this: &mut Self, _, cx| this.publish(cx))
                .detach();
            daemon.place_tray(config.tray, cx);
            // Models load on the worker thread, so the device they landed on is checked for
            // rather than announced.
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(DEVICE_POLL).await;
                    let alive = this.update(cx, |this, cx| {
                        if this.status.read(cx).device != this.transcriber.device() {
                            this.publish(cx);
                        }
                    });
                    if alive.is_err() {
                        return;
                    }
                }
            })
            .detach();
            daemon.publish(cx);
            daemon.preload(cx);
            daemon
        });

        let weak = daemon.downgrade();
        Host::install(
            store,
            library.clone(),
            daemon.read(cx).status.clone(),
            move |action, cx| {
                weak.update(cx, |daemon, cx| daemon.act(action, cx)).ok();
            },
            cx,
        );

        // The pill waits folded from the start, so even the first session shows it at once.
        daemon.update(cx, |daemon, cx| {
            let config = daemon.store.read(cx).config().clone();
            let output = tokativ_input::focused_output();
            daemon.pill = daemon.open_pill(Levels::default(), &config, output, true, cx);
        });

        let first_run = !config.setup_done || !library.read(cx).any_installed();
        if first_run {
            daemon.update(cx, |daemon, cx| daemon.open_setup(cx));
        }
        log::info!("tokativ is listening on {}", socket_path().display());
        Ok(daemon)
    }

    fn handle(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Command(command) => self.command(command, cx),
            Event::Tray(TrayEvent::Toggle) => self.toggle(cx),
            Event::Tray(TrayEvent::Model(model)) => self.set_model(model, cx),
            Event::Tray(TrayEvent::Settings) => self.open_settings(cx),
            Event::Tray(TrayEvent::Quit) => self.quit(cx),
            Event::Recorder { session, event } => self.recorded(session, event, cx),
            Event::Transcribed { session, result } => self.transcribed(session, result, cx),
            Event::Drafted {
                session,
                segment,
                result,
            } => self.drafted(session, segment, result),
            Event::Key(Key::Escape) => self.cancel(cx),
            Event::Key(Key::Enter) => self.confirm(cx),
            Event::ConfigFile => match Config::load() {
                Ok(config) => self.store.update(cx, |store, cx| store.replace(config, cx)),
                Err(error) => log::warn!("config: keeping the running settings: {error:#}"),
            },
        }
    }

    fn command(&mut self, command: Command, cx: &mut Context<Self>) {
        log::debug!("command: {command:?}");
        match command {
            Command::Toggle => self.toggle(cx),
            Command::Start => self.start(cx),
            Command::Stop | Command::Confirm => self.confirm(cx),
            Command::Cancel => self.cancel(cx),
            Command::Model(model) => self.set_model(model, cx),
            Command::Settings => self.open_settings(cx),
            Command::Quit => self.quit(cx),
            Command::Ping => {}
        }
    }

    fn act(&mut self, action: Action, cx: &mut Context<Self>) {
        match action {
            Action::Toggle => self.toggle(cx),
            Action::Confirm => self.confirm(cx),
            Action::Cancel => self.cancel(cx),
            Action::Settings => self.open_settings(cx),
            Action::Quit => self.quit(cx),
        }
    }

    /// Starts a session, confirms the one running, or, while one is finishing, asks for a new
    /// one to follow it so the pill goes straight back to listening.
    fn toggle(&mut self, cx: &mut Context<Self>) {
        match &mut self.session {
            None => self.start(cx),
            Some(session) if !session.stopping => self.confirm(cx),
            Some(session) => {
                session.restart = !session.restart;
                let phase = match session.restart {
                    true => Phase::Listening,
                    false => Phase::Processing,
                };
                if let Some(pill) = self.pill.as_ref().map(|placed| placed.handle) {
                    set_phase(pill, phase, cx);
                }
            }
        }
    }

    /// Shows the pill at once and opens the microphone in the background, since a device can
    /// take a few hundred milliseconds to open. Speech before it opens is not heard.
    fn start(&mut self, cx: &mut Context<Self>) {
        if self.session.is_some() {
            return;
        }
        let config = self.store.read(cx).config().clone();
        let Some(engine) =
            ModelId::parse(&config.model).filter(|engine| self.library.read(cx).installed(engine))
        else {
            log::warn!("session: {} is not downloaded", config.model);
            self.open_setup(cx);
            return;
        };

        self.next_session += 1;
        let id = self.next_session;
        let levels = Levels::default();
        self.show_pill(levels.clone(), &config, cx);
        self.transcriber.preload(engine.clone());
        if let Some(inserter) = &self.inserter {
            inserter.begin();
        }
        log::info!("session {id}: listening with {}", engine.name());
        self.session = Some(Session {
            id,
            engine,
            recorder: None,
            joiner: None,
            pending: 0,
            spoken: 0,
            inserted: 0,
            draft: None,
            capturing: true,
            stopping: false,
            restart: false,
            heard: false,
        });
        self.publish(cx);

        // Drafts show as preedit text, which only the input method can put at the cursor.
        let drafts = config.live_preview
            && self
                .inserter
                .as_ref()
                .is_some_and(|inserter| inserter.status().method == Method::InputMethod);
        let options = Options {
            device: config.input_device.clone(),
            timing: Timing {
                threshold: config.vad.threshold,
                silence_ms: config.vad.silence_ms,
                max_segment_s: config.vad.max_segment_s,
                preroll_ms: config.vad.preroll_ms,
            },
            progress: drafts.then_some(DRAFT_EVERY),
        };
        let events = self.events.clone();
        let opening = cx.background_spawn(async move {
            let silero = Silero::new()?;
            Recorder::start(
                options,
                Some(Box::new(silero) as Box<dyn SpeechDetector>),
                levels,
                move |event| {
                    events
                        .unbounded_send(Event::Recorder { session: id, event })
                        .ok();
                },
            )
        });
        cx.spawn(async move |this, cx| {
            let opened = opening.await;
            this.update(cx, |daemon, cx| daemon.opened(id, opened, cx))
                .ok();
        })
        .detach();
    }

    /// Takes the microphone a session asked for. A session that was confirmed meanwhile stops it
    /// at once, and one that was cancelled drops it.
    fn opened(&mut self, id: u64, opened: Result<Recorder>, cx: &mut Context<Self>) {
        let session = self.session.as_mut().filter(|session| session.id == id);
        match (session, opened) {
            (Some(session), Ok(recorder)) => match session.stopping {
                true => recorder.stop(),
                false => session.recorder = Some(recorder),
            },
            (Some(_), Err(error)) => {
                log::error!("session {id}: cannot record: {error:#}");
                if let Some(pill) = self.pill.as_ref().map(|placed| placed.handle) {
                    set_phase(pill, Phase::Error, cx);
                }
                let session = self.session.take().expect("session checked above");
                self.close(session, false, cx);
            }
            (None, Ok(recorder)) => recorder.cancel(),
            (None, Err(_)) => {}
        }
    }

    /// Unfolds the waiting pill, or opens a new one when there is none or it sits on another
    /// output or at another place than the config now asks for.
    fn show_pill(&mut self, levels: Levels, config: &Config, cx: &mut Context<Self>) {
        // The pill outlives sessions, so it picks up a light or dark switch here. The portal also
        // answers late at login, after the waiting pill was first drawn.
        tokativ_ui::refresh_theme(cx);
        let output = tokativ_input::focused_output();
        if let Some(placed) = &self.pill
            && placed.fits(&output, config)
        {
            let reopened = placed.handle.update(cx, |pill, window, cx| {
                pill.reopen(levels.clone(), window, cx)
            });
            if reopened.is_ok() {
                return;
            }
        }
        self.pill = self.open_pill(levels, config, output, false, cx);
    }

    /// Opens a pill window, replacing any there was. `resting` opens it folded away.
    fn open_pill(
        &mut self,
        levels: Levels,
        config: &Config,
        output: Option<String>,
        resting: bool,
        cx: &mut Context<Self>,
    ) -> Option<Placed> {
        if let Some(old) = self.pill.take() {
            cx.defer(move |cx| {
                old.handle
                    .update(cx, |_, window, _| window.remove_window())
                    .ok();
            });
        }
        tokativ_ui::refresh_theme(cx);
        let confirm = cx.entity().downgrade();
        let cancel = confirm.clone();
        let opened = tokativ_ui::pill::open(
            levels,
            config.pill.edge,
            config.pill.margin,
            move |cx| {
                confirm.update(cx, |daemon, cx| daemon.confirm(cx)).ok();
            },
            move |cx| {
                cancel.update(cx, |daemon, cx| daemon.cancel(cx)).ok();
            },
            resting,
            cx,
        );
        match opened {
            Ok(handle) => Some(Placed {
                handle,
                output,
                pill: config.pill.clone(),
            }),
            Err(error) => {
                log::error!("session: cannot show the pill: {error:#}");
                None
            }
        }
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        let Some(session) = &mut self.session else {
            return;
        };
        if session.stopping {
            return;
        }
        // Nothing was said, so there is nothing to wait for: close at once rather than waiting
        // for the microphone to open or flush an empty buffer.
        if !session.heard && session.pending == 0 {
            let mut session = self.session.take().expect("session checked above");
            if let Some(recorder) = session.recorder.take() {
                recorder.cancel();
            }
            log::info!("session {}: confirmed with nothing said", session.id);
            self.close(session, true, cx);
            return;
        }
        session.stopping = true;
        if let Some(recorder) = session.recorder.take() {
            recorder.stop();
        }
        if let Some(pill) = self.pill.as_ref().map(|placed| placed.handle) {
            set_phase(pill, Phase::Processing, cx);
        }
        log::info!("session {}: confirmed", session.id);
        self.finish_if_done(cx);
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        if let Some(recorder) = session.recorder.take() {
            recorder.cancel();
        }
        log::info!("session {}: cancelled", session.id);
        self.close(session, true, cx);
    }

    fn recorded(&mut self, id: u64, event: tokativ_audio::Event, cx: &mut Context<Self>) {
        let Some(session) = self.session.as_mut().filter(|session| session.id == id) else {
            return;
        };
        match event {
            tokativ_audio::Event::Started => {
                session.heard = true;
                log::debug!("session {id}: speech");
            }
            tokativ_audio::Event::Segment(segment) => {
                session.pending += 1;
                let index = session.spoken;
                session.spoken += 1;
                log::debug!(
                    "session {id}: segment of {:.1} s{}",
                    segment.samples.len() as f32 / tokativ_audio::RATE as f32,
                    if segment.forced {
                        ", cut at the limit"
                    } else {
                        ""
                    }
                );
                // The last draft of this segment stays up until its final text replaces it.
                let drafted = matches!(&session.draft, Some((segment, _)) if *segment == index);
                if !drafted && let Some(inserter) = &self.inserter {
                    session.draft = Some((index, PLACEHOLDER.into()));
                    inserter.preedit(PLACEHOLDER.into());
                }
                let events = self.events.clone();
                self.transcriber.transcribe(
                    session.engine.clone(),
                    segment.samples,
                    move |result| {
                        events
                            .unbounded_send(Event::Transcribed {
                                session: id,
                                result,
                            })
                            .ok();
                    },
                );
            }
            tokativ_audio::Event::Progress(samples) => {
                let segment = session.spoken;
                let events = self.events.clone();
                self.transcriber
                    .draft(session.engine.clone(), samples, move |result| {
                        events
                            .unbounded_send(Event::Drafted {
                                session: id,
                                segment,
                                result,
                            })
                            .ok();
                    });
            }
            tokativ_audio::Event::Failed(error) => {
                log::error!("session {id}: capture failed: {error}");
                if let Some(pill) = self.pill.as_ref().map(|placed| placed.handle) {
                    set_phase(pill, Phase::Error, cx);
                }
                let session = self.session.take().expect("session checked above");
                self.close(session, false, cx);
            }
            tokativ_audio::Event::Finished => {
                session.capturing = false;
                self.finish_if_done(cx);
            }
        }
    }

    fn transcribed(&mut self, id: u64, result: Result<String>, cx: &mut Context<Self>) {
        let style = &self.applied.style;
        let capitalize = style.capitals == Capitals::AsSpoken;
        let Some(session) = self.session.as_mut().filter(|session| session.id == id) else {
            return;
        };
        session.pending -= 1;
        let segment = session.inserted;
        session.inserted += 1;
        // A draft of a later segment stays up, and a placeholder while earlier ones are pending.
        let after = match &session.draft {
            Some((drafted, text)) if *drafted > segment => text.clone(),
            _ if session.pending > 0 => PLACEHOLDER.to_string(),
            _ => String::new(),
        };
        if after.is_empty() {
            session.draft = None;
        }
        let text = match result {
            Ok(text) => text,
            Err(error) => {
                log::error!("session {id}: transcription failed: {error:#}");
                String::new()
            }
        };
        log::debug!("session {id}: {text:?}");
        if let Some(inserter) = &self.inserter {
            let joiner = session.joiner.get_or_insert_with(|| {
                Joiner::new(inserter.before_cursor().as_deref(), capitalize)
            });
            match joiner.join(&restyle(&text, style)) {
                Some(text) => inserter.insert(text, after),
                None => inserter.preedit(after),
            }
        }
        self.finish_if_done(cx);
    }

    /// Shows a draft of the segment being spoken, unless that segment has finished since.
    fn drafted(&mut self, id: u64, segment: usize, result: Result<String>) {
        let style = &self.applied.style;
        let capitalize = style.capitals == Capitals::AsSpoken;
        let Some(session) = self.session.as_mut().filter(|session| session.id == id) else {
            return;
        };
        if segment != session.spoken || session.stopping {
            return;
        }
        let Ok(text) = result else {
            return;
        };
        let Some(inserter) = &self.inserter else {
            return;
        };
        let joiner = session
            .joiner
            .get_or_insert_with(|| Joiner::new(inserter.before_cursor().as_deref(), capitalize));
        if let Some(preview) = joiner.preview(&restyle(&text, style)) {
            log::debug!("session {id}: draft {preview:?}");
            session.draft = Some((segment, preview.clone()));
            inserter.preedit(preview);
        }
    }

    fn finish_if_done(&mut self, cx: &mut Context<Self>) {
        let done = self
            .session
            .as_ref()
            .is_some_and(|session| session.stopping && !session.capturing && session.pending == 0);
        if !done {
            return;
        }
        let session = self.session.take().expect("session checked above");
        log::info!("session {}: done", session.id);
        let restart = session.restart;
        self.close(session, !restart, cx);
        if restart {
            self.start(cx);
        }
    }

    /// Ends a session's text insertion and, with `dismiss`, folds the pill away. A pill that
    /// shows an error or goes on to the next session stays.
    fn close(&mut self, session: Session, dismiss: bool, cx: &mut Context<Self>) {
        drop(session);
        if let Some(inserter) = &self.inserter {
            inserter.end();
        }
        if dismiss && let Some(pill) = self.pill.as_ref().map(|placed| placed.handle) {
            cx.defer(move |cx| {
                pill.update(cx, |pill, _, cx| pill.dismiss(cx)).ok();
            });
        }
        self.publish(cx);
    }

    /// Switches the model sessions dictate with, keeping the choice in the config. A model that
    /// is not downloaded opens the setup window instead.
    fn set_model(&mut self, model: String, cx: &mut Context<Self>) {
        let installed =
            ModelId::parse(&model).is_some_and(|id| self.library.read(cx).installed(&id));
        if !installed {
            log::warn!("model: {model} is not downloaded");
            self.open_setup(cx);
            return;
        }
        self.store.update(cx, |store, cx| {
            store.update(|config| config.model = model, cx)
        });
    }

    /// Applies a changed config. Session settings take effect from the next session.
    fn apply(&mut self, config: Config, cx: &mut Context<Self>) {
        let old = std::mem::replace(&mut self.applied, config.clone());
        if old.processor != config.processor {
            self.transcriber.set_processor(config.processor);
        }
        if old.model != config.model || old.processor != config.processor {
            log::info!("model: {} on {:?}", config.model, config.processor);
            self.preload(cx);
        }
        if old.insertion.backend != config.insertion.backend {
            self.inserter = start_inserter(&config, &self.events);
        } else if old.insertion.terminals != config.insertion.terminals
            && let Some(inserter) = &self.inserter
        {
            inserter.set_terminals(config.insertion.terminals.clone());
        }
        if old.tray != config.tray {
            self.place_tray(config.tray, cx);
        }
        if old.launch_at_login != config.launch_at_login
            && let Err(error) = autostart::apply(config.launch_at_login)
        {
            log::error!("autostart: {error:#}");
        }
        if old.models != config.models {
            self.transcriber.set_idle_limit(idle_limit(&config));
        }
        self.publish(cx);
    }

    fn preload(&self, cx: &App) {
        let config = self.store.read(cx).config();
        if config.models.unload_after_minutes != 0 {
            return;
        }
        if let Some(id) = ModelId::parse(&config.model)
            && self.library.read(cx).installed(&id)
        {
            self.transcriber.preload(id);
        }
    }

    /// Shows or hides the tray item. A sandboxed daemon never shows one, so the panel keeps a
    /// single item for the real install.
    fn place_tray(&mut self, placed: bool, cx: &App) {
        self.tray = match placed && tokativ_config::sandbox().is_none() {
            false => None,
            true => Tray::spawn(
                self.tray_events.clone(),
                self.session.is_some(),
                self.shown(cx),
            )
            .map_err(|error| log::warn!("tray: {error:#}"))
            .ok(),
        };
    }

    /// The models as the tray lists them.
    fn shown(&self, cx: &App) -> Shown {
        let library = self.library.read(cx);
        Shown {
            model: self.store.read(cx).config().model.clone(),
            installed: library
                .installed_models()
                .iter()
                .map(|model| (model.id.key(), model.id.name()))
                .collect(),
        }
    }

    /// Pushes the daemon's state to the windows and the tray.
    fn publish(&mut self, cx: &mut Context<Self>) {
        let recording = self.session.is_some();
        let shown = self.shown(cx);
        let device = self.transcriber.device();
        let (method, note) = match &self.inserter {
            Some(inserter) => {
                let status = inserter.status();
                (status.method.name().to_string(), status.note)
            }
            None => (
                "Unavailable".to_string(),
                Some("no Wayland connection".to_string()),
            ),
        };
        self.status.update(cx, |status, cx| {
            *status = Status {
                method,
                note,
                recording,
                device,
            };
            cx.notify();
        });
        if let Some(tray) = &self.tray {
            tray.show(recording, shown);
        }
    }

    fn open_settings(&mut self, cx: &mut Context<Self>) {
        tokativ_ui::refresh_theme(cx);
        tokativ_ui::settings::open(cx);
    }

    fn open_setup(&mut self, cx: &mut Context<Self>) {
        tokativ_ui::refresh_theme(cx);
        tokativ_ui::setup::open(cx);
    }

    fn quit(&mut self, cx: &mut Context<Self>) {
        self.cancel(cx);
        self.tray = None;
        cx.quit();
    }
}

/// Changes the pill's phase once the current update is done, since the change may come from a
/// click inside the pill itself and GPUI cannot update a window that is mid-update.
fn set_phase(pill: WindowHandle<Pill>, phase: Phase, cx: &mut App) {
    cx.defer(move |cx| {
        pill.update(cx, |pill, window, cx| pill.set_phase(phase, window, cx))
            .ok();
    });
}

/// Connects the text insertion, with Escape and Enter fed back as events.
fn start_inserter(config: &Config, events: &UnboundedSender<Event>) -> Option<Inserter> {
    let keys = events.clone();
    Inserter::start(
        config.insertion.backend,
        config.insertion.terminals.clone(),
        move |key| {
            keys.unbounded_send(Event::Key(key)).ok();
        },
    )
    .map_err(|error| log::error!("text insertion is unavailable: {error:#}"))
    .ok()
}

fn idle_limit(config: &Config) -> Option<Duration> {
    match config.models.unload_after_minutes {
        0 => None,
        minutes => Some(Duration::from_secs(minutes as u64 * 60)),
    }
}

/// Watches the config folder and calls `changed` whenever the config file is written. The
/// folder is watched rather than the file, since saving replaces the file.
fn watch_config(changed: impl Fn() + Send + 'static) -> Result<notify::RecommendedWatcher> {
    let directory = config_dir();
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("creating {}", directory.display()))?;
    let path = config_path();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else {
            return;
        };
        let touched = event.paths.iter().any(|touched| touched == &path);
        if touched && (event.kind.is_create() || event.kind.is_modify()) {
            changed();
        }
    })
    .context("starting the config watcher")?;
    watcher
        .watch(&directory, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching {}", directory.display()))?;
    Ok(watcher)
}

/// The editing keys for the text fields in the setup and settings windows.
fn bindings() -> Vec<KeyBinding> {
    use ui::*;
    let editing = Some(INPUT_CONTEXT);
    vec![
        KeyBinding::new("backspace", Backspace, editing),
        KeyBinding::new("ctrl-backspace", BackspaceWord, editing),
        KeyBinding::new("delete", Delete, editing),
        KeyBinding::new("ctrl-delete", DeleteWord, editing),
        KeyBinding::new("left", Left, editing),
        KeyBinding::new("right", Right, editing),
        KeyBinding::new("ctrl-left", WordLeft, editing),
        KeyBinding::new("ctrl-right", WordRight, editing),
        KeyBinding::new("shift-left", SelectLeft, editing),
        KeyBinding::new("shift-right", SelectRight, editing),
        KeyBinding::new("shift-ctrl-left", SelectWordLeft, editing),
        KeyBinding::new("shift-ctrl-right", SelectWordRight, editing),
        KeyBinding::new("home", Home, editing),
        KeyBinding::new("end", End, editing),
        KeyBinding::new("shift-home", SelectHome, editing),
        KeyBinding::new("shift-end", SelectEnd, editing),
        KeyBinding::new("ctrl-a", SelectAll, editing),
        KeyBinding::new("ctrl-v", Paste, editing),
        KeyBinding::new("ctrl-c", Copy, editing),
        KeyBinding::new("ctrl-x", Cut, editing),
        KeyBinding::new("space", Space, editing),
        KeyBinding::new("escape", Dismiss, editing),
    ]
}
