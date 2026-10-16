//! Runs a 16 kHz mono WAV file through the VAD, the segmenter and an engine, and prints each
//! segment's text as a live session would insert it.
//!
//! `cargo run -p tokativ-engine --example file -- <parakeet|gigaam> <file.wav>`

use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use tokativ_audio::{Silero, Timing};
use tokativ_config::EngineKind;
use tokativ_engine::Joiner;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let kind = args
        .next()
        .and_then(|id| EngineKind::from_id(&id))
        .context("first argument: parakeet or gigaam")?;
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

    let mut engine = tokativ_engine::load(kind)?;
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
