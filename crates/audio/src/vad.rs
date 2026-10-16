//! Silero VAD v6, run through ONNX Runtime on 512-sample frames. The model ships inside the
//! binary, it is 2 MB.

use anyhow::{Result, anyhow};
use ndarray::{Array2, Array3, Axis, concatenate, s};
use ort::session::Session;
use ort::value::TensorRef;

use crate::{FRAME, RATE, SpeechDetector};

const MODEL: &[u8] = include_bytes!("../../../assets/silero/silero_vad.onnx");
/// The tail of the previous frame the model expects in front of each new one.
const CONTEXT: usize = 64;

/// A Silero detector with its recurrent state. One instance follows one audio stream.
pub struct Silero {
    session: Session,
    state: Array3<f32>,
    context: Vec<f32>,
}

impl Silero {
    pub fn new() -> Result<Self> {
        let session = build().map_err(|error| anyhow!("loading the Silero VAD model: {error}"))?;
        Ok(Self {
            session,
            state: Array3::zeros((2, 1, 128)),
            context: vec![0.; CONTEXT],
        })
    }

    fn run(&mut self, frame: &[f32; FRAME]) -> Result<f32> {
        let context = Array2::from_shape_vec((1, CONTEXT), self.context.clone())?;
        let current = Array2::from_shape_vec((1, FRAME), frame.to_vec())?;
        let input = concatenate(Axis(1), &[context.view(), current.view()])?;
        let rate = ndarray::arr0(RATE as i64);
        let outputs = self.session.run(ort::inputs![
            "input" => TensorRef::from_array_view(input.view())?,
            "state" => TensorRef::from_array_view(self.state.view())?,
            "sr" => TensorRef::from_array_view(rate.view())?,
        ])?;
        let probability = outputs["output"].try_extract_array::<f32>()?[[0, 0]];
        self.state = outputs["stateN"]
            .try_extract_array::<f32>()?
            .to_owned()
            .into_dimensionality()?;
        self.context = input.slice(s![0, FRAME..]).to_vec();
        Ok(probability)
    }
}

fn build() -> ort::Result<Session> {
    let session = Session::builder()?
        .with_intra_threads(1)?
        .with_inter_threads(1)?
        .commit_from_memory(MODEL)?;
    Ok(session)
}

impl SpeechDetector for Silero {
    fn probability(&mut self, frame: &[f32; FRAME]) -> f32 {
        self.run(frame).unwrap_or_else(|error| {
            log::warn!("vad: {error:#}");
            0.
        })
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
