//! Band levels of the microphone for the pill's bars. The analysis is Sonora's
//! `music::spectrum` cut down to one mono channel, with half-overlapping windows so the bars move
//! at the display's pace rather than every 43 ms.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

/// How many bands the analyzer splits the voice range into. The pill mirrors them around its
/// center, so it draws twice as many bars.
pub const BANDS: usize = 8;
const FFT_SIZE: usize = 1024;
const HOP: usize = FFT_SIZE / 2;
/// Speech carries little below 100 Hz and its consonants thin out above 6 kHz.
const MIN_FREQ: f32 = 100.;
const MAX_FREQ: f32 = 6_000.;
/// A microphone delivers far less level than a mastered track, so it is lifted harder than
/// Sonora lifts music.
const GAIN: f32 = 24.;
/// The level past which a band is compressed towards the ceiling instead of cut at it.
const KNEE: f32 = 0.6;
const ATTACK: f32 = 0.7;
const DECAY: f32 = 0.12;

/// The latest band levels in 0..1, written by the analyzer and read by the UI at any time.
#[derive(Clone)]
pub struct Levels {
    bands: Arc<[AtomicU32; BANDS]>,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            bands: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
        }
    }
}

impl Levels {
    pub fn read(&self) -> [f32; BANDS] {
        std::array::from_fn(|index| f32::from_bits(self.bands[index].load(Ordering::Relaxed)))
    }

    /// The loudest band, for a single level meter.
    pub fn peak(&self) -> f32 {
        self.read().into_iter().fold(0., f32::max)
    }

    pub(crate) fn clear(&self) {
        for band in self.bands.iter() {
            band.store(0f32.to_bits(), Ordering::Relaxed);
        }
    }

    fn set(&self, index: usize, value: f32) {
        self.bands[index].store(value.to_bits(), Ordering::Relaxed);
    }
}

/// Runs the FFT over the samples it is fed and publishes smoothed bands into [`Levels`].
pub(crate) struct Analyzer {
    levels: Levels,
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    edges: Vec<usize>,
    samples: Vec<f32>,
    buffer: Vec<Complex32>,
    scratch: Vec<Complex32>,
    smoothed: [f32; BANDS],
}

impl Analyzer {
    pub fn new(rate: u32, levels: Levels) -> Self {
        let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
        let scratch = vec![Complex32::default(); fft.get_inplace_scratch_len()];
        Self {
            levels,
            fft,
            window: hann_window(),
            edges: band_edges(rate as f32),
            samples: Vec::with_capacity(FFT_SIZE * 2),
            buffer: vec![Complex32::default(); FFT_SIZE],
            scratch,
            smoothed: [0.; BANDS],
        }
    }

    pub fn push(&mut self, input: &[f32]) {
        self.samples.extend_from_slice(input);
        while self.samples.len() >= FFT_SIZE {
            self.analyze();
            self.samples.drain(..HOP);
        }
    }

    fn analyze(&mut self) {
        for ((slot, sample), weight) in self.buffer.iter_mut().zip(&self.samples).zip(&self.window)
        {
            *slot = Complex32::new(sample * weight, 0.);
        }
        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);

        for (band, edge) in self.edges.windows(2).enumerate() {
            let low = edge[0];
            let high = edge[1].max(low + 1);
            let magnitude = self.buffer[low..high]
                .iter()
                .map(|bin| bin.norm())
                .fold(0f32, f32::max);
            let target = soften((magnitude * GAIN / (FFT_SIZE as f32 / 2.)).sqrt());
            let rate = match target > self.smoothed[band] {
                true => ATTACK,
                false => DECAY,
            };
            self.smoothed[band] += (target - self.smoothed[band]) * rate;
            self.levels.set(band, self.smoothed[band]);
        }
    }
}

fn hann_window() -> Vec<f32> {
    (0..FFT_SIZE)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (FFT_SIZE - 1) as f32).cos())
        .collect()
}

fn band_edges(rate: f32) -> Vec<usize> {
    let max_freq = MAX_FREQ.min(rate / 2.);
    let min_freq = MIN_FREQ.min(max_freq * 0.5).max(1.);
    let bin_hz = rate / FFT_SIZE as f32;
    (0..=BANDS)
        .map(|i| {
            let freq = min_freq * (max_freq / min_freq).powf(i as f32 / BANDS as f32);
            ((freq / bin_hz) as usize).clamp(1, FFT_SIZE / 2 - 1)
        })
        .collect()
}

/// A band's level on a curve that is linear up to `KNEE` and eases towards 1 above it without
/// reaching it, so two loud bands still land at different heights.
fn soften(level: f32) -> f32 {
    match level <= KNEE {
        true => level,
        false => KNEE + (1. - KNEE) * ((level - KNEE) / (1. - KNEE)).tanh(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tone_lights_its_band() {
        let levels = Levels::default();
        let mut analyzer = Analyzer::new(48_000, levels.clone());
        let tone: Vec<f32> = (0..48_000)
            .map(|i| 0.3 * (std::f32::consts::TAU * 1_000. * i as f32 / 48_000.).sin())
            .collect();
        analyzer.push(&tone);
        let bands = levels.read();
        let loudest = (0..BANDS)
            .max_by(|a, b| bands[*a].total_cmp(&bands[*b]))
            .unwrap();
        let edges = band_edges(48_000.);
        let bin = (1_000. / (48_000. / FFT_SIZE as f32)) as usize;
        assert!(
            edges[loudest] <= bin && bin <= edges[loudest + 1],
            "{bands:?}"
        );
    }

    #[test]
    fn silence_stays_flat() {
        let levels = Levels::default();
        Analyzer::new(48_000, levels.clone()).push(&[0.; 8_192]);
        assert_eq!(levels.peak(), 0.);
    }
}
