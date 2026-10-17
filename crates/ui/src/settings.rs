//! The settings window. Every control writes the config through [`Store`], and the daemon
//! applies the change from there.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, Global, ScrollHandle, SharedString, Size, Task, Window,
    WindowHandle, div, px,
};
use tokativ_config::{Backend, Config, PillEdge, Processor};
use tokativ_engine::ModelId;
use ui::{
    ActiveTheme as _, Input, Menu, MenuItem, Picker, Popovers, Radio, Scrollbar, Scroller,
    Scrubber, ScrubberState, Switch, Text,
};

use crate::chrome::{self, group, row};
use crate::host::{Host, Library, Status, Store};
use crate::models;
use crate::usage::{Sampler, Usage};

const WIDTH: f32 = 640.;
const HEIGHT: f32 = 760.;
/// The ranges the sliders cover.
const SILENCE_MS: (f32, f32) = (200., 2000.);
const THRESHOLD: (f32, f32) = (0.2, 0.9);
const MARGIN: (f32, f32) = (0., 240.);
const IDLE_MINUTES: [u32; 5] = [0, 5, 15, 30, 60];
/// How often the resources group reads the daemon's footprint.
const SAMPLE_EVERY: std::time::Duration = std::time::Duration::from_secs(1);
/// The tallest the microphone menu grows before it scrolls, about seven rows.
const DEVICE_MENU_HEIGHT: gpui::Pixels = px(300.);
/// How long typing must pause before the search goes out.
const SEARCH_PAUSE: std::time::Duration = std::time::Duration::from_millis(350);

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
    usage: Usage,
    _sampling: Task<()>,
    search_input: Entity<Input>,
    /// The query last sent, so a notify that did not change the text does not search again.
    searched: String,
    /// The search result whose files are listed.
    opened: Option<String>,
    _debounce: Option<Task<()>>,
}

impl Settings {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let view = cx.entity_id();
        let host = Host::global(cx);
        cx.observe(&host.store, |_, _, cx| cx.notify()).detach();
        cx.observe(&host.library, |_, _, cx| cx.notify()).detach();
        cx.observe(&host.status, |_, _, cx| cx.notify()).detach();
        let search_input =
            cx.new(|cx| Input::new("Search Hugging Face", cx).icon("icons/search.svg"));
        cx.observe(&search_input, |this: &mut Self, _, cx| {
            this.query_changed(cx)
        })
        .detach();
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
            usage: Usage::default(),
            search_input,
            searched: String::new(),
            opened: None,
            _debounce: None,
            _sampling: cx.spawn(async move |this, cx| {
                let mut sampler = Sampler::default();
                loop {
                    let usage = sampler.sample();
                    let open = this.update(cx, |this, cx| {
                        this.usage = usage;
                        cx.notify();
                    });
                    if open.is_err() {
                        return;
                    }
                    cx.background_executor().timer(SAMPLE_EVERY).await;
                }
            }),
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

    /// The daemon's own memory and processor use, refreshed every second.
    fn resources(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let value = |text: String| {
            div()
                .text_size(theme.text(Text::Small))
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        let memory = chrome::size(self.usage.memory);
        let cpu = match self.usage.cpu {
            Some(cpu) => format!("{cpu:.0}% of a core"),
            None => "Measuring".to_string(),
        };
        group(
            "Resources",
            vec![
                row(
                    "Memory",
                    "Mostly the loaded models. Unloading them when idle frees it.",
                    value(memory),
                    cx,
                )
                .into_any_element(),
                row(
                    "Processor",
                    "Over the last second. Close to nothing between sessions.",
                    value(cpu),
                    cx,
                )
                .into_any_element(),
            ],
            cx,
        )
    }

    /// The models to choose from, with the device they run on. Clicking an installed model makes
    /// it the one sessions use.
    fn models(&self, config: &Config, cx: &mut Context<Self>) -> impl IntoElement {
        let chosen = ModelId::parse(&config.model);
        let listed = models::listed(self.library.read(cx), chosen.as_slice());
        let mut rows: Vec<AnyElement> = listed
            .into_iter()
            .map(|model| {
                let id = model.id.clone();
                let radio = Radio::new(
                    SharedString::from(format!("use-{}", id.key())),
                    Some(&id) == chosen.as_ref(),
                )
                .into_any_element();
                let key = id.key();
                let choose = cx.listener(move |this, _, _, cx| {
                    let key = key.clone();
                    this.edit(cx, |config| config.model = key);
                });
                models::model_row(
                    model,
                    &self.library,
                    Some(radio),
                    Some(Box::new(choose)),
                    cx,
                )
            })
            .collect();

        let processor = config.processor;
        let device = self.status.read(cx).device.clone();
        let detail = match device {
            Some(device) => format!("Running on {device}."),
            None => "The graphics card is used through Vulkan when there is one.".to_string(),
        };
        let picker = Picker::new("processor", &self.popovers, processor.name())
            .width(Picker::REGULAR)
            .items(Processor::ALL.into_iter().map(|choice| {
                MenuItem::new(
                    SharedString::from(format!("processor-{choice:?}")),
                    choice.name(),
                )
                .selected(choice == processor)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.edit(cx, |config| config.processor = choice);
                }))
            }));
        rows.push(row("Run on", detail, picker.into_any_element(), cx).into_any_element());
        group("Model", rows, cx)
    }

    /// Search over Hugging Face for any model the engine can run.
    fn find(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let library = self.library.read(cx);
        let search = &library.search;
        let mut rows = vec![
            div()
                .px(chrome::GROUP_PADDING)
                .py_3()
                .child(self.search_input.clone())
                .into_any_element(),
        ];
        if search.query.trim().is_empty() {
            rows.push(models::note(
                "Search Hugging Face for speech models in GGUF form. Whisper, Parakeet, Canary, GigaAM, Moonshine, Qwen3-ASR and Voxtral all run here.",
                cx,
            ));
        } else if search.loading && search.results.is_empty() {
            rows.push(models::note("Searching", cx));
        } else if let Some(error) = &search.error {
            rows.push(models::note(&format!("Search failed: {error}"), cx));
        } else if search.results.is_empty() {
            rows.push(models::note("No models that run here match that.", cx));
        }
        let results = search.results.clone();
        rows.extend(results.iter().map(|repository| {
            let open = self.opened.as_deref() == Some(repository.id.as_str());
            let repo = repository.id.clone();
            let toggle = cx.listener(move |this, _, _, cx| {
                let repo = repo.clone();
                if this.opened.as_deref() == Some(repo.as_str()) {
                    this.opened = None;
                } else {
                    this.library
                        .update(cx, |library, cx| library.open_repository(repo.clone(), cx));
                    this.opened = Some(repo);
                }
                cx.notify();
            });
            models::repository_row(repository, open, &self.library, Box::new(toggle), cx)
        }));
        group("Find models", rows, cx)
    }

    /// Searches once typing pauses, so each keystroke does not reach the network.
    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).text().to_owned();
        if query == self.searched {
            return;
        }
        self.searched = query.clone();
        self._debounce = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_PAUSE).await;
            this.update(cx, |this, cx| {
                this.opened = None;
                this.library
                    .update(cx, |library, cx| library.search(query, cx));
            })
            .ok();
        }));
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
                        .child(self.find(cx))
                        .child(self.dictation(&config, cx))
                        .child(self.detection(&config, cx))
                        .child(self.insertion(&config, cx))
                        .child(self.pill(&config, cx))
                        .child(self.system(&config, cx))
                        .child(self.resources(cx)),
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
