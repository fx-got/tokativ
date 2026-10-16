//! Converts mono audio at the device rate to the 16 kHz the VAD and the engines take.

use anyhow::{Context as _, Result};
use rubato::{FftFixedIn, Resampler as _};

use crate::RATE;

const CHUNK: usize = 1024;

/// A streaming resampler. Input arrives in buffers of any length and output comes out in
/// whatever lengths the FFT blocks give, with a delay of about one block.
pub(crate) struct Resampler {
    inner: Option<FftFixedIn<f32>>,
    pending: Vec<f32>,
}

impl Resampler {
    pub fn new(from: u32) -> Result<Self> {
        let inner = match from == RATE {
            true => None,
            false => Some(
                FftFixedIn::new(from as usize, RATE as usize, CHUNK, 2, 1)
                    .context("building the resampler")?,
            ),
        };
        Ok(Self {
            inner,
            pending: Vec::new(),
        })
    }

    /// Feeds `input` and appends any finished output to `output`.
    pub fn push(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let Some(inner) = &mut self.inner else {
            output.extend_from_slice(input);
            return;
        };
        self.pending.extend_from_slice(input);
        let mut start = 0;
        loop {
            let needed = inner.input_frames_next();
            if self.pending.len() - start < needed {
                break;
            }
            match inner.process(&[&self.pending[start..start + needed]], None) {
                Ok(mut channels) => output.append(&mut channels[0]),
                Err(error) => log::warn!("resample: {error}"),
            }
            start += needed;
        }
        self.pending.drain(..start);
    }

    /// Pushes out whatever is still buffered, padding the last block with silence.
    pub fn finish(&mut self, output: &mut Vec<f32>) {
        let Some(inner) = &mut self.inner else {
            return;
        };
        let pending = std::mem::take(&mut self.pending);
        match inner.process_partial(Some(&[pending.as_slice()]), None) {
            Ok(mut channels) => output.append(&mut channels[0]),
            Err(error) => log::warn!("resample: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_follows_the_ratio() {
        let mut resampler = Resampler::new(48_000).unwrap();
        let mut output = Vec::new();
        for _ in 0..100 {
            resampler.push(&[0.25; 480], &mut output);
        }
        resampler.finish(&mut output);
        let expected = 48_000 / 3;
        assert!(output.len().abs_diff(expected) <= CHUNK, "{}", output.len());
    }
}
