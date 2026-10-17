//! Band levels of the microphone for the pill's bars. The FFT and band layout are Sonora's
//! `music::spectrum` cut down to one mono channel, with half-overlapping windows so the bars move
//! at the display's pace. Levels are judged in decibels against the room's own noise and the
//! speaker's own loudness, so a quiet microphone moves the bars as much as a loud one.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};

/// How many bands the analyzer splits the voice range into, one per bar in the pill.
pub const BANDS: usize = 10;
const FFT_SIZE: usize = 1024;
const HOP: usize = FFT_SIZE / 2;
/// Speech carries little below 100 Hz and its consonants thin out above 6 kHz.
const MIN_FREQ: f32 = 100.;
const MAX_FREQ: f32 = 6_000.;
/// How far above the room's noise a band must rise before its bar leaves the idle line.
const NOISE_MARGIN: f32 = 10.;
/// The noise estimate drops at once to a quieter window and creeps up this many decibels per
/// window otherwise, so speech never gets mistaken for noise.
const NOISE_RISE: f32 = 0.02;
const NOISE_RANGE: (f32, f32) = (-110., -45.);
/// The loudness estimate jumps to a louder window and falls this many decibels per window,
/// about 9 dB a second, so the bars keep their reach as the speaker gets quieter.
const LOUDNESS_FALL: f32 = 0.1;
/// The least span between the idle line and full height. Without it, steady noise would be
/// stretched to fill the bars.
const MIN_SPAN: f32 = 24.;
/// The level past which a band is compressed towards the ceiling instead of cut at it.
const KNEE: f32 = 0.6;
const ATTACK: f32 = 0.7;
const DECAY: f32 = 0.12;
/// The loudness reported for silence, in dBFS.
const SILENT: f32 = -120.;

/// The latest band levels in 0..1, written by the analyzer and read by the UI at any time.
#[derive(Clone)]
pub struct Levels {
    bands: Arc<[AtomicU32; BANDS]>,
    /// The RMS level of the latest window in dBFS, unscaled, for a level meter.
    loudness: Arc<AtomicU32>,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            bands: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
            loudness: Arc::new(AtomicU32::new(SILENT.to_bits())),
        }
    }
}

impl Levels {
    pub fn read(&self) -> [f32; BANDS] {
        std::array::from_fn(|index| f32::from_bits(self.bands[index].load(Ordering::Relaxed)))
    }

    /// The loudest band.
    pub fn peak(&self) -> f32 {
        self.read().into_iter().fold(0., f32::max)
    }

    /// How loud the input is in dBFS, as a level meter shows it. Unlike the bands it is not
    /// scaled to the room, so a quiet microphone reads quiet.
    pub fn loudness(&self) -> f32 {
        f32::from_bits(self.loudness.load(Ordering::Relaxed))
    }

    /// Puts `bands` on show as they are, without any analysis. For previews of the pill.
    pub fn show(&self, bands: [f32; BANDS]) {
        for (index, value) in bands.into_iter().enumerate() {
            self.set(index, value);
        }
    }

    pub(crate) fn clear(&self) {
        for band in self.bands.iter() {
            band.store(0f32.to_bits(), Ordering::Relaxed);
        }
        self.loudness.store(SILENT.to_bits(), Ordering::Relaxed);
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
    /// The room's noise in dBFS, as the quietest recent windows put it.
    noise: f32,
    /// The speaker's recent loudness in dBFS.
    loudness: f32,
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
            noise: NOISE_RANGE.1,
            loudness: NOISE_RANGE.1 + MIN_SPAN,
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
        let window = &self.samples[..FFT_SIZE];
        let rms =
            (window.iter().map(|sample| sample * sample).sum::<f32>() / FFT_SIZE as f32).sqrt();
        let loudness = (20. * rms.max(1e-6).log10()).max(SILENT);
        self.levels
            .loudness
            .store(loudness.to_bits(), Ordering::Relaxed);

        for ((slot, sample), weight) in self.buffer.iter_mut().zip(&self.samples).zip(&self.window)
        {
            *slot = Complex32::new(sample * weight, 0.);
        }
        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);

        let mut decibels = [0f32; BANDS];
        for (band, edge) in self.edges.windows(2).enumerate() {
            let low = edge[0];
            let high = edge[1].max(low + 1);
            let magnitude = self.buffer[low..high]
                .iter()
                .map(|bin| bin.norm())
                .fold(0f32, f32::max);
            decibels[band] = dbfs(magnitude);
        }
        let loudest = decibels.into_iter().fold(f32::MIN, f32::max);
        self.noise = match loudest < self.noise {
            true => loudest,
            false => self.noise + NOISE_RISE,
        }
        .clamp(NOISE_RANGE.0, NOISE_RANGE.1);
        let floor = self.noise + NOISE_MARGIN;
        self.loudness = loudest
            .max(self.loudness - LOUDNESS_FALL)
            .max(floor + MIN_SPAN);
        let span = self.loudness - floor;

        for (band, decibels) in decibels.into_iter().enumerate() {
            let target = soften(((decibels - floor) / span).clamp(0., 1.));
            let rate = match target > self.smoothed[band] {
                true => ATTACK,
                false => DECAY,
            };
            self.smoothed[band] += (target - self.smoothed[band]) * rate;
            self.levels.set(band, self.smoothed[band]);
        }
    }
}

/// A peak bin's magnitude as the level of the sine that would produce it, in dBFS. A Hann window
/// sums to half the window length, so a full-scale sine peaks at a quarter of it.
fn dbfs(magnitude: f32) -> f32 {
    20. * (magnitude / (FFT_SIZE as f32 / 4.)).max(1e-6).log10()
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
    fn a_quiet_voice_still_fills_the_bars() {
        let levels = Levels::default();
        let mut analyzer = Analyzer::new(48_000, levels.clone());
        let noise: Vec<f32> = (0..48_000)
            .map(|i| 0.000_02 * (i as f32 * 12.9898).sin().fract())
            .collect();
        analyzer.push(&noise);
        let amplitude = 10f32.powf(-55. / 20.);
        let tone: Vec<f32> = (0..24_000)
            .map(|i| amplitude * (std::f32::consts::TAU * 400. * i as f32 / 48_000.).sin())
            .collect();
        analyzer.push(&tone);
        assert!(levels.peak() > 0.5, "{:?}", levels.read());
    }

    #[test]
    fn silence_stays_flat() {
        let levels = Levels::default();
        Analyzer::new(48_000, levels.clone()).push(&[0.; 8_192]);
        assert_eq!(levels.peak(), 0.);
    }
}
