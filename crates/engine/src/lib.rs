//! Speech-to-text engines and the worker thread that runs them. Both engines are ONNX models
//! run on the CPU through transcribe-rs.

pub mod models;
mod text;

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::Path;
use std::sync::Once;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow};
use tokativ_config::EngineKind;
use transcribe_rs::onnx::Quantization;
use transcribe_rs::onnx::gigaam::GigaAMModel;
use transcribe_rs::onnx::parakeet::ParakeetModel;
use transcribe_rs::{OrtAccelerator, SpeechModel as _, TranscribeOptions};

pub use text::Joiner;

/// A loaded speech-to-text model.
pub trait Engine: Send {
    /// Loads the model from its folder. Slow, so it runs on the worker thread.
    fn load(dir: &Path) -> Result<Self>
    where
        Self: Sized;

    /// Transcribes 16 kHz mono audio.
    fn transcribe(&mut self, samples: &[f32]) -> Result<String>;

    /// The languages the model understands, as ISO 639-1 codes.
    fn languages(&self) -> &'static [&'static str];
}

/// NVIDIA's Parakeet TDT 0.6B v3. It picks the language itself from 25 European ones.
pub struct Parakeet(ParakeetModel);

impl Engine for Parakeet {
    fn load(dir: &Path) -> Result<Self> {
        ParakeetModel::load(dir, &Quantization::Int8)
            .map(Self)
            .map_err(|error| anyhow!("loading Parakeet from {}: {error}", dir.display()))
    }

    fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        self.0
            .transcribe(samples, &TranscribeOptions::default())
            .map(|result| result.text)
            .map_err(|error| anyhow!("Parakeet: {error}"))
    }

    fn languages(&self) -> &'static [&'static str] {
        &[
            "bg", "cs", "da", "de", "el", "en", "es", "et", "fi", "fr", "hr", "hu", "it", "lt",
            "lv", "mt", "nl", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "uk",
        ]
    }
}

/// Sber's GigaAM v3, the end-to-end CTC export that writes punctuation and capitals.
pub struct GigaAm(GigaAMModel);

impl Engine for GigaAm {
    fn load(dir: &Path) -> Result<Self> {
        GigaAMModel::load(dir, &Quantization::Int8)
            .map(Self)
            .map_err(|error| anyhow!("loading GigaAM from {}: {error}", dir.display()))
    }

    fn transcribe(&mut self, samples: &[f32]) -> Result<String> {
        self.0
            .transcribe(samples, &TranscribeOptions::default())
            .map(|result| result.text)
            .map_err(|error| anyhow!("GigaAM: {error}"))
    }

    fn languages(&self) -> &'static [&'static str] {
        &["ru"]
    }
}

/// Loads the engine of `kind` from its installed folder.
pub fn load(kind: EngineKind) -> Result<Box<dyn Engine>> {
    static CPU: Once = Once::new();
    CPU.call_once(|| transcribe_rs::set_ort_accelerator(OrtAccelerator::CpuOnly));
    if models::installed(kind).is_none() {
        return Err(anyhow!("{} is not downloaded", kind.name()));
    }
    let dir = models::dir(kind);
    let started = Instant::now();
    let engine: Box<dyn Engine> = match kind {
        EngineKind::Parakeet => Box::new(Parakeet::load(&dir)?),
        EngineKind::Gigaam => Box::new(GigaAm::load(&dir)?),
    };
    log::info!(
        "engine: loaded {} in {:.1?}",
        kind.name(),
        started.elapsed()
    );
    Ok(engine)
}

type Reply = Box<dyn FnOnce(Result<String>) + Send>;

enum Job {
    Preload(EngineKind),
    Transcribe {
        kind: EngineKind,
        samples: Vec<f32>,
        reply: Reply,
    },
    /// Drop models after this long without a job. `None` keeps them.
    IdleLimit(Option<Duration>),
}

/// The handle to the transcription thread. Jobs run one at a time in the order they were sent,
/// so segments come back in the order they were spoken. Models load on first use and stay
/// loaded until the idle limit drops them.
#[derive(Clone)]
pub struct Transcriber {
    jobs: Sender<Job>,
}

impl Transcriber {
    pub fn start(idle: Option<Duration>) -> Result<Self> {
        let (jobs, receiver) = mpsc::channel();
        thread::Builder::new()
            .name("transcribe".into())
            .spawn(move || work(receiver, idle))
            .context("spawning the transcription thread")?;
        Ok(Self { jobs })
    }

    /// Loads `kind` ahead of the first segment, so the first result is not held up by it.
    pub fn preload(&self, kind: EngineKind) {
        self.jobs.send(Job::Preload(kind)).ok();
    }

    /// Queues `samples` for transcription with `kind`. `reply` runs on the worker thread.
    pub fn transcribe(
        &self,
        kind: EngineKind,
        samples: Vec<f32>,
        reply: impl FnOnce(Result<String>) + Send + 'static,
    ) {
        let job = Job::Transcribe {
            kind,
            samples,
            reply: Box::new(reply),
        };
        if let Err(mpsc::SendError(Job::Transcribe { reply, .. })) = self.jobs.send(job) {
            reply(Err(anyhow!("the transcription thread is gone")));
        }
    }

    pub fn set_idle_limit(&self, idle: Option<Duration>) {
        self.jobs.send(Job::IdleLimit(idle)).ok();
    }
}

fn work(jobs: Receiver<Job>, mut idle: Option<Duration>) {
    let mut loaded: HashMap<EngineKind, Box<dyn Engine>> = HashMap::new();
    let mut last = Instant::now();
    loop {
        let wait = match (idle, loaded.is_empty()) {
            (Some(limit), false) => limit.saturating_sub(last.elapsed()),
            _ => Duration::from_secs(3600),
        };
        let job = match jobs.recv_timeout(wait) {
            Ok(job) => job,
            Err(RecvTimeoutError::Timeout) => {
                if idle.is_some_and(|limit| last.elapsed() >= limit) && !loaded.is_empty() {
                    log::info!("engine: unloading models after idling");
                    loaded.clear();
                }
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        match job {
            Job::Preload(kind) => {
                if let Err(error) = engine(&mut loaded, kind) {
                    log::error!("engine: {error:#}");
                }
            }
            Job::Transcribe {
                kind,
                samples,
                reply,
            } => {
                let started = Instant::now();
                let result =
                    engine(&mut loaded, kind).and_then(|engine| engine.transcribe(&samples));
                log::debug!(
                    "engine: {:.1} s of audio in {:.2?}",
                    samples.len() as f32 / 16_000.,
                    started.elapsed()
                );
                reply(result);
            }
            Job::IdleLimit(limit) => idle = limit,
        }
        last = Instant::now();
    }
}

fn engine(
    loaded: &mut HashMap<EngineKind, Box<dyn Engine>>,
    kind: EngineKind,
) -> Result<&mut Box<dyn Engine>> {
    match loaded.entry(kind) {
        Entry::Occupied(entry) => Ok(entry.into_mut()),
        Entry::Vacant(entry) => Ok(entry.insert(load(kind)?)),
    }
}
