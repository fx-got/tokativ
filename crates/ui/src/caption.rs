//! The caption beside the pill: what the session has said so far and a draft of the phrase being
//! spoken, on any platform. It sits away from the screen edge, lets clicks through, and every
//! new or changed word rises out of a blur on a spring.

use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    AvailableSpace, Context, DisplayId, Hsla, Pixels, ScrollHandle, Size, SpringConfig, TextRun,
    Window, WindowHandle, div, font, point, px,
};
use tokativ_config::PillEdge;
use ui::ActiveTheme as _;

use crate::overlay;
use crate::pill;
use crate::spring::Spring;

/// The caption's width, wider than the pill so a phrase has room.
const WIDTH: Pixels = px(520.);
/// Three readable lines in a card larger than the controls. Older lines move up out of view.
const LINE_HEIGHT: Pixels = px(24.);
const TEXT_HEIGHT: Pixels = px(72.);
const TEXT_SIZE: Pixels = px(16.);
const PADDING_X: Pixels = px(20.);
const PADDING_Y: Pixels = px(14.);
const BORDER: Pixels = px(1.);
/// Draw and fade into the card's existing padding, so scrolling lines and word blur never
/// meet a hard clip at the text area's edge. This does not add to the card's outer size.
const TEXT_BLEED: Pixels = px(10.);
const RADIUS: Pixels = px(20.);
/// The space between the pill and the caption.
const GAP: Pixels = px(10.);
const WORD_GAP: Pixels = px(5.);
/// How far a word rises, and how blurred it starts, as it comes in.
const WORD_RISE: Pixels = px(6.);
const WORD_BLUR: Pixels = px(8.);
const WORD_STAGGER: Duration = Duration::from_millis(30);
const MAX_STAGGER: Duration = Duration::from_millis(180);
/// Keep about this many words, removing only complete rows that have already scrolled away.
const MAX_WORDS: usize = 128;
/// The caption itself unfolding towards the pill and folding back.
const FOLDED_SCALE: f32 = 0.9;
const FOLDED_BLUR: Pixels = px(10.);
const FOLDED_SHIFT: Pixels = px(10.);
/// A soft bounce in position, with a separate, overdamped reveal so opacity and blur never
/// snap to their endpoints when the positional spring first overshoots.
const WORD: SpringConfig = SpringConfig::new(230., 24., 1.);
const CLARITY: SpringConfig = SpringConfig::new(180., 28., 1.);
const INK: SpringConfig = SpringConfig::new(160., 27., 1.);
/// Damped enough to resize without overshooting the text's required bounds.
const RESIZE: SpringConfig = SpringConfig::new(210., 30., 1.);
/// Scroll the existing lines together, with no bounce past the last line.
const SCROLL: SpringConfig = SpringConfig::new(210., 30., 1.);
const OPEN: SpringConfig = SpringConfig::new(320., 22., 1.);
const CLOSE: SpringConfig = SpringConfig::new(520., 44., 1.);
/// The longest a close may take before the window goes regardless.
const CLOSE_LIMIT: Duration = Duration::from_millis(600);

/// One word on show, with how far it has come in.
struct Word {
    index: usize,
    text: String,
    arrival: Spring,
    clarity: Spring,
    /// 0 for the draft's muted color, 1 for committed text. Carries its velocity across updates.
    ink: Spring,
    delay: Duration,
}

pub struct Caption {
    edge: PillEdge,
    words: Vec<Word>,
    body: Spring,
    width: Spring,
    height: Spring,
    scroll: Spring,
    scroll_handle: ScrollHandle,
    /// Text always wraps at its final width, independently of the animated background.
    content_size: Option<Size<Pixels>>,
    layout_dirty: bool,
    closing: Option<Instant>,
    beat: Option<Instant>,
}

/// Opens the caption beside a pill that sits `margin` from the `edge`, folded until the first
/// words arrive.
pub fn open(
    edge: PillEdge,
    margin: f32,
    display: Option<DisplayId>,
    cx: &mut gpui::App,
) -> anyhow::Result<WindowHandle<Caption>> {
    let width = display
        .and_then(|id| cx.find_display(id))
        .map(|display| (display.visible_bounds().size.width - pill::SHADOW * 2.).min(WIDTH))
        .unwrap_or(WIDTH);
    let size = Size::new(
        width + pill::SHADOW * 2.,
        TEXT_HEIGHT + PADDING_Y * 2. + BORDER * 2. + pill::SHADOW * 2.,
    );
    // The caption starts where the pill ends, and its window starts a shadow's width earlier.
    let offset = (px(margin) - pill::SHADOW).max(Pixels::ZERO) + pill::HEIGHT + GAP;
    let options = overlay::options(edge, offset, size, display, cx);
    cx.open_window(options, move |window, cx| {
        overlay::prepare(window, true);
        cx.new(|_| Caption::new(edge))
    })
}

impl Caption {
    fn new(edge: PillEdge) -> Self {
        Self {
            edge,
            words: Vec::new(),
            body: Spring::new(OPEN),
            width: Spring::new(RESIZE),
            height: Spring::new(RESIZE),
            scroll: Spring::new(SCROLL),
            scroll_handle: ScrollHandle::new(),
            content_size: None,
            layout_dirty: true,
            closing: None,
            beat: None,
        }
    }

    /// Shows `written`, the text the session has inserted, followed by `draft`, the phrase still
    /// being spoken. Words that stay as they were keep still, and each new or changed word comes
    /// in afresh.
    pub fn show(&mut self, written: &str, draft: &str, cx: &mut Context<Self>) {
        self.words = reconcile(std::mem::take(&mut self.words), written, draft);
        self.layout_dirty = true;
        if !self.words.is_empty() && self.closing.is_none() {
            self.body.aim(1., OPEN);
        }
        cx.notify();
    }

    /// Folds the caption away and closes its window once it has.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.closing.is_none() {
            self.closing = Some(Instant::now());
            self.body.aim(0., CLOSE);
            cx.notify();
        }
    }

    fn resize(&mut self, mut content: Size<Pixels>) {
        let first = self.content_size.is_none();
        self.scroll.aim(
            (content.height - TEXT_HEIGHT).max(Pixels::ZERO).as_f32(),
            SCROLL,
        );
        content.height = content.height.min(TEXT_HEIGHT);
        self.width.aim(
            (content.width + PADDING_X * 2. + BORDER * 2.).as_f32(),
            RESIZE,
        );
        self.height.aim(
            (content.height + PADDING_Y * 2. + BORDER * 2.).as_f32(),
            RESIZE,
        );
        // The opening animation already unfolds the first frame. Later changes keep the
        // current dimensions and velocity, even if more text arrives midway through a resize.
        if first {
            self.width.finish();
            self.height.finish();
            self.scroll.finish();
        }
        self.content_size = Some(content);
        self.layout_dirty = false;
    }

    fn trim_history(&mut self, metrics: &[Size<Pixels>], width: Pixels) {
        let excess = self.words.len().saturating_sub(MAX_WORDS);
        let (count, removed_height) = history_prefix(metrics, width, px(self.scroll.at()), excess);
        if count > 0 {
            self.words.drain(..count);
            // The retained words keep the same wrapping and screen coordinates. Rebase both
            // ends of the spring without changing its velocity as offscreen rows are removed.
            self.scroll.state.position -= removed_height.as_f32();
            self.scroll.target -= removed_height.as_f32();
        }
    }

    /// Moves every spring on, or lands them all at once when motion is reduced. Returns whether
    /// anything still moves.
    fn step(&mut self, still: bool) -> bool {
        let now = Instant::now();
        let elapsed = self.beat.replace(now).map_or(Duration::ZERO, |beat| {
            (now - beat).min(Duration::from_millis(50))
        });
        let mut moving = false;
        for spring in [
            &mut self.body,
            &mut self.width,
            &mut self.height,
            &mut self.scroll,
        ] {
            if still {
                spring.finish();
            } else {
                spring.step(elapsed);
            }
            moving |= !spring.settled();
        }
        for word in &mut self.words {
            if still {
                word.delay = Duration::ZERO;
                word.arrival.finish();
                word.clarity.finish();
                word.ink.finish();
            } else {
                let delay = word.delay.min(elapsed);
                word.delay -= delay;
                word.arrival.step(elapsed - delay);
                word.clarity.step(elapsed - delay);
                word.ink.step(elapsed);
            }
            moving |= !word.arrival.settled() || !word.clarity.settled() || !word.ink.settled();
        }
        moving
    }
}

impl Render for Caption {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        // Measure the same separate words the wrapping row draws. A short phrase hugs its
        // content; only longer phrases reach the maximum width and wrap onto another line.
        let word_widths: Vec<Pixels> = self
            .words
            .iter()
            .map(|word| {
                let run = TextRun {
                    len: word.text.len(),
                    font: font("Inter"),
                    color: theme.foreground,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                window
                    .text_system()
                    .shape_line(word.text.clone().into(), TEXT_SIZE, &[run], None)
                    .width
                    // GPUI rounds each text element's intrinsic width up independently.
                    // Summing unrounded advances could wrap a word while there is still
                    // plenty of room to expand the card.
                    .ceil()
            })
            .collect();
        let text_width = word_widths.iter().copied().sum::<Pixels>()
            + WORD_GAP * self.words.len().saturating_sub(1) as f32;
        let width = (text_width + PADDING_X * 2. + BORDER * 2. + px(1.))
            .min(window.viewport_size().width - pill::SHADOW * 2.);
        let content_width = (width - PADDING_X * 2. - BORDER * 2.).max(px(1.));
        if !self.words.is_empty()
            && (self.layout_dirty
                || self
                    .content_size
                    .is_none_or(|size| size.width != content_width))
        {
            if self.words.len() > MAX_WORDS {
                let metrics: Vec<_> = self
                    .words
                    .iter()
                    .zip(&word_widths)
                    .map(|(word, &width)| {
                        let height = if width > content_width {
                            let run = TextRun {
                                len: word.text.len(),
                                font: font("Inter"),
                                color: theme.foreground,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            window
                                .text_system()
                                .shape_text(
                                    word.text.clone().into(),
                                    TEXT_SIZE,
                                    &[run],
                                    Some(content_width),
                                    None,
                                )
                                .map(|lines| {
                                    lines.iter().map(|line| line.size(LINE_HEIGHT).height).sum()
                                })
                                .unwrap_or(LINE_HEIGHT)
                        } else {
                            LINE_HEIGHT
                        };
                        Size::new(width.min(content_width), height)
                    })
                    .collect();
                self.trim_history(&metrics, content_width);
            }
            // Measure the final wrapping layout once per text change, using the same text
            // elements and font as painting. Measuring the animated width would cause early
            // line breaks and a changing height target on every frame.
            let mut flow = text_flow(
                &self.words,
                content_width,
                theme.foreground,
                theme.muted_foreground,
            )
            .into_any_element();
            let measured = flow.layout_as_root(
                Size::new(
                    AvailableSpace::Definite(content_width),
                    AvailableSpace::MaxContent,
                ),
                window,
                cx,
            );
            self.resize(Size::new(content_width, measured.height));
        }

        let moving = self.step(cx.reduce_motion());
        if let Some(closing) = self.closing
            && (!moving || closing.elapsed() >= CLOSE_LIMIT)
        {
            window.remove_window();
            return div().size_full();
        }
        if moving {
            window.request_animation_frame();
        } else {
            self.beat = None;
        }

        let theme = cx.theme();
        let content = self
            .content_size
            .unwrap_or(Size::new(content_width, LINE_HEIGHT));
        let body = self.body.at().clamp(0., 1.2);
        let shown = body.min(1.);
        // The caption grows out of the pill's side, so it moves in from there.
        let towards_pill = match self.edge {
            PillEdge::Bottom => FOLDED_SHIFT,
            PillEdge::Top => -FOLDED_SHIFT,
        };
        let unfolding = body != 1.;
        self.scroll_handle
            .set_offset(point(px(0.), -px(self.scroll.at())));

        let bubble = div()
            .w(px(self.width.at()).max(PADDING_X * 2. + BORDER * 2.))
            .h(px(self.height.at()).max(PADDING_Y * 2. + BORDER * 2.))
            .flex_none()
            .px(PADDING_X)
            .py(PADDING_Y)
            .rounded(RADIUS)
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .overflow_hidden()
            .text_size(TEXT_SIZE)
            .line_height(LINE_HEIGHT)
            .text_color(theme.foreground)
            .when(unfolding, |this| {
                this.opacity((shown * 1.6).min(1.))
                    .layer_scale((FOLDED_SCALE + (1. - FOLDED_SCALE) * body).max(0.01))
                    .layer_scale_origin(point(
                        0.5,
                        match self.edge {
                            PillEdge::Bottom => 1.,
                            PillEdge::Top => 0.,
                        },
                    ))
                    .layer_translate(point(px(0.), towards_pill * (1. - shown)))
                    .when(shown < 1., |this| this.blur(FOLDED_BLUR * (1. - shown)))
            })
            .child(
                // Scroll before painting/clipping, so incoming lines are rendered throughout
                // the motion. A composited translation would only move already visible pixels.
                div()
                    .id("caption-text")
                    .relative()
                    .left(-TEXT_BLEED)
                    .top(-TEXT_BLEED)
                    .w(content.width + TEXT_BLEED * 2.)
                    .h(content.height + TEXT_BLEED * 2.)
                    .p(TEXT_BLEED)
                    .flex_none()
                    .overflow_hidden()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .fade_edges(TEXT_BLEED, TEXT_BLEED)
                    .fade_sides(TEXT_BLEED, TEXT_BLEED)
                    .flex()
                    .flex_col()
                    .child(text_flow(
                        &self.words,
                        content.width,
                        theme.foreground,
                        theme.muted_foreground,
                    )),
            );

        // The caption hugs the pill: above it when the pill sits at the bottom, below it when the
        // pill sits at the top.
        div()
            .size_full()
            .p(pill::SHADOW)
            .flex()
            .flex_col()
            .items_center()
            .font_family("Inter")
            .map(|this| match self.edge {
                PillEdge::Bottom => this.justify_end(),
                PillEdge::Top => this.justify_start(),
            })
            .child(bubble)
    }
}

fn text_flow(words: &[Word], width: Pixels, settled: Hsla, draft: Hsla) -> gpui::Div {
    div()
        .flex_none()
        .w(width)
        .flex()
        .flex_wrap()
        .gap_x(WORD_GAP)
        .font_family("Inter")
        .text_size(TEXT_SIZE)
        .line_height(LINE_HEIGHT)
        .children(words.iter().map(|word| word_element(word, settled, draft)))
}

/// Find complete rows within both the history budget and the area already scrolled offscreen.
fn history_prefix(
    metrics: &[Size<Pixels>],
    width: Pixels,
    scrolled: Pixels,
    excess: usize,
) -> (usize, Pixels) {
    let mut used = Pixels::ZERO;
    let mut row_height = Pixels::ZERO;
    let mut removed_height = Pixels::ZERO;
    let mut prefix = (0, Pixels::ZERO);
    for (index, word) in metrics.iter().enumerate() {
        if used > Pixels::ZERO && used + WORD_GAP + word.width > width {
            removed_height += row_height;
            if index > excess || removed_height > scrolled {
                break;
            }
            prefix = (index, removed_height);
            used = Pixels::ZERO;
            row_height = Pixels::ZERO;
        }
        if used > Pixels::ZERO {
            used += WORD_GAP;
        }
        used += word.width;
        row_height = row_height.max(word.height);
    }
    prefix
}

/// A word rising into place out of a blur. Once it has arrived it is drawn plainly, since any
/// layer effect left on resamples it by a fraction of a pixel.
fn word_element(word: &Word, settled: Hsla, draft: Hsla) -> impl IntoElement {
    let arrived = word.arrival.at();
    let clarity = word.clarity.at().clamp(0., 1.);
    let coming = arrived != 1. || clarity != 1.;
    let rest = 1. - clarity;
    div()
        .flex_none()
        .max_w_full()
        .text_color(blend(draft, settled, word.ink.at().clamp(0., 1.)))
        .when(coming, |this| {
            this.opacity(clarity)
                .layer_translate(point(px(0.), WORD_RISE * (1. - arrived)))
                .when(rest > 0., |this| this.blur(WORD_BLUR * rest))
        })
        .child(word.text.clone())
}

/// Keep springs for words whose text and position survive a draft revision or a final commit.
/// Absolute indices also preserve them when old history is dropped from the front.
fn reconcile(previous: Vec<Word>, written: &str, draft: &str) -> Vec<Word> {
    let next = written
        .split_whitespace()
        .map(|word| (word, false))
        .chain(draft.split_whitespace().map(|word| (word, true)));
    // Keep the previous row boundary. Arbitrarily cutting to the last N words would rewrap
    // every remaining line and make visible text jump during a long session.
    let skip = previous.first().map_or(0, |word| word.index);
    let mut old = previous.into_iter().peekable();
    let mut delay = Duration::ZERO;
    next.enumerate()
        .skip(skip)
        .map(|(index, (text, draft))| {
            while old.peek().is_some_and(|word| word.index < index) {
                old.next();
            }
            if old.peek().is_some_and(|word| word.index == index) {
                let mut word = old.next().expect("index checked above");
                if word.text == text {
                    word.ink.aim(if draft { 0. } else { 1. }, INK);
                    return word;
                }
            }
            let mut arrival = Spring::new(WORD);
            arrival.aim(1., WORD);
            let mut clarity = Spring::new(CLARITY);
            clarity.aim(1., CLARITY);
            let mut ink = Spring::new(INK);
            ink.aim(if draft { 0. } else { 1. }, INK);
            let word = Word {
                index,
                text: text.to_owned(),
                arrival,
                clarity,
                ink,
                delay,
            };
            delay = (delay + WORD_STAGGER).min(MAX_STAGGER);
            word
        })
        .collect()
}

fn blend(from: Hsla, to: Hsla, progress: f32) -> Hsla {
    let from = from.to_rgb();
    let to = to.to_rgb();
    gpui::Rgba {
        r: from.r + (to.r - from.r) * progress,
        g: from.g + (to.g - from.g) * progress,
        b: from.b + (to.b - from.b) * progress,
        a: from.a + (to.a - from.a) * progress,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflowing_lines_scroll_without_jumping_or_restarting_the_motion() {
        let mut caption = Caption::new(PillEdge::Bottom);
        caption.resize(Size::new(px(478.), TEXT_HEIGHT));
        caption.resize(Size::new(px(478.), TEXT_HEIGHT + LINE_HEIGHT));
        assert_eq!(caption.scroll.at(), 0.);
        assert_eq!(caption.scroll.target, LINE_HEIGHT.as_f32());
        caption.beat = Some(Instant::now() - Duration::from_millis(50));
        assert!(caption.step(false));
        assert!(caption.scroll.at() > 0. && caption.scroll.at() < caption.scroll.target);

        let position = caption.scroll.at();
        let velocity = caption.scroll.state.velocity;
        caption.resize(Size::new(px(478.), TEXT_HEIGHT + LINE_HEIGHT * 2.));
        assert_eq!(caption.scroll.at(), position);
        assert_eq!(caption.scroll.state.velocity, velocity);
        for _ in 0..180 {
            let previous = caption.scroll.at();
            caption.beat = Some(Instant::now() - Duration::from_millis(16));
            caption.step(false);
            assert!(
                caption.scroll.at() >= previous && caption.scroll.at() <= caption.scroll.target
            );
        }
        assert_eq!(caption.scroll.at(), LINE_HEIGHT.as_f32() * 2.);
        assert!(!caption.step(false));
        caption.resize(Size::new(px(478.), TEXT_HEIGHT + LINE_HEIGHT * 3.));
        assert!(!caption.step(true));
        assert_eq!(caption.scroll.at(), caption.scroll.target);
        assert_eq!(caption.scroll.state.velocity, 0.);
    }

    #[test]
    fn removing_old_rows_preserves_visible_positions_and_scroll_velocity() {
        let text = (0..137)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut caption = Caption::new(PillEdge::Top);
        caption.words = reconcile(Vec::new(), &text, "");
        // Three words per row at this width: 46 rows, with the last three visible.
        caption.resize(Size::new(px(40.), LINE_HEIGHT * 46.));
        caption.scroll.state.velocity = 10.;
        let visible_y = LINE_HEIGHT * 44. - px(caption.scroll.at());
        let metrics = vec![Size::new(px(10.), LINE_HEIGHT); 137];
        caption.trim_history(&metrics, px(40.));
        assert_eq!(caption.words.len(), MAX_WORDS);
        assert_eq!(caption.words[0].index, 9);
        assert_eq!(LINE_HEIGHT * 41. - px(caption.scroll.at()), visible_y);
        assert_eq!(caption.scroll.state.velocity, 10.);
        assert_eq!(caption.scroll.at(), caption.scroll.target);
    }

    #[test]
    fn history_trimming_never_splits_a_row_or_removes_visible_content() {
        let metrics = vec![Size::new(px(10.), LINE_HEIGHT); 137];
        assert_eq!(
            history_prefix(&metrics, px(40.), px(100.), 7),
            (6, LINE_HEIGHT * 2.)
        );
        assert_eq!(
            history_prefix(&metrics, px(40.), px(36.), 9),
            (3, LINE_HEIGHT)
        );
        // A single long word can span multiple text lines.
        let metrics = [
            Size::new(px(40.), LINE_HEIGHT * 2.),
            Size::new(px(10.), LINE_HEIGHT),
        ];
        assert_eq!(
            history_prefix(&metrics, px(40.), LINE_HEIGHT, 1),
            (0, px(0.))
        );
        assert_eq!(
            history_prefix(&metrics, px(40.), LINE_HEIGHT * 2., 1),
            (1, LINE_HEIGHT * 2.)
        );
    }

    #[test]
    fn resizing_preserves_dimensions_and_velocity_when_new_text_arrives() {
        let mut caption = Caption::new(PillEdge::Bottom);
        caption.resize(Size::new(px(100.), LINE_HEIGHT));
        let width = caption.width.at();
        let height = caption.height.at();
        caption.resize(Size::new(px(470.), TEXT_HEIGHT));
        assert_eq!(caption.width.at(), width);
        assert_eq!(caption.height.at(), height);
        caption.beat = Some(Instant::now() - Duration::from_millis(50));
        assert!(caption.step(false));
        assert!(caption.width.at() > width && caption.width.at() < caption.width.target);
        assert!(caption.height.at() > height && caption.height.at() < caption.height.target);

        let width = caption.width.at();
        let velocity = caption.width.state.velocity;
        caption.resize(Size::new(px(300.), LINE_HEIGHT * 2.));
        assert_eq!(caption.width.at(), width);
        assert_eq!(caption.width.state.velocity, velocity);
        for _ in 0..180 {
            caption.beat = Some(Instant::now() - Duration::from_millis(16));
            caption.step(false);
        }
        assert_eq!(caption.width.at(), caption.width.target);
        assert_eq!(caption.height.at(), caption.height.target);
        assert!(!caption.step(false));
    }

    #[test]
    fn reduced_motion_finishes_a_resize_without_remaining_velocity() {
        let mut caption = Caption::new(PillEdge::Top);
        caption.resize(Size::new(px(100.), LINE_HEIGHT));
        caption.resize(Size::new(px(470.), TEXT_HEIGHT));
        caption.beat = Some(Instant::now() - Duration::from_millis(50));
        caption.step(false);
        assert!(!caption.step(true));
        assert_eq!(caption.width.at(), caption.width.target);
        assert_eq!(caption.height.at(), caption.height.target);
        assert_eq!(caption.width.state.velocity, 0.);
        assert_eq!(caption.height.state.velocity, 0.);
    }

    #[test]
    fn committing_a_draft_changes_only_its_color_target() {
        let mut words = reconcile(Vec::new(), "", "Привет мир");
        for word in &mut words {
            word.arrival.finish();
            word.clarity.finish();
        }
        let mut words = reconcile(words, "Привет мир", "");
        for word in &mut words {
            assert_eq!(word.arrival.at(), 1.);
            assert_eq!(word.clarity.at(), 1.);
            assert_eq!(word.ink.at(), 0.);
            assert_eq!(word.ink.target, 1.);
            word.ink.step(Duration::from_millis(100));
            assert!(word.ink.at() > 0. && word.ink.at() < 1.);
        }
    }

    #[test]
    fn revising_a_word_keeps_the_other_words_and_their_color_in_motion() {
        let mut words = reconcile(Vec::new(), "One", "small step");
        for word in &mut words {
            word.arrival.finish();
            word.clarity.finish();
        }
        words[0].ink.step(Duration::from_millis(100));
        let color = words[0].ink.at();
        let velocity = words[0].ink.state.velocity;
        let words = reconcile(words, "One", "giant step");
        assert_eq!(words[0].ink.at(), color);
        assert_eq!(words[0].ink.state.velocity, velocity);
        assert_eq!(words[1].arrival.at(), 0.);
        assert_eq!(words[1].clarity.at(), 0.);
        assert_eq!(words[2].arrival.at(), 1.);
        assert_eq!(words[2].clarity.at(), 1.);
    }

    #[test]
    fn clarity_and_color_approach_their_endpoints_without_reversing() {
        for config in [CLARITY, INK] {
            for hz in [30, 60, 120, 144] {
                let mut spring = Spring::new(config);
                spring.aim(1., config);
                for _ in 0..hz * 3 {
                    let before = spring.at();
                    spring.step(Duration::from_secs_f32(1. / hz as f32));
                    assert!(spring.at() >= before && spring.at() <= 1.);
                }
                assert_eq!(spring.at(), 1.);
            }
        }
    }

    #[test]
    fn history_is_kept_until_complete_offscreen_rows_can_be_removed() {
        let text = (0..MAX_WORDS)
            .map(|i| format!("word{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut words = reconcile(Vec::new(), &text, "");
        for word in &mut words {
            word.arrival.finish();
        }
        let words = reconcile(words, &text, "new");
        assert_eq!(words.len(), MAX_WORDS + 1);
        assert_eq!(words[0].index, 0);
        assert_eq!(words[0].arrival.at(), 1.);
        assert_eq!(words.last().unwrap().text, "new");
        assert_eq!(words.last().unwrap().arrival.at(), 0.);
    }
}
