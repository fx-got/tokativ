//! Runs a 16 kHz mono WAV file through the VAD, the segmenter and an engine, and prints each
//! segment's text as a live session would insert it.
//!
//! `cargo run -p tokativ-engine --example file -- <parakeet|gigaam> <file.wav>`

use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use tokativ_audio::{Silero, Timing};
use tokativ_config::Processor;
use tokativ_engine::{Engine as _, Gguf, Joiner, ModelId};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let model = args
        .next()
        .and_then(|id| ModelId::parse(&id))
        .context("first argument: a model as owner/repo/file.gguf")?;
    let path = args.next().context("second argument: a wav file")?;
    let mut reader = hound::WavReader::open(&path)?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000 || spec.channels != 1 {
        bail!(
            "need 16 kHz mono, got {} Hz with {} channels",
            spec.sample_rate,
            spec.channels
        );
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader
            .samples::<i16>()
            .map(|sample| sample.map(|value| value as f32 / i16::MAX as f32))
            .collect::<Result<_, _>>()?,
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
    };

    let timing = Timing {
        threshold: 0.5,
        silence_ms: 500,
        max_segment_s: 20,
        preroll_ms: 200,
    };
    let started = Instant::now();
    let segments = tokativ_audio::segments(&samples, timing, &mut Silero::new()?);
    println!("{} segments in {:.2?}", segments.len(), started.elapsed());

    let processor = match std::env::args().nth(3).as_deref() {
        Some("cpu") => Processor::Cpu,
        Some("gpu") => Processor::Gpu,
        _ => Processor::Auto,
    };
    let mut engine = Gguf::load(&model, processor)?;
    println!("running on {}", engine.device());
    let mut joiner = Joiner::new(None);
    for segment in segments {
        let started = Instant::now();
        let text = engine.transcribe(&segment.samples)?;
        let seconds = segment.samples.len() as f32 / 16_000.;
        println!(
            "[{seconds:.1} s in {:.2?}] {:?}",
            started.elapsed(),
            joiner.join(&text)
        );
    }
    Ok(())
}
