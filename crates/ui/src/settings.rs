//! The settings window. Every control writes the config through [`Store`], and the daemon
//! applies the change from there.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Global, ScrollHandle, SharedString, Size, Window,
    WindowHandle, div, px,
};
use tokativ_config::{Backend, Config, Model, PillEdge};
use ui::{
    ActiveTheme as _, Button, Menu, MenuItem, Picker, Popovers, Radio, Scrollbar, Scroller,
    Scrubber, ScrubberState, Switch, Text,
};

use crate::chrome::{self, group, meter, row};
use crate::host::{Host, Library, ModelState, Status, Store};

const WIDTH: f32 = 640.;
const HEIGHT: f32 = 760.;
/// The ranges the sliders cover.
const SILENCE_MS: (f32, f32) = (200., 2000.);
const THRESHOLD: (f32, f32) = (0.2, 0.9);
const MARGIN: (f32, f32) = (0., 240.);
const IDLE_MINUTES: [u32; 5] = [0, 5, 15, 30, 60];
/// The tallest the microphone menu grows before it scrolls, about seven rows.
const DEVICE_MENU_HEIGHT: gpui::Pixels = px(300.);
pub(crate) const NIRI_BIND: &str =
    "Mod+D { spawn \"tokativ\" \"toggle\"; }\nMod+Shift+D { spawn \"tokativ\" \"cancel\"; }";

struct Open(WindowHandle<Settings>);

impl Global for Open {}

/// Opens the settings window, or raises it when it is already open.
pub fn open(cx: &mut App) {
    if let Some(handle) = cx.try_global::<Open>().map(|open| open.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let options = chrome::options("Tokativ settings", Size::new(px(WIDTH), px(HEIGHT)), cx);
    match cx.open_window(options, |window, cx| {
        window.set_rem_size(cx.theme().font_size);
        cx.new(|cx| Settings::new(window, cx))
    }) {
        Ok(handle) => cx.set_global(Open(handle)),
        Err(error) => log::error!("settings: cannot open the window: {error:#}"),
    }
}

/// A slider whose value is shown while it is dragged and saved once it is let go.
struct Slider {
    state: ScrubberState,
    dragging: Option<f32>,
}

impl Slider {
    fn new(id: &str) -> Self {
        Self {
            state: ScrubberState::new(id.to_owned()),
            dragging: None,
        }
    }
}

pub struct Settings {
    store: Entity<Store>,
    library: Entity<Library>,
    status: Entity<Status>,
    popovers: Popovers,
    silence: Slider,
    threshold: Slider,
    margin: Slider,
    devices: Vec<tokativ_audio::InputDevice>,
    scrollbar: Entity<Scrollbar>,
    device_scrollbar: Entity<Scrollbar>,
}

impl Settings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let view = cx.entity_id();
        let host = Host::global(cx);
        cx.observe(&host.store, |_, _, cx| cx.notify()).detach();
        cx.observe(&host.library, |_, _, cx| cx.notify()).detach();
        cx.observe(&host.status, |_, _, cx| cx.notify()).detach();
        cx.observe_window_appearance(window, |_, _, cx| {
            crate::refresh_theme(cx);
            cx.notify();
        })
        .detach();
        Self {
            store: host.store,
            library: host.library,
            status: host.status,
            popovers: Popovers::default(),
            silence: Slider::new("silence"),
            threshold: Slider::new("threshold"),
            margin: Slider::new("margin"),
            devices: tokativ_audio::input_devices(),
            scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(view)),
            device_scrollbar: cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(view)),
        }
    }

    fn edit(&self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Config)) {
        self.store.update(cx, |store, cx| store.update(edit, cx));
    }

    fn dictation(&self, config: &Config, cx: &mut Context<Self>) -> impl IntoElement {
        let device = config.input_device.clone();
        let device_name = device
            .as_ref()
            .and_then(|id| self.devices.iter().find(|known| &known.id == id))
            .map(|known| known.name.clone())
            .unwrap_or_else(|| "System default".into());
        let default_item = MenuItem::new("device-default", "System default")
            .selected(device.is_none())
            .on_click(cx.listener(|this, _, _, cx| {
                this.edit(cx, |config| config.input_device = None);
            }));
        let device_items = self.devices.iter().map(|known| {
            let id = known.id.clone();
            MenuItem::new(
                SharedString::from(format!("device-{}", known.id)),
                known.name.clone(),
            )
            .selected(device.as_deref() == Some(&known.id))
            .on_click(cx.listener(move |this, _, _, cx| {
                let id = id.clone();
                this.edit(cx, |config| config.input_device = Some(id));
            }))
        });
        let device_picker = Picker::new("device", &self.popovers, device_name)
            .menu(device_menu("device-menu", &self.device_scrollbar))
            .item(default_item)
            .items(device_items.collect::<Vec<_>>());

        let live = config.live_preview;
        let live_switch = Switch::new("live-preview", live)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.edit(cx, |config| config.live_preview = !live);
            }))
            .into_any_element();
        group(
            "Dictation",
            vec![
                row(
                    "Live text",
                    "Show a draft of each phrase at the cursor while you speak. Needs the input method.",
                    live_switch,
                    cx,
                )
                .into_any_element(),
                row(
                    "Microphone",
                "Used from the next session on.",
                device_picker.into_any_element(),
                cx,
                )
                .into_any_element(),
            ],
            cx,
        )
    }

    fn detection(&self, config: &Config, cx: &mut Context<Self>) -> impl IntoElement {
        let silence = self
            .silence
            .dragging
            .unwrap_or_else(|| fraction(config.vad.silence_ms as f32, SILENCE_MS));
        let silence_ms = value(silence, SILENCE_MS).round() as u32;
        let threshold = self
            .threshold
            .dragging
            .unwrap_or_else(|| fraction(config.vad.threshold, THRESHOLD));
        let threshold_value = value(threshold, THRESHOLD);

        group(
            "Voice detection",
            vec![
                row(
                    "Pause length",
                    "How long you pause before a phrase is transcribed and inserted.",
                    self.slider(
                        &self.silence,
                        silence,
                        format!("{silence_ms} ms"),
                        |this| &mut this.silence,
                        |config, fraction| {
                            config.vad.silence_ms = value(fraction, SILENCE_MS).round() as u32
                        },
                        cx,
                    ),
                    cx,
                )
                .into_any_element(),
                row(
                    "Speech threshold",
                    "Higher ignores more background noise but may clip quiet words.",
                    self.slider(
                        &self.threshold,
                        threshold,
                        format!("{threshold_value:.2}"),
                        |this| &mut this.threshold,
                        |config, fraction| {
                            config.vad.threshold =
                                (value(fraction, THRESHOLD) * 100.).round() / 100.
                        },
                        cx,
                    ),
                    cx,
                )
                .into_any_element(),
            ],
            cx,
        )
    }

    /// A slider row's control: the scrubber and the value beside it.
    #[allow(clippy::too_many_arguments)]
    fn slider(
        &self,
        slider: &Slider,
        fraction: f32,
        label: String,
        field: fn(&mut Self) -> &mut Slider,
        save: fn(&mut Config, f32),
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.theme();
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div().w(theme.metrics.cover).child(
                    Scrubber::new(&slider.state, fraction)
                        .colors(theme.progress_bar, theme.muted, theme.foreground)
                        .on_move(cx.listener(move |this, fraction: &f32, _, cx| {
                            field(this).dragging = Some(*fraction);
                            cx.notify();
                        }))
                        .on_release(cx.listener(move |this, _, _, cx| {
                            if let Some(fraction) = field(this).dragging.take() {
                                this.edit(cx, |config| save(config, fraction));
                            }
                        })),
                ),
            )
            .child(
                div()
                    .flex_none()
                    .w(theme.metrics.control * 1.8)
                    .whitespace_nowrap()
                    .text_right()
                    .text_size(theme.text(Text::Small))
                    .child(label),
            )
            .into_any_element()
    }

    fn insertion(&self, config: &Config, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.status.read(cx);
        let detail = match &status.note {
            Some(note) => format!("Using {}, because {note}.", status.method.to_lowercase()),
            None => format!("Using {}.", status.method.to_lowercase()),
        };
        let backend = config.insertion.backend;
        let picker = Picker::new("backend", &self.popovers, backend.name())
            .width(Picker::REGULAR)
            .items(Backend::ALL.into_iter().map(|choice| {
                MenuItem::new(
                    SharedString::from(format!("backend-{choice:?}")),
                    choice.name(),
                )
                .selected(choice == backend)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.edit(cx, |config| config.insertion.backend = choice);
                }))
            }));
        group(
            "Insertion",
            vec![row("Method", detail, picker.into_any_element(), cx).into_any_element()],
            cx,
        )
    }

    fn pill(&self, config: &Config, cx: &mut Context<Self>) -> impl IntoElement {
        let edge = config.pill.edge;
        let picker = Picker::new("edge", &self.popovers, edge.name())
            .width(Picker::NARROW)
            .items(PillEdge::ALL.into_iter().map(|choice| {
                MenuItem::new(
                    SharedString::from(format!("edge-{choice:?}")),
                    choice.name(),
                )
                .selected(choice == edge)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.edit(cx, |config| config.pill.edge = choice);
                }))
            }));
        let margin = self
            .margin
            .dragging
            .unwrap_or_else(|| fraction(config.pill.margin, MARGIN));
        let pixels = value(margin, MARGIN).round();
        group(
            "Pill",
            vec![
                row(
                    "Position",
                    "The edge of the focused screen the pill appears on.",
                    picker.into_any_element(),
                    cx,
                )
                .into_any_element(),
                row(
                    "Distance from the edge",
                    "Measured to the pill itself.",
                    self.slider(
                        &self.margin,
                        margin,
                        format!("{pixels} px"),
                        |this| &mut this.margin,
                        |config, fraction| config.pill.margin = value(fraction, MARGIN).round(),
                        cx,
                    ),
                    cx,
                )
                .into_any_element(),
            ],
            cx,
        )
    }

    fn system(&self, config: &Config, cx: &mut Context<Self>) -> impl IntoElement {
        let login = config.launch_at_login;
        let tray = config.tray;
        let idle = config.models.unload_after_minutes;
        let idle_name = |minutes: u32| match minutes {
            0 => "Never".to_string(),
            minutes => format!("After {minutes} min"),
        };
        let idle_picker = Picker::new("idle", &self.popovers, idle_name(idle))
            .width(Picker::NARROW)
            .items(IDLE_MINUTES.into_iter().map(|minutes| {
                MenuItem::new(
                    SharedString::from(format!("idle-{minutes}")),
                    idle_name(minutes),
                )
                .selected(minutes == idle)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.edit(cx, |config| config.models.unload_after_minutes = minutes);
                }))
            }));
        group(
            "System",
            vec![
                row(
                    "Launch at login",
                "Adds an autostart entry. Not needed with the Home Manager service.",
                Switch::new("login", login)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit(cx, |config| config.launch_at_login = !login);
                    }))
                    .into_any_element(),
                cx,
                )
                .into_any_element(),
                row(
                    "Tray icon",
                "Start, stop and switch language from the panel.",
                Switch::new("tray", tray)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit(cx, |config| config.tray = !tray);
                    }))
                    .into_any_element(),
                cx,
                )
                .into_any_element(),
                row(
                    "Unload models",
                "Frees about a gigabyte of memory between sessions, at the cost of a slower start.",
                idle_picker.into_any_element(),
                cx,
                )
                .into_any_element(),
            ],
            cx,
        )
    }

    /// The models to choose from. Clicking a downloaded one makes it the one sessions use.
    fn models(&self, config: &Config, cx: &mut Context<Self>) -> impl IntoElement {
        let library = self.library.read(cx);
        let rows: Vec<_> = Model::ALL
            .into_iter()
            .map(|model| (model, library.state(model)))
            .collect();
        let chosen = config.model;
        group(
            "Model",
            rows.into_iter()
                .map(|(model, state)| {
                    let installed = matches!(state, ModelState::Installed { .. });
                    let radio = Radio::new(
                        SharedString::from(format!("use-{}", model.id())),
                        model == chosen,
                    )
                    .into_any_element();
                    div()
                        .id(SharedString::from(format!("model-{}", model.id())))
                        .when(installed, |this| {
                            this.cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.edit(cx, |config| config.model = model);
                                }))
                        })
                        .child(model_row(model, state, &self.library, Some(radio), cx))
                        .into_any_element()
                })
                .collect(),
            cx,
        )
    }

    /// The window's opening: the app's mark and name.
    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mark = theme.metrics.control * 1.4;
        div()
            .flex()
            .items_center()
            .gap_4()
            .pt_6()
            .pb_2()
            .child(
                div()
                    .flex_none()
                    .size(mark)
                    .rounded(mark / 2.)
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(theme.foreground)
                    .child(
                        gpui::svg()
                            .path(icons::path("icons/mic-vocal.svg"))
                            .size(mark * 0.5)
                            .text_color(theme.background),
                    ),
            )
            .child(
                div()
                    .text_size(theme.text(Text::Large))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("Tokativ"),
            )
    }
}

/// One model's row: its name and maker, what it is good at, where it stands on disk, and the
/// button that changes that. `leading` goes in front, a radio or a checkbox.
pub(crate) fn model_row(
    model: Model,
    state: ModelState,
    library: &Entity<Library>,
    leading: Option<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let download = tokativ_engine::models::download_size(model);
    let (status, progress) = match &state {
        ModelState::Missing => (format!("{} download", chrome::size(download)), None),
        ModelState::Downloading { done, total } => {
            let total = (*total).max(1);
            (
                format!(
                    "Downloading, {} of {}",
                    chrome::size(*done),
                    chrome::size(total)
                ),
                Some(*done as f32 / total as f32),
            )
        }
        ModelState::Installed { bytes } => (format!("{} on disk", chrome::size(*bytes)), None),
        ModelState::Failed(error) => (format!("Download failed: {error}"), None),
    };
    let id = model.id();
    let action = {
        let library = library.clone();
        match state {
            ModelState::Downloading { .. } => {
                Button::new(SharedString::from(format!("cancel-{id}")))
                    .label("Cancel")
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| {
                        library.update(cx, |library, _| library.cancel(model))
                    })
            }
            ModelState::Installed { .. } => Button::new(SharedString::from(format!("delete-{id}")))
                .label("Delete")
                .small()
                .outline()
                .on_click(move |_, _, cx| {
                    library.update(cx, |library, cx| library.delete(model, cx))
                }),
            ModelState::Missing | ModelState::Failed(_) => {
                Button::new(SharedString::from(format!("download-{id}")))
                    .label("Download")
                    .small()
                    .primary()
                    .on_click(move |_, _, cx| {
                        library.update(cx, |library, cx| library.download(model, cx))
                    })
            }
        }
    };
    let small = theme.text(Text::Small);
    div()
        .flex()
        .items_start()
        .gap_3()
        .px(chrome::GROUP_PADDING)
        .py_3()
        .when_some(leading, |this, leading| {
            this.child(div().flex_none().pt(px(2.)).child(leading))
        })
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap_2()
                        .child(model.name())
                        .child(
                            div()
                                .text_size(small)
                                .text_color(theme.muted_foreground)
                                .child(model.maker()),
                        ),
                )
                .child(
                    div()
                        .text_size(small)
                        .text_color(theme.muted_foreground)
                        .child(model.summary()),
                )
                .child(match progress {
                    Some(progress) => div()
                        .pt_1()
                        .child(meter(progress, px(6.), cx))
                        .into_any_element(),
                    None => div()
                        .text_size(small)
                        .text_color(theme.muted_foreground.opacity(0.75))
                        .child(status.clone())
                        .into_any_element(),
                })
                .when(progress.is_some(), |this| {
                    this.child(
                        div()
                            .text_size(small)
                            .text_color(theme.muted_foreground)
                            .child(status),
                    )
                }),
        )
        .child(div().flex_none().child(action))
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.store.read(cx).config().clone();
        let content = div()
            .flex()
            .flex_col()
            .size_full()
            .child(
                Scroller::new("settings", &self.scrollbar).child(
                    div()
                        .w_full()
                        .max_w(px(WIDTH))
                        .mx_auto()
                        .px_6()
                        .pb_6()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(self.header(cx))
                        .child(self.models(&config, cx))
                        .child(self.dictation(&config, cx))
                        .child(self.detection(&config, cx))
                        .child(self.insertion(&config, cx))
                        .child(self.pill(&config, cx))
                        .child(self.system(&config, cx)),
                ),
            )
            .into_any_element();
        chrome::frame("Settings", content, window, cx)
    }
}

/// The menu a microphone picker opens: capped in height and scrolled with Sonora's scrollbar,
/// since a machine can list many inputs.
pub(crate) fn device_menu(id: &'static str, scrollbar: &Entity<Scrollbar>) -> Menu {
    Menu::new(id)
        .w(Picker::WIDE)
        .max_h(DEVICE_MENU_HEIGHT)
        .scrollbar(scrollbar.clone())
}

/// Where `value` sits in `range`, as a fraction from 0 to 1.
fn fraction(value: f32, (low, high): (f32, f32)) -> f32 {
    ((value - low) / (high - low)).clamp(0., 1.)
}

fn value(fraction: f32, (low, high): (f32, f32)) -> f32 {
    low + (high - low) * fraction.clamp(0., 1.)
}
