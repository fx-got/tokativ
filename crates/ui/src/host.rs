//! The state the windows share with the daemon: the config, the model library and what the
//! daemon reports about itself, plus the way a window asks the daemon to act.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Entity, Global};
use tokativ_config::{Config, EngineKind, LanguageMode};
use tokativ_engine::models;

/// What a window asks the daemon to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Confirm,
    Cancel,
    Settings,
    Quit,
}

type Dispatch = Rc<dyn Fn(Action, &mut App)>;

/// The handles every window reaches through `Host::global(cx)`.
#[derive(Clone)]
pub struct Host {
    pub store: Entity<Store>,
    pub library: Entity<Library>,
    pub status: Entity<Status>,
    dispatch: Dispatch,
}

impl Global for Host {}

impl Host {
    pub fn install(
        store: Entity<Store>,
        library: Entity<Library>,
        status: Entity<Status>,
        dispatch: impl Fn(Action, &mut App) + 'static,
        cx: &mut App,
    ) {
        cx.set_global(Self {
            store,
            library,
            status,
            dispatch: Rc::new(dispatch),
        });
    }

    pub fn global(cx: &App) -> Self {
        cx.global::<Self>().clone()
    }

    pub fn dispatch(&self, action: Action, cx: &mut App) {
        (self.dispatch)(action, cx);
    }
}

/// The config as the app currently runs it. Changes made here are written to disk at once, and
/// the daemon observes this entity to apply them.
pub struct Store {
    config: Config,
}

impl Store {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Edits the config and saves it. A failed save is logged and the change still applies for
    /// this run.
    pub fn update(&mut self, edit: impl FnOnce(&mut Config), cx: &mut Context<Self>) {
        let mut config = self.config.clone();
        edit(&mut config);
        if config == self.config {
            return;
        }
        if let Err(error) = config.save() {
            log::error!("config: cannot save: {error:#}");
        }
        self.config = config;
        cx.notify();
    }

    /// Takes a config read back from disk after an outside edit, without saving it again.
    pub fn replace(&mut self, config: Config, cx: &mut Context<Self>) {
        if config != self.config {
            self.config = config;
            cx.notify();
        }
    }
}

/// What the daemon reports for the windows to show.
#[derive(Clone, Debug, Default)]
pub struct Status {
    /// The insertion method in use, by name.
    pub method: String,
    /// Why the method is not the one asked for, when it is not.
    pub note: Option<String>,
    pub recording: bool,
    pub mode: LanguageMode,
}

/// Where one model stands on disk.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelState {
    Missing,
    Downloading { done: u64, total: u64 },
    Installed { bytes: u64 },
    Failed(String),
}

/// The downloadable models and their state. Downloads run on their own threads and report
/// progress here.
pub struct Library {
    states: HashMap<EngineKind, ModelState>,
    cancels: HashMap<EngineKind, Arc<AtomicBool>>,
}

const PROGRESS_TICK: Duration = Duration::from_millis(120);

impl Library {
    pub fn new() -> Self {
        let mut library = Self {
            states: HashMap::new(),
            cancels: HashMap::new(),
        };
        library.scan();
        library
    }

    pub fn state(&self, kind: EngineKind) -> ModelState {
        self.states
            .get(&kind)
            .cloned()
            .unwrap_or(ModelState::Missing)
    }

    pub fn installed(&self, kind: EngineKind) -> bool {
        matches!(self.state(kind), ModelState::Installed { .. })
    }

    pub fn any_installed(&self) -> bool {
        EngineKind::ALL.into_iter().any(|kind| self.installed(kind))
    }

    fn scan(&mut self) {
        for kind in EngineKind::ALL {
            if matches!(self.state(kind), ModelState::Downloading { .. }) {
                continue;
            }
            let state = match models::installed(kind) {
                Some(bytes) => ModelState::Installed { bytes },
                None => ModelState::Missing,
            };
            self.states.insert(kind, state);
        }
    }

    pub fn download(&mut self, kind: EngineKind, cx: &mut Context<Self>) {
        if matches!(self.state(kind), ModelState::Downloading { .. }) {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancels.insert(kind, cancel.clone());
        self.states
            .insert(kind, ModelState::Downloading { done: 0, total: 0 });
        cx.notify();

        let progress = Arc::new(Mutex::new((0u64, 0u64)));
        let reported = progress.clone();
        let (sender, receiver) = futures::channel::oneshot::channel();
        std::thread::Builder::new()
            .name(format!("download-{}", kind.id()))
            .spawn(move || {
                let result = models::download(
                    kind,
                    &|done, total| *reported.lock().expect("progress lock") = (done, total),
                    &cancel,
                );
                sender.send(result).ok();
            })
            .expect("spawning a download thread");

        cx.spawn(async move |this, cx| {
            let mut receiver = receiver;
            let result = loop {
                let tick = cx.background_executor().timer(PROGRESS_TICK);
                match futures::future::select(&mut receiver, tick).await {
                    futures::future::Either::Left((result, _)) => break result,
                    futures::future::Either::Right(_) => {
                        let (done, total) = *progress.lock().expect("progress lock");
                        let updated = this.update(cx, |this, cx| {
                            this.states
                                .insert(kind, ModelState::Downloading { done, total });
                            cx.notify();
                        });
                        if updated.is_err() {
                            return;
                        }
                    }
                }
            };
            this.update(cx, |this, cx| {
                this.cancels.remove(&kind);
                let state = match result {
                    Ok(Ok(())) => match models::installed(kind) {
                        Some(bytes) => ModelState::Installed { bytes },
                        None => ModelState::Failed("the download left no model".into()),
                    },
                    Ok(Err(error)) if error.is::<models::Cancelled>() => ModelState::Missing,
                    Ok(Err(error)) => {
                        log::error!("models: {} download failed: {error:#}", kind.id());
                        ModelState::Failed(format!("{error:#}"))
                    }
                    Err(_) => ModelState::Failed("the download thread died".into()),
                };
                this.states.insert(kind, state);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn cancel(&mut self, kind: EngineKind) {
        if let Some(cancel) = self.cancels.get(&kind) {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn delete(&mut self, kind: EngineKind, cx: &mut Context<Self>) {
        if let Err(error) = models::delete(kind) {
            log::error!("models: cannot delete {}: {error:#}", kind.id());
        }
        self.scan();
        cx.notify();
    }
}

impl Default for Library {
    fn default() -> Self {
        Self::new()
    }
}

/// Creates the shared entities with the config loaded from disk.
pub fn entities(config: Config, cx: &mut App) -> (Entity<Store>, Entity<Library>, Entity<Status>) {
    (
        cx.new(|_| Store::new(config)),
        cx.new(|_| Library::new()),
        cx.new(|_| Status::default()),
    )
}
