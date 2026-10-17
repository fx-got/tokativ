//! Speech-to-text and the worker thread that runs it. Models are GGUF files run by
//! transcribe.cpp, on the graphics card through Vulkan when there is one.

pub mod models;
mod text;

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use tokativ_config::Processor;
use transcribe_cpp::{Backend, ModelOptions, RunOptions, Session};

pub use models::ModelId;
pub use text::{Joiner, restyle};

/// A loaded speech-to-text model.
pub trait Engine: Send {
    /// Transcribes 16 kHz mono audio.
    fn transcribe(&mut self, samples: &[f32]) -> Result<String>;

    /// The languages the model understands, as ISO 639-1 codes. Empty when the model does not
    /// say.
    fn languages(&self) -> Vec<String>;

    /// The device the model runs on, as the user would name it.
    fn device(&self) -> String;
}

/// Any model transcribe.cpp can load: Parakeet, GigaAM, Whisper, Canary and the rest.
pub struct Gguf {
    model: transcribe_cpp::Model,
    session: Session,
}

impl Gguf {
    /// Loads a model file on the device `processor` asks for. Slow, so it runs on the worker.
    pub fn load(id: &ModelId, processor: Processor) -> Result<Self> {
        if models::size_on_disk(id).is_none() {
            return Err(anyhow!("{} is not downloaded", id.name()));
        }
        let backend = match processor {
            Processor::Auto => Backend::Auto,
            Processor::Cpu => Backend::Cpu,
            Processor::Gpu => Backend::Vulkan,
        };
        let started = Instant::now();
        let model = transcribe_cpp::Model::load_with(
            id.path(),
            &ModelOptions {
                backend,
                device: None,
            },
        )
        .map_err(|error| anyhow!("loading {}: {error}", id.name()))?;
        let session = model
            .session()
            .map_err(|error| anyhow!("starting {}: {error}", id.name()))?;
        let engine = Self { model, session };
        log::info!(
            "engine: loaded {} ({}) on {} in {:.1?}",
            id.name(),
            engine.model.arch(),
            engine.device(),
            started.elapsed()
        );
        Ok(engine)
    }
}

impl Engine for Gguf {
    fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        self.session
            .run(samples, &RunOptions::default())
            .map(|transcript| transcript.text)
            .map_err(|error| anyhow!("transcribing: {error}"))
    }

    fn languages(&self) -> Vec<String> {
        self.model
            .capabilities()
            .map(|capabilities| capabilities.languages)
            .unwrap_or_default()
    }

    fn device(&self) -> String {
        match self.model.device() {
            Ok(device) if !device.description.is_empty() => device.description,
            _ => self.model.backend(),
        }
    }
}

type Reply = Box<dyn FnOnce(Result<String>) + Send>;

enum Job {
    Preload(ModelId),
    Transcribe {
        model: ModelId,
        samples: Vec<f32>,
        reply: Reply,
    },
    /// Wakes the worker to take whatever draft sits in the shared slot.
    Draft,
    /// Drop models after this long without a job. `None` keeps them.
    IdleLimit(Option<Duration>),
    /// Reload models on another device from the next job on.
    Processor(Processor),
}

/// The handle to the transcription thread. Jobs run one at a time in the order they were sent,
/// so segments come back in the order they were spoken. Models load on first use and stay
/// loaded until the idle limit drops them.
#[derive(Clone)]
pub struct Transcriber {
    jobs: Sender<Job>,
    draft: Arc<Mutex<Option<Draft>>>,
    device: Arc<Mutex<Option<String>>>,
}

/// A transcription whose result is only shown, never kept. Only the newest one waits, so drafts
/// never pile up behind a slow model.
struct Draft {
    model: ModelId,
    samples: Vec<f32>,
    reply: Reply,
}

impl Transcriber {
    pub fn start(idle: Option<Duration>, processor: Processor) -> Result<Self> {
        let (jobs, receiver) = mpsc::channel();
        let draft = Arc::new(Mutex::new(None));
        let device = Arc::new(Mutex::new(None));
        let worker = Worker {
            jobs: receiver,
            drafts: draft.clone(),
            device: device.clone(),
            idle,
            processor,
            loaded: HashMap::new(),
        };
        thread::Builder::new()
            .name("transcribe".into())
            .spawn(move || worker.run())
            .context("spawning the transcription thread")?;
        Ok(Self {
            jobs,
            draft,
            device,
        })
    }

    /// Loads `model` ahead of the first segment, so the first result is not held up by it.
    pub fn preload(&self, model: ModelId) {
        self.jobs.send(Job::Preload(model)).ok();
    }

    /// Queues `samples` for transcription with `model`. `reply` runs on the worker thread.
    pub fn transcribe(
        &self,
        model: ModelId,
        samples: Vec<f32>,
        reply: impl FnOnce(Result<String>) + Send + 'static,
    ) {
        let job = Job::Transcribe {
            model,
            samples,
            reply: Box::new(reply),
        };
        if let Err(mpsc::SendError(Job::Transcribe { reply, .. })) = self.jobs.send(job) {
            reply(Err(anyhow!("the transcription thread is gone")));
        }
    }

    /// Transcribes `samples` as a draft for live preview. It runs after the segments already
    /// queued, and a newer draft replaces this one if it has not started yet, in which case
    /// `reply` never runs.
    pub fn draft(
        &self,
        model: ModelId,
        samples: Vec<f32>,
        reply: impl FnOnce(Result<String>) + Send + 'static,
    ) {
        *self.draft.lock().expect("draft lock") = Some(Draft {
            model,
            samples,
            reply: Box::new(reply),
        });
        self.jobs.send(Job::Draft).ok();
    }

    pub fn set_idle_limit(&self, idle: Option<Duration>) {
        self.jobs.send(Job::IdleLimit(idle)).ok();
    }

    pub fn set_processor(&self, processor: Processor) {
        self.jobs.send(Job::Processor(processor)).ok();
    }

    /// The device the most recently loaded model runs on, once one has loaded.
    pub fn device(&self) -> Option<String> {
        self.device.lock().expect("device lock").clone()
    }
}

struct Worker {
    jobs: Receiver<Job>,
    drafts: Arc<Mutex<Option<Draft>>>,
    device: Arc<Mutex<Option<String>>>,
    idle: Option<Duration>,
    processor: Processor,
    loaded: HashMap<ModelId, Gguf>,
}

impl Worker {
    fn run(mut self) {
        let mut last = Instant::now();
        loop {
            let wait = match (self.idle, self.loaded.is_empty()) {
                (Some(limit), false) => limit.saturating_sub(last.elapsed()),
                _ => Duration::from_secs(3600),
            };
            let job = match self.jobs.recv_timeout(wait) {
                Ok(job) => job,
                Err(RecvTimeoutError::Timeout) => {
                    if self.idle.is_some_and(|limit| last.elapsed() >= limit)
                        && !self.loaded.is_empty()
                    {
                        log::info!("engine: unloading models after idling");
                        self.loaded.clear();
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };
            match job {
                Job::Preload(model) => {
                    if let Err(error) = self.engine(model) {
                        log::error!("engine: {error:#}");
                    }
                }
                Job::Transcribe {
                    model,
                    samples,
                    reply,
                } => {
                    let started = Instant::now();
                    let result = self
                        .engine(model)
                        .and_then(|engine| engine.transcribe(&samples));
                    log::debug!(
                        "engine: {:.1} s of audio in {:.2?}",
                        samples.len() as f32 / 16_000.,
                        started.elapsed()
                    );
                    reply(result);
                }
                Job::Draft => {
                    let Some(draft) = self.drafts.lock().expect("draft lock").take() else {
                        continue;
                    };
                    let result = self
                        .engine(draft.model)
                        .and_then(|engine| engine.transcribe(&draft.samples));
                    (draft.reply)(result);
                }
                Job::IdleLimit(limit) => self.idle = limit,
                Job::Processor(processor) => {
                    if processor != self.processor {
                        self.processor = processor;
                        self.loaded.clear();
                    }
                }
            }
            last = Instant::now();
        }
    }

    fn engine(&mut self, model: ModelId) -> Result<&mut Gguf> {
        let engine = match self.loaded.entry(model) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let engine = Gguf::load(entry.key(), self.processor)?;
                entry.insert(engine)
            }
        };
        *self.device.lock().expect("device lock") = Some(engine.device());
        Ok(engine)
    }
}

/// Sends transcribe.cpp's and ggml's messages through the `log` crate instead of straight to
/// stderr. Call once at startup, before any model loads.
pub fn route_native_logs() {
    transcribe_cpp::init_logging();
}
