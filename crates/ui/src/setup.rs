//! The first-run window: download models, pick a microphone, try a dictation, and copy the niri
//! keybind.

use gpui::Focusable as _;
use gpui::prelude::*;
use gpui::{
    Animation, AnimationExt as _, App, ClipboardItem, Context, Entity, Global, ScrollHandle,
    SharedString, Size, Task, Window, WindowHandle, div, px, relative, svg,
};
use std::time::Instant;

use tokativ_audio::{InputDevice, Levels, Recorder};
use tokativ_config::Model;
use ui::{
    ActiveTheme as _, Button, Input, MenuItem, Picker, Popovers, Radio, Scrollbar, Scroller, Text,
};

use crate::chrome::{self, meter};
use crate::desktop::{self, Bound, Desktop};
use crate::host::{Action, Host, Library, ModelState, Status, Store};
use crate::settings::{device_menu, model_row};

/// Names the step the setup window opens on, for working on one step in the sandbox.
pub const STEP_VARIABLE: &str = "TOKATIV_SETUP_STEP";
const WIDTH: f32 = 600.;
const HEIGHT: f32 = 560.;
/// The height of a step segment and how long the newest one takes to fill.
const SEGMENT: gpui::Pixels = px(6.);
const FILL: std::time::Duration = std::time::Duration::from_millis(420);
/// The level meter spans this many dBFS up to full scale, and settles up and down with these
/// time constants in seconds.
const METER_FLOOR: f32 = -60.;
/// How often the hotkey step looks at the desktop config again.
const BINDING_POLL: std::time::Duration = std::time::Duration::from_secs(1);
const METER_ATTACK: f32 = 0.03;
const METER_RELEASE: f32 = 0.3;

struct Open(WindowHandle<Setup>);

impl Global for Open {}

/// Opens the setup window, or raises it when it is already open.
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<Open>().map(|open| open.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let options = chrome::options("Set up Tokativ", Size::new(px(WIDTH), px(HEIGHT)), cx);
    match cx.open_window(options, |window, cx| {
        window.set_rem_size(cx.theme().font_size);
        cx.new(|cx| {
            let mut setup = Setup::new(window, cx);
            let first = std::env::var(STEP_VARIABLE)
                .ok()
                .and_then(|id| Step::ALL.into_iter().find(|step| step.id() == id))
                .unwrap_or(Step::Models);
            setup.go(first, window, cx);
            setup
        })
    }) {
        Ok(handle) => cx.set_global(Open(handle)),
        Err(error) => log::error!("setup: cannot open the window: {error:#}"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Models,
    Microphone,
    Try,
    Hotkey,
}

impl Step {
    const ALL: [Self; 4] = [Self::Models, Self::Microphone, Self::Try, Self::Hotkey];

    /// The name the sandbox's `--step` flag takes.
    fn id(self) -> &'static str {
        match self {
            Self::Models => "models",
            Self::Microphone => "microphone",
            Self::Try => "try",
            Self::Hotkey => "hotkey",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Models => "Models",
            Self::Microphone => "Microphone",
            Self::Try => "Try it",
            Self::Hotkey => "Hotkey",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Models => "icons/library-big.svg",
            Self::Microphone => "icons/mic-vocal.svg",
            Self::Try => "icons/text-select.svg",
            Self::Hotkey => "icons/sliders-horizontal.svg",
        }
    }

    /// One line on what the step asks for.
    fn summary(self) -> &'static str {
        match self {
            Self::Models => "Choose the speech model to dictate with",
            Self::Microphone => "Choose what to listen to and check the level",
            Self::Try => "Dictate a sentence into the field below",
            Self::Hotkey => "Bind a key to start and finish dictating",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|step| *step == self).unwrap_or(0)
    }
}

pub struct Setup {
    store: Entity<Store>,
    library: Entity<Library>,
    status: Entity<Status>,
    step: Step,
    popovers: Popovers,
    devices: Vec<InputDevice>,
    /// Feeds the level meter while the microphone step is shown.
    monitor: Option<Recorder>,
    desktop: Desktop,
    /// The binding found in the desktop's config, looked for again while the hotkey step shows.
    bound: Option<Bound>,
    /// A session started while the hotkey step showed, so the key is known to work.
    pressed: bool,
    _watching: Option<Task<()>>,
    /// What the meter reads, shared with the recorder as it opens.
    levels: Levels,
    /// Counts microphone openings, so one that finishes late can tell it is stale.
    listening: u64,
    meter_shown: f32,
    meter_beat: Option<Instant>,
    field: Entity<Input>,
    scrollbar: Entity<Scrollbar>,
    device_scrollbar: Entity<Scrollbar>,
}

impl Setup {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let host = Host::global(cx);
        cx.observe(&host.store, |_, _, cx| cx.notify()).detach();
        cx.observe(&host.library, |_, _, cx| cx.notify()).detach();
        cx.observe(&host.status, |this, status, cx| {
            if this.step == Step::Hotkey && status.read(cx).recording {
                this.pressed = true;
            }
            cx.notify();
        })
        .detach();
        cx.observe_window_appearance(window, |_, _, cx| {
            crate::refresh_theme(cx);
            cx.notify();
        })
        .detach();
        let field = cx.new(|cx| Input::new("Your words appear here", cx));
        let view = cx.entity_id();
        Self {
            store: host.store,
            library: host.library,
            status: host.status,
            step: Step::Models,
            popovers: Popovers::default(),
            devices: tokativ_audio::input_devices(),
            monitor: None,
            desktop: Desktop::detect(),
            bound: None,
            pressed: false,
            _watching: None,
            levels: Levels::default(),
            listening: 0,
            meter_shown: 0.,
            meter_beat: None,
            field,
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(view)),
            device_scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(view)),
        }
    }

    fn go(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        self.step = step;
        self.monitor = None;
        self._watching = None;
        match step {
            Step::Hotkey => self.watch_binding(cx),
            Step::Microphone => self.listen(cx),
            Step::Try => {
                let focus = self.field.focus_handle(cx);
                focus.focus(window, cx);
            }
            _ => {}
        }
        cx.notify();
    }

    /// Opens the chosen microphone for the level meter.
    /// Opens the chosen microphone for the level meter. Opening takes a moment, so it happens in
    /// the background, and a device that opens after the step was left or another was chosen is
    /// closed again.
    fn listen(&mut self, cx: &mut Context<Self>) {
        self.monitor = None;
        self.listening += 1;
        let generation = self.listening;
        self.levels = Levels::default();
        let levels = self.levels.clone();
        let device = self.store.read(cx).config().input_device.clone();
        let timing = tokativ_audio::Timing {
            threshold: 0.5,
            silence_ms: 500,
            max_segment_s: 20,
            preroll_ms: 200,
        };
        let opening = cx.background_spawn(async move {
            Recorder::start(
                tokativ_audio::Options {
                    device,
                    timing,
                    progress: None,
                },
                None,
                levels,
                |_| {},
            )
        });
        cx.spawn(async move |this, cx| {
            let opened = opening.await;
            this.update(cx, |this, _| match opened {
                Ok(recorder) if this.listening == generation && this.step == Step::Microphone => {
                    this.monitor = Some(recorder);
                }
                Ok(recorder) => recorder.cancel(),
                Err(error) => log::error!("setup: cannot open the microphone: {error:#}"),
            })
            .ok();
        })
        .detach();
    }

    /// Looks for a binding now and every second while the hotkey step shows, so one added in
    /// another window is picked up without leaving the step.
    fn watch_binding(&mut self, cx: &mut Context<Self>) {
        let desktop = self.desktop;
        self._watching = Some(cx.spawn(async move |this, cx| {
            loop {
                let bound = cx.background_spawn(async move { desktop.bound() }).await;
                let alive = this.update(cx, |this, cx| {
                    if this.bound != bound {
                        this.bound = bound;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
                cx.background_executor().timer(BINDING_POLL).await;
            }
        }));
    }

    /// Moves the meter towards the input's loudness: up at once so peaks register, down slowly
    /// so the eye can follow, at a rate set by the real time between frames.
    fn meter(&mut self) -> f32 {
        let now = Instant::now();
        let elapsed = self
            .meter_beat
            .replace(now)
            .map_or(0., |beat| (now - beat).as_secs_f32().min(0.1));
        let target = ((self.levels.loudness() - METER_FLOOR) / -METER_FLOOR).clamp(0., 1.);
        let settle = match target > self.meter_shown {
            true => METER_ATTACK,
            false => METER_RELEASE,
        };
        self.meter_shown += (target - self.meter_shown) * (1. - (-elapsed / settle).exp());
        self.meter_shown
    }

    fn models(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let library = self.library.read(cx);
        let states: Vec<(Model, ModelState)> = Model::ALL
            .into_iter()
            .map(|model| (model, library.state(model)))
            .collect();
        let chosen = self.store.read(cx).config().model;

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(intro(
                "Pick the model to dictate with. It runs on this computer and nothing you say leaves it. You can download another one later in settings.",
                cx,
            ))
            .children(states.into_iter().map(|(model, state)| {
                let radio = Radio::new(
                    SharedString::from(format!("pick-{}", model.id())),
                    model == chosen,
                )
                .into_any_element();
                div()
                    .id(SharedString::from(format!("setup-model-{}", model.id())))
                    .cursor_pointer()
                    .rounded(chrome::GROUP_RADIUS)
                    .border_1()
                    .border_color(match model == chosen {
                        true => cx.theme().foreground.opacity(0.5),
                        false => cx.theme().border.opacity(0.6),
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.store
                            .update(cx, |store, cx| store.update(|config| config.model = model, cx));
                    }))
                    .child(model_row(model, state, &self.library, Some(radio), cx))
            }))
    }

    fn microphone(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let device = self.store.read(cx).config().input_device.clone();
        let name = device
            .as_ref()
            .and_then(|id| self.devices.iter().find(|known| &known.id == id))
            .map(|known| known.name.clone())
            .unwrap_or_else(|| "System default".into());
        let mut items = vec![
            MenuItem::new("device-default", "System default")
                .selected(device.is_none())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.store.update(cx, |store, cx| {
                        store.update(|config| config.input_device = None, cx)
                    });
                    this.listen(cx);
                })),
        ];
        items.extend(self.devices.iter().map(|known| {
            let id = known.id.clone();
            MenuItem::new(
                SharedString::from(format!("device-{}", known.id)),
                known.name.clone(),
            )
            .selected(device.as_deref() == Some(&known.id))
            .on_click(cx.listener(move |this, _, _, cx| {
                let id = id.clone();
                this.store.update(cx, |store, cx| {
                    store.update(|config| config.input_device = Some(id), cx)
                });
                this.listen(cx);
            }))
        }));
        let level = self.meter();
        window.request_animation_frame();

        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(intro(
                "Pick the microphone to dictate with and say something to check the level.",
                cx,
            ))
            .child(
                Picker::new("setup-device", &self.popovers, name)
                    .menu(device_menu("setup-device-menu", &self.device_scrollbar))
                    .items(items),
            )
            .child(meter(level, px(12.), cx))
    }

    fn try_it(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let recording = self.status.read(cx).recording;
        let field = self.field.clone();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(intro(
                "Press Start, speak a sentence or two, then press the check mark on the pill. Each phrase is typed into the field as you pause.",
                cx,
            ))
            .child(self.field.clone())
            .child(
                div().flex().gap_2().child(
                    Button::new("try")
                        .label(match recording {
                            true => "Finish",
                            false => "Start dictation",
                        })
                        .primary()
                        .on_click(move |_, window, cx| {
                            let focus = field.focus_handle(cx);
                            focus.focus(window, cx);
                            Host::global(cx).dispatch(Action::Toggle, cx);
                        }),
                ),
            )
            .when(!window.is_window_active(), |this| {
                this.child(intro("Keep this window focused so the text lands in the field.", cx))
            })
    }

    /// Directions for binding the toggle, written for the desktop this session runs in. The step
    /// watches the config for a binding and confirms once the key actually starts a session.
    fn hotkey(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let desktop = self.desktop;
        let snippet = desktop.snippet();
        let (icon, status, ready) = match (&self.bound, self.pressed) {
            (_, true) => (
                "icons/circle-check.svg",
                "The key works. Press it once more to close the pill.".to_string(),
                true,
            ),
            (Some(bound), false) => (
                "icons/info.svg",
                format!(
                    "Found {} in {}. Press it now to check it.",
                    bound.keys,
                    bound
                        .file
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ),
                false,
            ),
            (None, false) => (
                "icons/info.svg",
                "No key runs Tokativ yet. Add one, then press it here to check it.".to_string(),
                false,
            ),
        };
        let status = div()
            .flex()
            .items_center()
            .gap_2()
            .px(chrome::GROUP_PADDING)
            .py_3()
            .rounded(chrome::GROUP_RADIUS)
            .border_1()
            .border_color(match ready {
                true => theme.foreground.opacity(0.5),
                false => theme.border.opacity(0.6),
            })
            .child(
                svg()
                    .path(icons::path(icon))
                    .size(px(16.))
                    .flex_none()
                    .text_color(match ready {
                        true => theme.foreground,
                        false => theme.muted_foreground,
                    }),
            )
            .child(div().text_size(theme.text(Text::Small)).child(status));

        let config = desktop.config_file();
        let actions = div()
            .flex()
            .gap_2()
            .child(
                Button::new("copy-bind")
                    .label("Copy")
                    .icon("icons/copy.svg")
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(snippet.to_owned()))
                    }),
            )
            .when_some(config, |this, config| {
                this.child(
                    Button::new("open-config")
                        .label("Open config")
                        .icon("icons/external-link.svg")
                        .small()
                        .outline()
                        .on_click(move |_, _, _| desktop::open(&config)),
                )
            });

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(status)
            .child(
                div()
                    .pt_2()
                    .child(ui::eyebrow(format!("{} detected", desktop.name()), cx)),
            )
            .child(intro(desktop.directions(), cx))
            .when(desktop.supported(), |this| {
                this.child(chrome::code(snippet, cx)).child(actions)
            })
    }

    fn can_advance(&self, cx: &App) -> bool {
        match self.step {
            Step::Models => self
                .library
                .read(cx)
                .installed(self.store.read(cx).config().model),
            _ => true,
        }
    }

    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.monitor = None;
        let config = self.store.read(cx).config().clone();
        if !tokativ_config::config_path().exists()
            && let Err(error) = config.save()
        {
            log::error!("setup: cannot save the config: {error:#}");
        }
        window.remove_window();
    }
}

fn intro(text: &'static str, cx: &App) -> impl IntoElement {
    div()
        .text_color(cx.theme().muted_foreground)
        .text_size(cx.theme().text(Text::Small))
        .child(text)
}

/// The step header: the step's icon, name and number, and a bar of one segment per step. The
/// segment of the step just reached fills in as it appears.
fn progress(step: Step, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let index = step.index();
    let count = Step::ALL.len();
    let segments = Step::ALL.into_iter().map(move |each| {
        let reached = each.index() <= index;
        let fill = move || div().h_full().rounded_full().bg(theme.progress_bar);
        div()
            .flex_1()
            .h(SEGMENT)
            .rounded_full()
            .bg(theme.muted)
            .overflow_hidden()
            .when(reached && each != step, |this| this.child(fill().w_full()))
            .when(each == step, |this| {
                this.child(fill().with_animation(
                    SharedString::from(format!("segment-{}", each.index())),
                    Animation::new(FILL).with_easing(ui::ease_out_cubic),
                    |fill, delta| fill.w(relative(delta)),
                ))
            })
    });
    div()
        .flex()
        .flex_col()
        .gap_3()
        .p(chrome::GROUP_PADDING)
        .rounded(chrome::GROUP_RADIUS)
        .bg(theme.secondary.opacity(0.45))
        .border_1()
        .border_color(theme.border.opacity(0.6))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    svg()
                        .path(icons::path(step.icon()))
                        .size(px(16.))
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .flex_1()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(step.title()),
                )
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(format!("{}/{count}", index + 1)),
                ),
        )
        .child(
            div()
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .child(step.summary()),
        )
        .child(div().flex().gap_1().children(segments))
}

impl Render for Setup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let step = self.step;
        let index = step.index();
        let last = index == Step::ALL.len() - 1;

        let body = match step {
            Step::Models => self.models(cx).into_any_element(),
            Step::Microphone => self.microphone(window, cx).into_any_element(),
            Step::Try => self.try_it(window, cx).into_any_element(),
            Step::Hotkey => self.hotkey(cx).into_any_element(),
        };
        let advance = self.can_advance(cx);

        let footer = div()
            .flex()
            .justify_between()
            .items_center()
            .child(
                Button::new("back")
                    .label("Back")
                    .outline()
                    .disabled(index == 0)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if index > 0 {
                            this.go(Step::ALL[index - 1], window, cx);
                        }
                    })),
            )
            .child(
                Button::new("next")
                    .label(match last {
                        true => "Finish",
                        false => "Next",
                    })
                    .primary()
                    .disabled(!advance)
                    .on_click(cx.listener(move |this, _, window, cx| match last {
                        true => this.finish(window, cx),
                        false => this.go(Step::ALL[index + 1], window, cx),
                    })),
            );

        let content = div()
            .id("setup")
            .size_full()
            .flex()
            .flex_col()
            .px_6()
            .pb_6()
            .gap_4()
            .pt_4()
            .child(progress(step, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .child(Scroller::new("setup-body", &self.scrollbar).child(body)),
            )
            .child(footer)
            .into_any_element();
        chrome::frame("Set up Tokativ", content, window, cx)
    }
}
