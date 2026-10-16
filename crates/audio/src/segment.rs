//! Cuts a stream of 16 kHz frames into speech segments using a voice activity probability per
//! frame.

use std::collections::VecDeque;

use crate::{FRAME, RATE};

/// Silero's own advice: once speech has started, stay in it until the probability falls this
/// far below the threshold that started it.
const HYSTERESIS: f32 = 0.15;
/// Speech shorter than this is a click or a breath and never becomes a segment.
const MIN_SPEECH_MS: u32 = 160;

/// The timing a [`Segmenter`] cuts with, in the units the config uses.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub threshold: f32,
    pub silence_ms: u32,
    pub max_segment_s: u32,
    pub preroll_ms: u32,
}

/// A finished stretch of speech, 16 kHz mono.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub samples: Vec<f32>,
    /// The segment hit the length limit and was cut while the speaker was still talking.
    pub forced: bool,
}

/// What a frame did to the segmenter.
#[derive(Debug, PartialEq)]
pub enum Cut {
    None,
    /// Speech started in this frame. The pill and the insertion placeholder react to it.
    Started,
    Segment(Segment),
}

/// Groups frames into segments. Frames go in one at a time with their speech probability, and a
/// segment comes out once the speaker pauses for `silence_ms` or talks past `max_segment_s`.
pub struct Segmenter {
    timing: Timing,
    preroll: VecDeque<[f32; FRAME]>,
    current: Vec<f32>,
    speaking: bool,
    speech_frames: u32,
    silent_frames: u32,
}

impl Segmenter {
    pub fn new(timing: Timing) -> Self {
        Self {
            timing,
            preroll: VecDeque::new(),
            current: Vec::new(),
            speaking: false,
            speech_frames: 0,
            silent_frames: 0,
        }
    }

    pub fn push(&mut self, frame: &[f32; FRAME], probability: f32) -> Cut {
        let threshold = match self.speaking {
            true => self.timing.threshold - HYSTERESIS,
            false => self.timing.threshold,
        };
        let speech = probability >= threshold;

        if !self.speaking {
            if !speech {
                self.preroll.push_back(*frame);
                while self.preroll.len() > frames(self.timing.preroll_ms) {
                    self.preroll.pop_front();
                }
                return Cut::None;
            }
            self.speaking = true;
            self.silent_frames = 0;
            self.speech_frames = 0;
            self.current.clear();
            for frame in self.preroll.drain(..) {
                self.current.extend_from_slice(&frame);
            }
        }

        self.current.extend_from_slice(frame);
        let started = self.speech_frames == 0 && speech;
        match speech {
            true => {
                self.speech_frames += 1;
                self.silent_frames = 0;
            }
            false => self.silent_frames += 1,
        }

        if self.silent_frames as usize >= frames(self.timing.silence_ms) {
            return self.close(false).map_or(Cut::None, Cut::Segment);
        }
        if self.current.len() >= self.timing.max_segment_s as usize * RATE as usize {
            let segment = self.close(true);
            // The speaker is still mid-sentence, so the next frame continues a new segment
            // rather than waiting for fresh speech to cross the starting threshold.
            self.speaking = true;
            return segment.map_or(Cut::None, Cut::Segment);
        }
        match started {
            true => Cut::Started,
            false => Cut::None,
        }
    }

    /// Ends the segment in progress, if it holds enough speech to transcribe.
    pub fn flush(&mut self) -> Option<Segment> {
        self.speaking.then(|| self.close(false)).flatten()
    }

    fn close(&mut self, forced: bool) -> Option<Segment> {
        let speech = self.speech_frames;
        let mut samples = std::mem::take(&mut self.current);
        // Most of the trailing silence carries nothing, but a little keeps the last word's tail.
        let trailing = (self.silent_frames as usize).saturating_sub(frames(self.timing.preroll_ms));
        samples.truncate(samples.len() - trailing * FRAME);
        self.speaking = false;
        self.speech_frames = 0;
        self.silent_frames = 0;
        (speech as usize >= frames(MIN_SPEECH_MS)).then_some(Segment { samples, forced })
    }
}

/// How many whole frames cover `ms` milliseconds, rounded up.
fn frames(ms: u32) -> usize {
    (ms as usize * RATE as usize).div_ceil(1000 * FRAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing() -> Timing {
        Timing {
            threshold: 0.5,
            silence_ms: 500,
            max_segment_s: 2,
            preroll_ms: 200,
        }
    }

    fn feed(segmenter: &mut Segmenter, probability: f32, count: usize) -> Vec<Cut> {
        (0..count)
            .map(|_| segmenter.push(&[probability; FRAME], probability))
            .filter(|cut| *cut != Cut::None)
            .collect()
    }

    #[test]
    fn pause_ends_a_segment_with_preroll() {
        let mut segmenter = Segmenter::new(timing());
        assert!(feed(&mut segmenter, 0., 20).is_empty());
        assert_eq!(feed(&mut segmenter, 0.9, 30), vec![Cut::Started]);
        let cuts = feed(&mut segmenter, 0., frames(500));
        let [Cut::Segment(segment)] = cuts.as_slice() else {
            panic!("expected one segment, got {cuts:?}");
        };
        assert!(!segment.forced);
        let preroll = frames(200);
        assert_eq!(segment.samples.len(), (preroll + 30 + preroll) * FRAME);
        assert_eq!(segment.samples[0], 0.);
        assert_eq!(segment.samples[preroll * FRAME], 0.9);
    }

    #[test]
    fn long_speech_is_force_cut() {
        let mut segmenter = Segmenter::new(timing());
        let cuts = feed(&mut segmenter, 0.9, frames(2000) + 10);
        assert!(matches!(
            &cuts[1],
            Cut::Segment(Segment { forced: true, .. })
        ));
        assert!(segmenter.flush().is_some());
    }

    #[test]
    fn clicks_are_dropped() {
        let mut segmenter = Segmenter::new(timing());
        feed(&mut segmenter, 0.9, 2);
        assert!(feed(&mut segmenter, 0., frames(500)).is_empty());
        assert!(segmenter.flush().is_none());
    }
}
