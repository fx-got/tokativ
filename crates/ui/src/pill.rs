//! The floating pill shown while a dictation session runs: cancel on the left, live bars in the
//! middle, confirm on the right. It lives on its own layer surface on Wayland and in a floating
//! window elsewhere, and never takes keyboard focus, so the app being dictated into keeps it.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, Hsla, Pixels, Size, SpringConfig, Window, WindowHandle, canvas, div,
    point, px, svg,
};
use tokativ_audio::{BANDS, Levels};
use tokativ_config::PillEdge;
use ui::ActiveTheme as _;

use crate::overlay;
use crate::radius::nested_radius;
use crate::spring::Spring;

const HEIGHT_PX: f32 = 44.;
pub(crate) const HEIGHT: Pixels = px(HEIGHT_PX);
/// The gap between the pill's edge and its buttons, the same on every side a button touches.
const PADDING_PX: f32 = 6.;
const PADDING: Pixels = px(PADDING_PX);
/// The pill's outline, which counts towards the padding so the buttons sit exactly `PADDING`
/// from the outer edge.
const BORDER: Pixels = px(1.);
/// Covers shadow_lg's offset and full Gaussian tail (10 - 3 + 3 * 15 = 52px),
/// plus room for the opening spring. Clicks in this margin pass through.
pub(crate) const SHADOW: Pixels = px(56.);
const BARS: usize = BANDS;
const BAR_WIDTH_PX: f32 = 3.;
const BAR_GAP_PX: f32 = 4.;
/// The space between each button and the nearest bar.
const BAR_MARGIN_PX: f32 = 12.;
/// Just wide enough for the two buttons and the bars between them.
pub(crate) const WIDTH: Pixels = px(PADDING_PX * 2.
    + (HEIGHT_PX - PADDING_PX * 2.) * 2.
    + BAR_MARGIN_PX * 2.
    + BAR_WIDTH_PX * BARS as f32
    + BAR_GAP_PX * (BARS - 1) as f32);
/// The tallest a bar grows, as a share of the pill's height.
const BAR_REACH: f32 = 0.62;
/// How long the bars take to settle a change, as the time constant of their easing.
const SETTLE: Duration = Duration::from_millis(45);
const PULSE: Duration = Duration::from_millis(1100);
const ERROR_HOLD: Duration = Duration::from_millis(1400);
const ICON: Pixels = px(16.);
/// The capsule's opening: quick, with a little overshoot so it lands with a bounce.
const OPEN: SpringConfig = SpringConfig::new(320., 19., 1.);
/// The buttons popping in once the capsule has room for them.
const POP: SpringConfig = SpringConfig::new(420., 22., 1.);
/// Closing is near critically damped, so the capsule folds away without wobbling.
const CLOSE: SpringConfig = SpringConfig::new(520., 44., 1.);
/// How long after opening the buttons start to pop in.
const CONTROLS_DELAY: Duration = Duration::from_millis(70);
/// When the middle bars light up, and how much later each bar further out follows.
const BARS_DELAY: Duration = Duration::from_millis(110);
const BARS_STAGGER: Duration = Duration::from_millis(24);
const BAR_REVEAL: Duration = Duration::from_millis(200);
/// How fast the bars go out when the pill closes, before the capsule folds.
const BARS_FADE: Duration = Duration::from_millis(90);
/// How much of the spring's overshoot past full width shows as growth in scale, once the
/// capsule has reached its full width.
const OVERSHOOT_SCALE: f32 = 0.5;
/// The longest a close may take before the window goes regardless.
const CLOSE_LIMIT: Duration = Duration::from_millis(700);
/// How small and how blurred the capsule is when folded.
const FOLDED_SCALE: f32 = 0.82;
const FOLDED_BLUR: Pixels = px(8.);
/// Where the pill takes clicks: the capsule at full width, not the room around it for the shadow.
const REGION: Bounds<Pixels> = Bounds {
    origin: point(SHADOW, SHADOW),
    size: Size {
        width: WIDTH,
        height: HEIGHT,
    },
};

/// What the session is doing, which is what the bars show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Listening,
    /// Capture has stopped and the last segments are being transcribed.
    Processing,
    /// Something failed. The pill tints red and closes itself shortly after.
    Error,
}

type Handler = Rc<dyn Fn(&mut App)>;

pub struct Pill {
    levels: Levels,
    phase: Phase,
    shown: [f32; BARS],
    beat: Option<Instant>,
    since: Instant,
    on_confirm: Handler,
    on_cancel: Handler,
    opened: Instant,
    /// When the pill started to close, if it has.
    closing: Option<Instant>,
    /// How far the capsule has unfolded, from a dot at 0 to full width at 1.
    body: Spring,
    /// How far the buttons have popped in.
    controls: Spring,
    /// The pill has folded away and waits, invisible and click-through, for the next session.
    /// Keeping the surface instead of closing it lets the next session show its first frame at
    /// once, where a new layer surface takes over a hundred milliseconds to appear.
    resting: bool,
    /// The capsule has reached its full width since it last started to open. From then on its
    /// width holds and the spring's bounce shows as scale, which moves by fractions of a pixel
    /// where the width can only step by whole ones.
    widened: bool,
}

/// Opens the pill on the focused output. `levels` feeds the bars, and the handlers run when
/// the buttons are clicked. A `resting` pill starts folded away, ready to unfold at once.
pub fn open(
    levels: Levels,
    edge: PillEdge,
    margin: f32,
    on_confirm: impl Fn(&mut App) + 'static,
    on_cancel: impl Fn(&mut App) + 'static,
    resting: bool,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Pill>> {
    let size = Size::new(WIDTH + SHADOW * 2., HEIGHT + SHADOW * 2.);
    // The shadow margin is part of the surface, so the visible gap to the edge takes it off.
    let gap = px(margin) - SHADOW;
    let options = overlay::options(edge, gap, size, None, cx);
    let on_confirm: Handler = Rc::new(on_confirm);
    let on_cancel: Handler = Rc::new(on_cancel);
    cx.open_window(options, move |window, cx| {
        overlay::prepare(window, false);
        window.set_input_region(Some(match resting {
            true => &[],
            false => &[REGION],
        }));
        cx.new(|_| Pill {
            levels,
            phase: Phase::Listening,
            shown: [0.; BARS],
            beat: None,
            since: Instant::now(),
            on_confirm,
            on_cancel,
            opened: Instant::now(),
            closing: None,
            body: Spring::new(OPEN),
            controls: Spring::new(POP),
            resting,
            widened: false,
        })
    })
}

impl Pill {
    pub fn set_phase(&mut self, phase: Phase, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase == phase {
            return;
        }
        self.phase = phase;
        self.since = Instant::now();
        if phase == Phase::Error {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(ERROR_HOLD).await;
                this.update_in(cx, |pill, _, cx| pill.dismiss(cx)).ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Unfolds the pill again for a new session, from wherever its folding has got to, so a
    /// quick second press turns the motion around instead of starting over.
    pub fn reopen(&mut self, levels: Levels, window: &mut Window, cx: &mut Context<Self>) {
        let folded = self.closing.take().is_some() || self.resting;
        if self.resting {
            self.resting = false;
            window.set_input_region(Some(&[REGION]));
        }
        self.levels = levels;
        self.phase = Phase::Listening;
        self.since = Instant::now();
        if folded {
            self.opened = Instant::now();
            self.widened = false;
        }
        cx.notify();
    }

    /// Folds the pill away. Once folded it rests, invisible, until [`Pill::reopen`].
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.closing.is_none() {
            self.closing = Some(Instant::now());
            self.body.aim(0., CLOSE);
            self.controls.aim(0., CLOSE);
            cx.notify();
        }
    }

    /// Advances the opening or closing motion. Returns false once a close has finished.
    fn animate(&mut self, elapsed: Duration, still: bool) -> bool {
        match self.closing {
            None => {
                self.body.aim(1., OPEN);
                if self.opened.elapsed() >= CONTROLS_DELAY {
                    self.controls.aim(1., POP);
                }
            }
            Some(closing) if closing.elapsed() < BARS_FADE => {
                // The bars go out first, then the buttons and the capsule fold.
                self.controls.target = self.controls.at();
                self.body.target = self.body.at();
            }
            Some(_) => {
                self.controls.aim(0., CLOSE);
                self.body.aim(0., CLOSE);
            }
        }
        if still {
            self.body.finish();
            self.controls.finish();
        } else {
            self.body.step(elapsed);
            self.controls.step(elapsed);
        }
        let Some(closing) = self.closing else {
            return true;
        };
        let folded = closing.elapsed() >= BARS_FADE && self.body.at() < 0.02 && self.body.settled();
        !(folded || still || closing.elapsed() >= CLOSE_LIMIT)
    }

    /// How much of each bar shows: they light up from the middle outwards after opening and all
    /// go out together on close.
    fn reveal(&self) -> [f32; BARS] {
        let opened = self.opened.elapsed().as_secs_f32();
        let fade = self.closing.map_or(1., |closing| {
            1. - (closing.elapsed().as_secs_f32() / BARS_FADE.as_secs_f32()).min(1.)
        });
        let middle = (BARS - 1) as f32 / 2.;
        std::array::from_fn(|index| {
            let delay = BARS_DELAY.as_secs_f32()
                + BARS_STAGGER.as_secs_f32() * (index as f32 - middle).abs().floor();
            let t = ((opened - delay) / BAR_REVEAL.as_secs_f32()).clamp(0., 1.);
            ui::ease_out_cubic(t) * fade
        })
    }

    /// Eases the drawn bars towards the latest levels at a rate that follows the real time
    /// between frames, so the motion looks the same at any refresh rate.
    fn step(&mut self) -> Duration {
        let now = Instant::now();
        let elapsed = self.beat.replace(now).map_or(Duration::ZERO, |beat| {
            (now - beat).min(Duration::from_millis(50))
        });
        let rate = 1. - (-elapsed.as_secs_f32() / SETTLE.as_secs_f32()).exp();
        let target = match self.phase {
            Phase::Listening => self.levels.read(),
            Phase::Processing | Phase::Error => [0.; BARS],
        };
        for (shown, target) in self.shown.iter_mut().zip(target) {
            *shown += (target - *shown) * rate;
        }
        elapsed
    }

    /// How opaque the bars are, which pulses while the last segments are transcribed.
    fn intensity(&self) -> f32 {
        match self.phase {
            Phase::Listening => 1.,
            Phase::Processing => {
                let t = self.since.elapsed().as_secs_f32() / PULSE.as_secs_f32();
                0.35 + 0.3 * (0.5 - 0.5 * (t * std::f32::consts::TAU).cos())
            }
            Phase::Error => 0.5,
        }
    }
}

impl Render for Pill {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.resting {
            return div().size_full();
        }
        let elapsed = self.step();
        if !self.animate(elapsed, cx.reduce_motion()) {
            // Folded: clear the surface, stop taking clicks and stop asking for frames. A floating
            // window cannot let clicks through, so off Linux it closes and the next session opens
            // a new one.
            self.resting = true;
            self.beat = None;
            window.set_input_region(Some(&[]));
            #[cfg(not(target_os = "linux"))]
            window.remove_window();
            return div().size_full();
        }
        window.request_animation_frame();

        let theme = cx.theme();
        let radius = HEIGHT / 2.;
        let button = nested_radius(radius, PADDING) * 2.;
        let error = self.phase == Phase::Error;
        let surface = match error {
            true => mix(theme.background, theme.danger, 0.35),
            false => theme.background,
        };
        let bar_color = theme.foreground.opacity(self.intensity());
        let shown = self.shown;
        let reveal = self.reveal();
        let on_confirm = self.on_confirm.clone();
        let on_cancel = self.on_cancel.clone();
        let listening = self.phase == Phase::Listening;

        // The capsule unfolds sideways from a dot, growing and sharpening as it goes. The spring
        // overshoots a little, so it lands slightly wide and settles back.
        let body = self.body.at().max(0.);
        let unfolded = body.min(1.);
        let opening = self.closing.is_none();
        if opening && body >= 1. {
            self.widened = true;
        }
        let (spread, scale) = match opening && self.widened {
            true => (1., 1. + (body - 1.) * OVERSHOOT_SCALE),
            false => (unfolded, FOLDED_SCALE + (1. - FOLDED_SCALE) * body),
        };
        let width = ui::snapped(HEIGHT + (WIDTH - HEIGHT) * spread, window);
        let controls = self.controls.at().max(0.);
        let pop = (0.4 + 0.6 * controls).max(0.01);
        let control_opacity = controls.min(1.);
        // At rest the capsule is drawn plainly. Any layer effect left on, even at a scale of
        // 0.9999, resamples it and shifts its edges by a fraction of a pixel.
        let moving = body != 1. || controls != 1.;

        // The capsule is placed on whole device pixels. Centring it with flex would put it on a
        // half pixel whenever the room around it is an odd number of them.
        let left = ui::snapped((window.viewport_size().width - width) / 2., window);
        let top = ui::snapped(SHADOW, window);

        div().size_full().relative().font_family("Inter").child(
            div()
                .absolute()
                .left(left)
                .top(top)
                .w(width)
                .h(HEIGHT)
                .flex()
                .items_center()
                .justify_between()
                .p(PADDING - BORDER)
                .rounded(radius)
                .bg(surface)
                .border_1()
                .border_color(theme.border)
                .shadow_lg()
                .overflow_hidden()
                .when(moving, |this| {
                    this.opacity((unfolded * 2.5).min(1.))
                        .layer_scale(scale.max(0.01))
                })
                .when(unfolded < 1., |this| {
                    this.blur(FOLDED_BLUR * (1. - unfolded))
                })
                .child(
                    circle("cancel", button, theme.secondary, theme.secondary_hover)
                        .when(moving, |this| {
                            this.opacity(control_opacity).layer_scale(pop)
                        })
                        .child(icon("icons/x.svg", theme.foreground))
                        .on_click(move |_, _, cx| on_cancel(cx)),
                )
                .child(bars(shown, reveal, bar_color).flex_1().h_full())
                .child(
                    circle(
                        "confirm",
                        button,
                        theme.foreground,
                        theme.foreground.opacity(0.85),
                    )
                    .opacity(match listening {
                        true => control_opacity,
                        false => control_opacity * 0.5,
                    })
                    .when(moving, |this| this.layer_scale(pop))
                    .child(icon("icons/check.svg", theme.background))
                    .on_click(move |_, _, cx| on_confirm(cx)),
                ),
        )
    }
}

fn circle(id: &'static str, size: Pixels, fill: Hsla, hover: Hsla) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex_none()
        .size(size)
        .rounded(size / 2.)
        .flex()
        .items_center()
        .justify_center()
        .bg(fill)
        .hover(move |style| style.bg(hover))
        .cursor_pointer()
}

fn icon(path: &'static str, color: Hsla) -> impl IntoElement {
    svg().path(icons::path(path)).size(ICON).text_color(color)
}

/// Paints the bars as rounded rectangles centered on the pill's midline, low frequencies on the
/// left. `reveal` says how much of each bar shows while the pill opens and closes. A silent band draws a dot as tall as the bar is wide, so silence reads as a flat dotted
/// line. Every edge lands on a whole device pixel, so at fractional scales all bars come out the
/// same width and the dots sit on one line.
fn bars(levels: [f32; BARS], reveal: [f32; BARS], color: Hsla) -> gpui::Div {
    div().child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let scale = window.scale_factor();
                let device = |logical: f32| (logical * scale).round();
                let width = device(BAR_WIDTH_PX).max(1.);
                let gap = device(BAR_GAP_PX).max(1.);
                let span = width * BARS as f32 + gap * (BARS - 1) as f32;
                let left = device(bounds.origin.x.as_f32())
                    + ((device(bounds.size.width.as_f32()) - span) / 2.).round();
                let middle = device(bounds.origin.y.as_f32() + bounds.size.height.as_f32() / 2.);
                let reach = bounds.size.height.as_f32() * BAR_REACH * scale;
                for (index, (level, shown)) in levels.into_iter().zip(reveal).enumerate() {
                    if shown <= 0. {
                        continue;
                    }
                    // Rounding to the bar's own parity keeps every bar centered on the same line.
                    let mut height = (reach * level.clamp(0., 1.) * shown).round().max(width);
                    if (height - width) % 2. != 0. {
                        height += 1.;
                    }
                    let x = left + (width + gap) * index as f32;
                    let top = middle - ((height - width) / 2.) - (width / 2.).floor();
                    let bar = Bounds {
                        origin: point(px(x / scale), px(top / scale)),
                        size: Size::new(px(width / scale), px(height / scale)),
                    };
                    let color = color.opacity(shown);
                    window.paint_quad(gpui::fill(bar, color).corner_radii(px(width / scale / 2.)));
                }
            },
        )
        .size_full(),
    )
}

fn mix(from: Hsla, to: Hsla, amount: f32) -> Hsla {
    let from = from.to_rgb();
    let to = to.to_rgb();
    gpui::Rgba {
        r: from.r + (to.r - from.r) * amount,
        g: from.g + (to.g - from.g) * amount,
        b: from.b + (to.b - from.b) * amount,
        a: from.a + (to.a - from.a) * amount,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_follow_the_radius_rule() {
        let radius = HEIGHT / 2.;
        let button = nested_radius(radius, PADDING) * 2.;
        assert_eq!(button + PADDING * 2., HEIGHT);
    }
}
