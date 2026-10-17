//! A value animated by a spring, which the pill and the caption move their shapes and words with.

use std::time::Duration;

use gpui::{SpringConfig, SpringState};

/// How close to its target, and how slow, a spring must be to count as at rest. Fine enough that
/// landing on the exact target moves nothing by a visible fraction of a pixel.
const REST: f32 = 0.001;

/// One animated value driven by a spring towards its target.
pub(crate) struct Spring {
    pub config: SpringConfig,
    pub state: SpringState,
    pub target: f32,
}

impl Spring {
    pub fn new(config: SpringConfig) -> Self {
        Self {
            config,
            state: SpringState {
                position: 0.,
                velocity: 0.,
            },
            target: 0.,
        }
    }

    pub fn aim(&mut self, target: f32, config: SpringConfig) {
        self.target = target;
        self.config = config;
    }

    /// Moves the value on by `elapsed`. Once it has come to rest it lands exactly on the target,
    /// since a value left a hair off would keep a shape on fractional pixels for good.
    pub fn step(&mut self, elapsed: Duration) {
        self.state = self
            .config
            .step(self.state, self.target, elapsed.as_secs_f32());
        if self.settled() {
            self.state = SpringState {
                position: self.target,
                velocity: 0.,
            };
        }
    }

    pub fn at(&self) -> f32 {
        self.state.position
    }

    pub fn settled(&self) -> bool {
        self.config.is_settled(self.state, self.target, REST)
    }

    /// Land immediately, including cancelling velocity, for reduced motion.
    pub fn finish(&mut self) {
        self.state = SpringState {
            position: self.target,
            velocity: 0.,
        };
    }
}
