//! Silero VAD v6.2, run through transcribe.cpp on 512-sample frames. The model ships inside the
//! binary, it is 1.2 MB, and is written out once to the cache folder for the engine to load.

use std::path::PathBuf;

use anyhow::{Context as _, Result, anyhow};
use transcribe_cpp::{Backend, Model, ModelOptions, VadSession};

use crate::{FRAME, SpeechDetector};

const MODEL: &[u8] = include_bytes!("../../../assets/silero/silero-vad-v6.2-F32.gguf");

/// A Silero detector with its recurrent state. One instance follows one audio stream.
pub struct Silero {
    session: VadSession,
}

impl Silero {
    pub fn new() -> Result<Self> {
        let path = model_file()?;
        // The model is tiny and runs a frame at a time, so the CPU is faster than a round trip
        // to the graphics card.
        let model = Model::load_with(
            &path,
            &ModelOptions {
                backend: Backend::Cpu,
                device: None,
            },
        )
        .map_err(|error| anyhow!("loading the Silero VAD model: {error}"))?;
        let session = model
            .vad_session()
            .map_err(|error| anyhow!("starting the Silero VAD: {error}"))?;
        Ok(Self { session })
    }
}

/// The bundled model on disk, written there the first time or whenever it changed.
fn model_file() -> Result<PathBuf> {
    let folder = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(tokativ_config::APP_ID);
    let path = folder.join("silero-vad-v6.2-F32.gguf");
    let current = std::fs::metadata(&path).is_ok_and(|meta| meta.len() == MODEL.len() as u64);
    if !current {
        std::fs::create_dir_all(&folder)
            .with_context(|| format!("creating {}", folder.display()))?;
        std::fs::write(&path, MODEL).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(path)
}

impl SpeechDetector for Silero {
    fn probability(&mut self, frame: &[f32; FRAME]) -> f32 {
        match self.session.stream_feed(frame) {
            Ok(result) => result.probs.last().copied().unwrap_or(0.),
            Err(error) => {
                log::warn!("vad: {error}");
                0.
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_not_speech() {
        let mut silero = Silero::new().unwrap();
        for _ in 0..10 {
            assert!(silero.probability(&[0.; FRAME]) < 0.1);
        }
    }
}
