//! The settings window. Every control writes the config through [`Store`], and the daemon
//! applies the change from there.

use gpui::prelude::*;
use gpui::{
    App, ClipboardItem, Context, Entity, Global, SharedString, Size, Window, WindowHandle, div, px,
};
use tokativ_config::{Backend, Config, EngineKind, LanguageMode, PillEdge};
use ui::{
    ActiveTheme as _, Button, MenuItem, Picker, Popovers, Scrubber, ScrubberState, Switch, Text,
};

use crate::chrome::{self, meter, row, section};
use crate::host::{Host, Library, ModelState, Status, Store};

const WIDTH: f32 = 640.;
const HEIGHT: f32 = 760.;
/// The ranges the sliders cover.
const SILENCE_MS: (f32, f32) = (200., 2000.);
const THRESHOLD: (f32, f32) = (0.2, 0.9);
const MARGIN: (f32, f32) = (0., 240.);
const IDLE_MINUTES: [u32; 5] = [0, 5, 15, 30, 60];
pub(crate) const NIRI_BIND: &str = "binds {\n    Mod+D { spawn \"tokativ\" \"toggle\"; }\n    Mod+Shift+D { spawn \"tokativ\" \"cancel\"; }\n}";

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
}

impl Settings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        }
    }

    fn edit(&self, cx: &mut Context<Self>, edit: impl FnOnce(&mut Config)) {
        self.store.update(cx, |store, cx| store.update(edit, cx));
    }

    fn dictation(&self, config: &Config, cx: &mut Context<Self>) -> impl IntoElement {
        let language = config.language;
        let language_picker = Picker::new("language", &self.popovers, language.name())
            .width(Picker::NARROW)
            .items(LanguageMode::ALL.into_iter().map(|mode| {
                MenuItem::new(SharedString::from(mode.id()), mode.name())
                    .selected(mode == language)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit(cx, |config| config.language = mode);
                    }))
            }));

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
            .width(Picker::WIDE)
            .item(default_item)
            .items(device_items.collect::<Vec<_>>());

        div()
            .child(section("Dictation", cx))
            .child(row(
                "Language mode",
                "The mode a session starts in. The hotkey and tray can switch it.",
                language_picker.into_any_element(),
                cx,
            ))
            .child(row(
                "Engine for Auto",
                "Parakeet detects English, Russian and 23 more languages.",
                self.engine_picker(
                    "engine-auto",
                    config.engines.auto,
                    |config, kind| config.engines.auto = kind,
                    cx,
                )
                .into_any_element(),
                cx,
            ))
            .child(row(
                "Engine for Russian",
                "GigaAM is trained on Russian only and punctuates it well.",
                self.engine_picker(
                    "engine-russian",
                    config.engines.russian,
                    |config, kind| config.engines.russian = kind,
                    cx,
                )
                .into_any_element(),
                cx,
            ))
            .child(row(
                "Microphone",
                "Used from the next session on.",
                device_picker.into_any_element(),
                cx,
            ))
    }

    fn engine_picker(
        &self,
        key: &'static str,
        current: EngineKind,
        set: fn(&mut Config, EngineKind),
        cx: &mut Context<Self>,
    ) -> Picker {
        Picker::new(key, &self.popovers, current.name())
            .width(Picker::REGULAR)
            .items(EngineKind::ALL.into_iter().map(move |kind| {
                MenuItem::new(
                    SharedString::from(format!("{key}-{}", kind.id())),
                    kind.name(),
                )
                .selected(kind == current)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.edit(cx, |config| set(config, kind));
                }))
            }))
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

        div()
            .child(section("Voice detection", cx))
            .child(row(
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
            ))
            .child(row(
                "Speech threshold",
                "Higher ignores more background noise but may clip quiet words.",
                self.slider(
                    &self.threshold,
                    threshold,
                    format!("{threshold_value:.2}"),
                    |this| &mut this.threshold,
                    |config, fraction| {
                        config.vad.threshold = (value(fraction, THRESHOLD) * 100.).round() / 100.
                    },
                    cx,
                ),
                cx,
            ))
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
        div().child(section("Insertion", cx)).child(row(
            "Method",
            detail,
            picker.into_any_element(),
            cx,
        ))
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
        div()
            .child(section("Pill", cx))
            .child(row(
                "Position",
                "The edge of the focused screen the pill appears on.",
                picker.into_any_element(),
                cx,
            ))
            .child(row(
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
            ))
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
        div()
            .child(section("System", cx))
            .child(row(
                "Launch at login",
                "Adds an autostart entry. Not needed with the Home Manager service.",
                Switch::new("login", login)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit(cx, |config| config.launch_at_login = !login);
                    }))
                    .into_any_element(),
                cx,
            ))
            .child(row(
                "Tray icon",
                "Start, stop and switch language from the panel.",
                Switch::new("tray", tray)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit(cx, |config| config.tray = !tray);
                    }))
                    .into_any_element(),
                cx,
            ))
            .child(row(
                "Unload models",
                "Frees about a gigabyte of memory between sessions, at the cost of a slower start.",
                idle_picker.into_any_element(),
                cx,
            ))
    }

    fn models(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let library = self.library.read(cx);
        let rows: Vec<_> = EngineKind::ALL
            .into_iter()
            .map(|kind| (kind, library.state(kind)))
            .collect();
        div().child(section("Models", cx)).children(
            rows.into_iter()
                .map(|(kind, state)| model_row(kind, state, &self.library, cx)),
        )
    }

    fn hotkey(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .child(section("Hotkey", cx))
            .child(
                div()
                    .py_3()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .text_size(cx.theme().text(Text::Small))
                            .child("Add this to your niri config, then press the key to start and again to insert."),
                    )
                    .child(chrome::code(NIRI_BIND, cx))
                    .child(
                        div().child(
                            Button::new("copy-bind")
                                .label("Copy")
                                .icon("icons/copy.svg")
                                .small()
                                .outline()
                                .on_click(|_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        NIRI_BIND.to_owned(),
                                    ))
                                }),
                        ),
                    ),
            )
    }
}

/// One model's row: its name, where it stands, and the button that changes that.
pub(crate) fn model_row(
    kind: EngineKind,
    state: ModelState,
    library: &Entity<Library>,
    cx: &App,
) -> impl IntoElement {
    let download = tokativ_engine::models::download_size(kind);
    let (detail, progress) = match &state {
        ModelState::Missing => (format!("Not downloaded, {}", chrome::size(download)), None),
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
        ModelState::Installed { bytes } => {
            (format!("Installed, {} on disk", chrome::size(*bytes)), None)
        }
        ModelState::Failed(error) => (format!("Download failed: {error}"), None),
    };
    let id = kind.id();
    let action = {
        let library = library.clone();
        match state {
            ModelState::Downloading { .. } => {
                Button::new(SharedString::from(format!("cancel-{id}")))
                    .label("Cancel")
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| library.update(cx, |library, _| library.cancel(kind)))
            }
            ModelState::Installed { .. } => Button::new(SharedString::from(format!("delete-{id}")))
                .label("Delete")
                .small()
                .outline()
                .on_click(move |_, _, cx| {
                    library.update(cx, |library, cx| library.delete(kind, cx))
                }),
            ModelState::Missing | ModelState::Failed(_) => {
                Button::new(SharedString::from(format!("download-{id}")))
                    .label("Download")
                    .small()
                    .primary()
                    .on_click(move |_, _, cx| {
                        library.update(cx, |library, cx| library.download(kind, cx))
                    })
            }
        }
    };
    div()
        .child(row(kind.name(), detail, action.into_any_element(), cx))
        .when_some(progress, |this, progress| {
            this.child(div().pb_3().child(meter(progress, px(8.), cx)))
        })
}

impl Render for Settings {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let config = self.store.read(cx).config().clone();
        let content = div()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .max_w(px(WIDTH))
                    .mx_auto()
                    .px_6()
                    .pb_6()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(self.dictation(&config, cx))
                    .child(self.detection(&config, cx))
                    .child(self.insertion(&config, cx))
                    .child(self.pill(&config, cx))
                    .child(self.system(&config, cx))
                    .child(self.models(cx))
                    .child(self.hotkey(cx)),
            )
            .into_any_element();
        chrome::frame("Settings", content, window, cx)
    }
}

/// Where `value` sits in `range`, as a fraction from 0 to 1.
fn fraction(value: f32, (low, high): (f32, f32)) -> f32 {
    ((value - low) / (high - low)).clamp(0., 1.)
}

fn value(fraction: f32, (low, high): (f32, f32)) -> f32 {
    low + (high - low) * fraction.clamp(0., 1.)
}
