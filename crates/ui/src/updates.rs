//! The update popup and its shared background worker. Network and disk work stay off the UI thread.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::channel::oneshot;
use gpui::prelude::*;
use gpui::{App, Context, Entity, Global, SharedString, Size, Window, WindowHandle, div, px};
use tokativ_updater::{Installation, Platform, Prepared, Release};
use ui::{ActiveTheme as _, Button, Text};

use crate::chrome;
use crate::host::{Action, Host};

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone)]
enum Phase {
    Idle,
    Checking,
    Latest,
    Available,
    Downloading { done: u64, total: u64 },
    Ready,
    Installing,
    Failed(String),
}

struct Updater {
    phase: Phase,
    release: Option<Release>,
    prepared: Option<Prepared>,
    installation: Installation,
    notified: Option<String>,
    recovery: bool,
}

struct Service(Entity<Updater>);
impl Global for Service {}
struct Open(WindowHandle<UpdateWindow>);
impl Global for Open {}

pub fn init(cx: &mut App) {
    let error = tokativ_updater::take_error();
    let recovery = error.is_some();
    let host = Host::global(cx);
    let updater = cx.new(|cx| {
        cx.observe(&host.status, |this: &mut Updater, _, cx| {
            this.maybe_popup(cx)
        })
        .detach();
        cx.observe(&host.store, |this: &mut Updater, _, cx| {
            this.maybe_popup(cx)
        })
        .detach();
        Updater {
            phase: error.map_or(Phase::Idle, Phase::Failed),
            release: None,
            prepared: None,
            installation: tokativ_updater::installation().unwrap_or(Installation::Managed),
            notified: None,
            recovery,
        }
    });
    cx.set_global(Service(updater.clone()));
    if recovery {
        cx.defer(show);
    }
    // Sandboxes and Linux package-manager installations never check automatically.
    if Platform::current().is_some() && tokativ_config::sandbox().is_none() {
        updater.update(cx, |_, cx| {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(15))
                    .await;
                loop {
                    if this
                        .update(cx, |this, cx| {
                            if Host::global(cx).store.read(cx).config().check_for_updates
                                && !this.recovery
                            {
                                this.check(false, cx);
                            }
                        })
                        .is_err()
                    {
                        return;
                    }
                    cx.background_executor().timer(INTERVAL).await;
                }
            })
            .detach();
        });
    }
}

pub fn open(cx: &mut App) {
    if let Some(service) = cx.try_global::<Service>() {
        let updater = service.0.clone();
        updater.update(cx, |this, cx| this.check(true, cx));
        show(cx);
    }
}

pub fn installing(cx: &App) -> bool {
    cx.try_global::<Service>()
        .is_some_and(|service| matches!(service.0.read(cx).phase, Phase::Installing))
}

fn show(cx: &mut App) {
    #[cfg(target_os = "macos")]
    cx.activate(true);
    if let Some(handle) = cx.try_global::<Open>().map(|open| open.0)
        && handle
            .update(cx, |_, window, _| window.activate_window())
            .is_ok()
    {
        return;
    }
    let Some(service) = cx.try_global::<Service>() else {
        return;
    };
    let updater = service.0.clone();
    let mut options = chrome::options("Tokativ update", Size::new(px(520.), px(430.)), cx);
    options.window_min_size = Some(Size::new(px(440.), px(340.)));
    options.is_resizable = false;
    match cx.open_window(options, |window, cx| {
        window.set_rem_size(cx.theme().font_size);
        cx.new(|cx| {
            cx.observe(&updater, |_: &mut UpdateWindow, _, cx| cx.notify())
                .detach();
            let status = Host::global(cx).status;
            cx.observe(&status, |_: &mut UpdateWindow, _, cx| cx.notify())
                .detach();
            UpdateWindow { updater }
        })
    }) {
        Ok(handle) => cx.set_global(Open(handle)),
        Err(error) => log::error!("updates: cannot open the popup: {error:#}"),
    }
}

impl Updater {
    fn check(&mut self, manual: bool, cx: &mut Context<Self>) {
        if matches!(
            self.phase,
            Phase::Checking | Phase::Downloading { .. } | Phase::Ready | Phase::Installing
        ) {
            return;
        }
        let Some(platform) = Platform::current() else {
            return;
        };
        self.recovery = false;
        self.phase = Phase::Checking;
        self.release = None;
        cx.notify();
        let (sender, receiver) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = sender.send(tokativ_updater::check(CURRENT, platform));
        });
        cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("The update check stopped unexpectedly")));
            this.update(cx, |this, cx| {
                match result {
                    Ok(Some(release)) => {
                        if manual {
                            this.notified = Some(release.version.clone());
                        }
                        this.release = Some(release);
                        this.phase = Phase::Available;
                    }
                    Ok(None) => this.phase = Phase::Latest,
                    Err(error) => {
                        log::warn!("updates: {error:#}");
                        this.phase = Phase::Failed(format!("{error:#}"));
                    }
                }
                if !manual {
                    this.maybe_popup(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn maybe_popup(&mut self, cx: &mut Context<Self>) {
        let host = Host::global(cx);
        if !matches!(self.phase, Phase::Available)
            || host.status.read(cx).recording
            || !host.store.read(cx).config().setup_done
            || !host.store.read(cx).config().check_for_updates
        {
            return;
        }
        if let Some(release) = &self.release
            && self.notified.as_ref() != Some(&release.version)
        {
            self.notified = Some(release.version.clone());
            cx.defer(show);
        }
    }

    fn download(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.phase, Phase::Available | Phase::Failed(_)) {
            return;
        }
        let Some(release) = self.release.clone() else {
            return;
        };
        if !self.installation.can_install() {
            cx.open_url(&release.page);
            return;
        }
        self.phase = Phase::Downloading { done: 0, total: 0 };
        cx.notify();
        let installation = self.installation.clone();
        let progress = Arc::new(Mutex::new((0, 0)));
        let reported = progress.clone();
        let (sender, mut receiver) = oneshot::channel();
        std::thread::spawn(move || {
            let result = tokativ_updater::download(&release, installation, |done, total| {
                *reported.lock().expect("update progress") = (done, total);
            });
            let _ = sender.send(result);
        });
        cx.spawn(async move |this, cx| {
            loop {
                match receiver.try_recv() {
                    Ok(Some(result)) => {
                        this.update(cx, |this, cx| {
                            match result {
                                Ok(prepared) => {
                                    this.prepared = Some(prepared);
                                    this.phase = Phase::Ready;
                                }
                                Err(error) => this.phase = Phase::Failed(format!("{error:#}")),
                            }
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                    Err(_) => {
                        this.update(cx, |this, cx| {
                            this.phase = Phase::Failed("The download stopped unexpectedly".into());
                            cx.notify();
                        })
                        .ok();
                        break;
                    }
                    Ok(None) => {
                        let (done, total) = *progress.lock().expect("update progress");
                        if this
                            .update(cx, |this, cx| {
                                this.phase = Phase::Downloading { done, total };
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }
                cx.background_executor()
                    .timer(Duration::from_millis(120))
                    .await;
            }
        })
        .detach();
    }

    fn install(&mut self, cx: &mut Context<Self>) {
        if Host::global(cx).status.read(cx).recording {
            return;
        }
        let Some(prepared) = self.prepared.take() else {
            return;
        };
        self.phase = Phase::Installing;
        cx.notify();
        let (sender, receiver) = oneshot::channel();
        std::thread::spawn(move || {
            let _ = sender.send(prepared.install());
        });
        cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err(anyhow::anyhow!("The update helper did not start")));
            this.update(cx, |this, cx| match result {
                Ok(()) => Host::global(cx).dispatch(Action::Quit, cx),
                Err(error) => {
                    this.phase = Phase::Failed(format!("{error:#}"));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

struct UpdateWindow {
    updater: Entity<Updater>,
}

impl Render for UpdateWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let updater = self.updater.read(cx);
        let phase = updater.phase.clone();
        let release = updater.release.clone();
        let can_install = updater.installation.can_install();
        let recording = Host::global(cx).status.read(cx).recording;
        let version = release
            .as_ref()
            .map_or(CURRENT, |release| release.version.as_str());
        let (title, detail) = match &phase {
            Phase::Idle | Phase::Checking => (
                "Checking for updates".into(),
                format!("You have Tokativ {CURRENT}."),
            ),
            Phase::Latest => (
                "You're up to date".into(),
                format!("Tokativ {CURRENT} is the latest version."),
            ),
            Phase::Available => (
                format!("Tokativ {version} is available"),
                if can_install {
                    "Download the update, then restart Tokativ to install it.".into()
                } else {
                    "Download the app, or update with the package manager used to install Tokativ."
                        .into()
                },
            ),
            Phase::Downloading { done, total } => (
                format!("Downloading Tokativ {version}"),
                if *total == 0 {
                    "Starting the download…".into()
                } else {
                    format!("{:.0}% downloaded", *done as f64 / *total as f64 * 100.)
                },
            ),
            Phase::Ready => (
                "Ready to install".into(),
                if recording {
                    "Finish your dictation before restarting Tokativ.".into()
                } else {
                    "Tokativ will close and reopen with the update. Your settings and models will stay in place.".into()
                },
            ),
            Phase::Installing => (
                "Installing update".into(),
                "Closing Tokativ and starting the installer…".into(),
            ),
            Phase::Failed(error) => ("Could not update Tokativ".into(), error.clone()),
        };
        let theme = cx.theme();
        let mut actions = Vec::new();
        if let Some(release) = &release {
            let page = release.page.clone();
            actions.push(
                Button::new("release-notes")
                    .label("Release notes")
                    .ghost()
                    .on_click(move |_, _, cx| cx.open_url(&page))
                    .into_any_element(),
            );
        }
        actions.push(
            Button::new("later")
                .label(if matches!(phase, Phase::Available | Phase::Ready) {
                    "Later"
                } else {
                    "Close"
                })
                .secondary()
                .on_click(|_, window, _| window.remove_window())
                .into_any_element(),
        );
        let service = self.updater.clone();
        match &phase {
            Phase::Available => actions.push(
                Button::new("download-update")
                    .label(if can_install {
                        "Download update"
                    } else {
                        "Open download"
                    })
                    .primary()
                    .on_click(move |_, _, cx| service.update(cx, |this, cx| this.download(cx)))
                    .into_any_element(),
            ),
            Phase::Ready => actions.push(
                Button::new("install-update")
                    .label("Restart and install")
                    .primary()
                    .disabled(recording)
                    .on_click(move |_, _, cx| service.update(cx, |this, cx| this.install(cx)))
                    .into_any_element(),
            ),
            Phase::Failed(_) => {
                let has_release = release.is_some();
                actions.push(
                    Button::new("retry-update")
                        .label("Try again")
                        .primary()
                        .on_click(move |_, _, cx| {
                            service.update(cx, |this, cx| {
                                if has_release {
                                    this.download(cx);
                                } else {
                                    this.check(true, cx);
                                }
                            })
                        })
                        .into_any_element(),
                );
            }
            _ => {}
        }
        let notes = release
            .as_ref()
            .filter(|_| matches!(phase, Phase::Available))
            .map(|release| release.notes.chars().take(4000).collect::<String>());
        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .p_6()
            .gap_4()
            .child(
                div()
                    .text_size(theme.text(Text::Title))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(SharedString::from(title)),
            )
            .child(
                div()
                    .id("update-message")
                    .max_h(px(110.))
                    .overflow_y_scroll()
                    .text_color(theme.muted_foreground)
                    .child(detail),
            )
            .when_some(notes.filter(|notes| !notes.is_empty()), |this, notes| {
                this.child(
                    div()
                        .id("update-notes")
                        .max_h(px(160.))
                        .overflow_y_scroll()
                        .text_size(theme.text(Text::Small))
                        .child(notes),
                )
            })
            .when_some(
                match phase {
                    Phase::Downloading { done, total } if total > 0 => {
                        Some(done as f32 / total as f32)
                    }
                    _ => None,
                },
                |this, progress| this.child(chrome::meter(progress, px(8.), cx)),
            )
            .child(div().flex_1())
            .child(div().flex().justify_end().gap_2().children(actions))
            .into_any_element();
        chrome::frame("Update", content, window, cx)
    }
}
