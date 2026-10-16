//! The floating pill shown while a dictation session runs: cancel on the left, live bars in the
//! middle, confirm on the right. It lives on its own layer surface that never takes keyboard
//! focus, so the app being dictated into keeps it.

use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
use gpui::prelude::*;
use gpui::{
    App, Bounds, Context, Hsla, Pixels, Size, Window, WindowBackgroundAppearance, WindowBounds,
    WindowHandle, WindowKind, WindowOptions, canvas, div, point, px, svg,
};
use tokativ_audio::{BANDS, Levels};
use tokativ_config::{APP_ID, PillEdge};
use ui::ActiveTheme as _;

use crate::radius::nested_radius;

const HEIGHT_PX: f32 = 44.;
const HEIGHT: Pixels = px(HEIGHT_PX);
/// The gap between the pill's edge and its buttons, the same on every side a button touches.
const PADDING_PX: f32 = 6.;
const PADDING: Pixels = px(PADDING_PX);
/// The pill's outline, which counts towards the padding so the buttons sit exactly `PADDING`
/// from the outer edge.
const BORDER: Pixels = px(1.);
/// Room around the pill for its shadow. Clicks there fall through to whatever is below.
const SHADOW: Pixels = px(14.);
const BARS: usize = BANDS;
const BAR_WIDTH_PX: f32 = 3.;
const BAR_GAP_PX: f32 = 4.;
/// The space between each button and the nearest bar.
const BAR_MARGIN_PX: f32 = 12.;
/// Just wide enough for the two buttons and the bars between them.
const WIDTH: Pixels = px(PADDING_PX * 2.
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
}

/// Opens the pill on the focused output. `levels` feeds the bars, and the handlers run when
/// the buttons are clicked.
pub fn open(
    levels: Levels,
    edge: PillEdge,
    margin: f32,
    on_confirm: impl Fn(&mut App) + 'static,
    on_cancel: impl Fn(&mut App) + 'static,
    cx: &mut App,
) -> anyhow::Result<WindowHandle<Pill>> {
    let size = Size::new(WIDTH + SHADOW * 2., HEIGHT + SHADOW * 2.);
    // The shadow margin is part of the surface, so the visible gap to the edge takes it off.
    let gap = (px(margin) - SHADOW).max(Pixels::ZERO);
    let (anchor, margin) = match edge {
        PillEdge::Bottom => (Anchor::BOTTOM, (px(0.), px(0.), gap, px(0.))),
        PillEdge::Top => (Anchor::TOP, (gap, px(0.), px(0.), px(0.))),
    };
    let on_confirm: Handler = Rc::new(on_confirm);
    let on_cancel: Handler = Rc::new(on_cancel);
    cx.open_window(
        WindowOptions {
            titlebar: None,
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size,
            })),
            app_id: Some(APP_ID.into()),
            window_background: WindowBackgroundAppearance::Transparent,
            // The pill never has focus, and without this its bars would drop to the throttled
            // frame rate GPUI gives inactive windows.
            inactive_frame_interval: None,
            kind: WindowKind::LayerShell(LayerShellOptions {
                namespace: APP_ID.into(),
                layer: Layer::Overlay,
                anchor,
                exclusive_zone: Some(px(0.)),
                margin: Some(margin),
                keyboard_interactivity: KeyboardInteractivity::None,
                ..Default::default()
            }),
            ..Default::default()
        },
        move |window, cx| {
            window.set_input_region(Some(&[Bounds {
                origin: point(SHADOW, SHADOW),
                size: Size::new(WIDTH, HEIGHT),
            }]));
            cx.new(|_| Pill {
                levels,
                phase: Phase::Listening,
                shown: [0.; BARS],
                beat: None,
                since: Instant::now(),
                on_confirm,
                on_cancel,
            })
        },
    )
}

impl Pill {
    pub fn set_phase(&mut self, phase: Phase, window: &mut Window, cx: &mut Context<Self>) {
        if self.phase == phase {
            return;
        }
        self.phase = phase;
        self.since = Instant::now();
        if phase == Phase::Error {
            cx.spawn_in(window, async move |_, cx| {
                cx.background_executor().timer(ERROR_HOLD).await;
                cx.update(|window, _| window.remove_window()).ok();
            })
            .detach();
        }
        cx.notify();
    }

    /// Eases the drawn bars towards the latest levels at a rate that follows the real time
    /// between frames, so the motion looks the same at any refresh rate.
    fn step(&mut self) {
        let now = Instant::now();
        let elapsed = self
            .beat
            .replace(now)
            .map_or(Duration::ZERO, |beat| now - beat);
        let rate = 1. - (-elapsed.as_secs_f32() / SETTLE.as_secs_f32()).exp();
        let target = match self.phase {
            Phase::Listening => self.levels.read(),
            Phase::Processing | Phase::Error => [0.; BARS],
        };
        for (shown, target) in self.shown.iter_mut().zip(target) {
            *shown += (target - *shown) * rate;
        }
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
        self.step();
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
        let on_confirm = self.on_confirm.clone();
        let on_cancel = self.on_cancel.clone();
        let listening = self.phase == Phase::Listening;

        div().size_full().p(SHADOW).font_family("Inter").child(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_between()
                .p(PADDING - BORDER)
                .rounded(radius)
                .bg(surface)
                .border_1()
                .border_color(theme.border)
                .shadow_lg()
                .child(
                    circle("cancel", button, theme.secondary, theme.secondary_hover)
                        .child(icon("icons/x.svg", theme.foreground))
                        .on_click(move |_, _, cx| on_cancel(cx)),
                )
                .child(bars(shown, bar_color).flex_1().h_full())
                .child(
                    circle(
                        "confirm",
                        button,
                        theme.foreground,
                        theme.foreground.opacity(0.85),
                    )
                    .when(!listening, |this| this.opacity(0.5))
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
/// left. A silent band draws a dot as tall as the bar is wide, so silence reads as a flat dotted
/// line. Every edge lands on a whole device pixel, so at fractional scales all bars come out the
/// same width and the dots sit on one line.
fn bars(levels: [f32; BARS], color: Hsla) -> gpui::Div {
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
                for (index, level) in levels.into_iter().enumerate() {
                    // Rounding to the bar's own parity keeps every bar centered on the same line.
                    let mut height = (reach * level.clamp(0., 1.)).round().max(width);
                    if (height - width) % 2. != 0. {
                        height += 1.;
                    }
                    let x = left + (width + gap) * index as f32;
                    let top = middle - ((height - width) / 2.) - (width / 2.).floor();
                    let bar = Bounds {
                        origin: point(px(x / scale), px(top / scale)),
                        size: Size::new(px(width / scale), px(height / scale)),
                    };
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
