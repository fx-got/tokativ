//! The first-run window: download models, pick a microphone, try a dictation, and copy the niri
//! keybind.

use gpui::Focusable as _;
use gpui::prelude::*;
use gpui::{
    App, ClipboardItem, Context, Entity, Global, SharedString, Size, Window, WindowHandle, div, px,
};
use tokativ_audio::{InputDevice, Recorder};
use tokativ_config::EngineKind;
use ui::{ActiveTheme as _, Button, Checkbox, Input, MenuItem, Picker, Popovers, Text};

use crate::chrome::{self, meter, section};
use crate::host::{Action, Host, Library, ModelState, Status, Store};
use crate::settings::{NIRI_BIND, model_row};

const WIDTH: f32 = 600.;
const HEIGHT: f32 = 560.;

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
        cx.new(|cx| Setup::new(window, cx))
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

    fn title(self) -> &'static str {
        match self {
            Self::Models => "Models",
            Self::Microphone => "Microphone",
            Self::Try => "Try it",
            Self::Hotkey => "Hotkey",
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
    wanted: [bool; 2],
    popovers: Popovers,
    devices: Vec<InputDevice>,
    /// Feeds the level meter while the microphone step is shown.
    monitor: Option<Recorder>,
    field: Entity<Input>,
}

impl Setup {
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
        let field = cx.new(|cx| Input::new("Your words appear here", cx));
        Self {
            store: host.store,
            library: host.library,
            status: host.status,
            step: Step::Models,
            wanted: [true, false],
            popovers: Popovers::default(),
            devices: tokativ_audio::input_devices(),
            monitor: None,
            field,
        }
    }

    fn go(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        self.step = step;
        self.monitor = None;
        match step {
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
    fn listen(&mut self, cx: &mut Context<Self>) {
        self.monitor = None;
        let device = self.store.read(cx).config().input_device.clone();
        let timing = tokativ_audio::Timing {
            threshold: 0.5,
            silence_ms: 500,
            max_segment_s: 20,
            preroll_ms: 200,
        };
        match Recorder::start(tokativ_audio::Options { device, timing }, None, |_| {}) {
            Ok(recorder) => self.monitor = Some(recorder),
            Err(error) => log::error!("setup: cannot open the microphone: {error:#}"),
        }
    }

    fn models(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let library = self.library.read(cx);
        let states: Vec<(EngineKind, ModelState)> = EngineKind::ALL
            .into_iter()
            .map(|kind| (kind, library.state(kind)))
            .collect();
        let blurbs = [
            "English, Russian and 23 more European languages, detected on the fly.",
            "Russian only, with punctuation. Pick it if you dictate mostly in Russian.",
        ];
        let downloadable = states.iter().enumerate().any(|(index, (_, state))| {
            self.wanted[index] && matches!(state, ModelState::Missing | ModelState::Failed(_))
        });

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(intro(
                "Pick the speech models to download. They run on this computer and nothing you say leaves it.",
                cx,
            ))
            .children(states.into_iter().enumerate().map(|(index, (kind, state))| {
                let wanted = self.wanted[index];
                div()
                    .flex()
                    .gap_3()
                    .items_start()
                    .child(
                        div().pt_4().child(
                            Checkbox::new(SharedString::from(format!("want-{}", kind.id())), wanted)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.wanted[index] = !this.wanted[index];
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(model_row(kind, state, &self.library, cx))
                            .child(
                                div()
                                    .pb_2()
                                    .text_size(cx.theme().text(Text::Small))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(blurbs[index]),
                            ),
                    )
            }))
            .child(
                div().pt_2().child(
                    Button::new("download-selected")
                        .label("Download selected")
                        .primary()
                        .disabled(!downloadable)
                        .on_click(cx.listener(|this, _, _, cx| {
                            for (index, kind) in EngineKind::ALL.into_iter().enumerate() {
                                if this.wanted[index] {
                                    this.library.update(cx, |library, cx| {
                                        if !library.installed(kind) {
                                            library.download(kind, cx);
                                        }
                                    });
                                }
                            }
                        })),
                ),
            )
    }

    fn microphone(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        let level = self
            .monitor
            .as_ref()
            .map(|monitor| monitor.levels().peak())
            .unwrap_or(0.);
        if self.monitor.is_some() {
            window.request_animation_frame();
        }

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
                    .width(px(360.))
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

    fn hotkey(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(intro(
                "Bind the toggle in your niri config. The first press opens the pill, the second inserts the rest and closes it.",
                cx,
            ))
            .child(chrome::code(NIRI_BIND, cx))
            .child(
                div().child(
                    Button::new("copy-bind")
                        .label("Copy")
                        .icon("icons/copy.svg")
                        .small()
                        .outline()
                        .on_click(|_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(NIRI_BIND.to_owned()))
                        }),
                ),
            )
    }

    fn can_advance(&self, cx: &App) -> bool {
        match self.step {
            Step::Models => self.library.read(cx).any_installed(),
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

impl Render for Setup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let step = self.step;
        let index = step.index();
        let last = index == Step::ALL.len() - 1;
        let steps = div()
            .flex()
            .gap_4()
            .children(Step::ALL.into_iter().map(|each| {
                let current = each == step;
                div()
                    .text_size(theme.text(Text::Small))
                    .font_weight(match current {
                        true => gpui::FontWeight::SEMIBOLD,
                        false => gpui::FontWeight::NORMAL,
                    })
                    .text_color(match current {
                        true => theme.foreground,
                        false => theme.muted_foreground,
                    })
                    .child(format!("{}. {}", each.index() + 1, each.title()))
            }));

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
            .child(section(step.title(), cx))
            .child(steps)
            .child(
                div()
                    .id("setup-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(body),
            )
            .child(footer)
            .into_any_element();
        chrome::frame("Set up Tokativ", content, window, cx)
    }
}
