//! The StatusNotifierItem in the panel, through ksni. Menu clicks travel to the daemon over a
//! channel, and the daemon pushes the recording state and language mode back.

use anyhow::{Context as _, Result};
use futures::channel::mpsc::UnboundedSender;
use ksni::blocking::{Handle, TrayMethods as _};
use ksni::menu::{MenuItem, RadioGroup, RadioItem, StandardItem};
use tokativ_config::{APP_ID, LanguageMode};

const SIZE: i32 = 32;

/// What a tray click asks of the daemon.
#[derive(Clone, Copy, Debug)]
pub enum TrayEvent {
    Toggle,
    Lang(LanguageMode),
    Settings,
    Quit,
}

/// The running tray item. Dropping it removes the item from the panel.
pub struct Tray {
    handle: Handle<Item>,
}

impl Tray {
    pub fn spawn(
        events: UnboundedSender<TrayEvent>,
        recording: bool,
        mode: LanguageMode,
    ) -> Result<Self> {
        let item = Item {
            events,
            recording,
            mode,
        };
        let handle = item
            .spawn()
            .context("cannot reach the status notifier host")?;
        Ok(Self { handle })
    }

    pub fn show(&self, recording: bool, mode: LanguageMode) {
        self.handle.update(|item| {
            item.recording = recording;
            item.mode = mode;
        });
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        drop(self.handle.shutdown());
    }
}

struct Item {
    events: UnboundedSender<TrayEvent>,
    recording: bool,
    mode: LanguageMode,
}

impl Item {
    fn send(&self, event: TrayEvent) {
        self.events.unbounded_send(event).ok();
    }

    fn entry(label: &str, event: TrayEvent) -> MenuItem<Self> {
        StandardItem {
            label: label.into(),
            activate: Box::new(move |item: &mut Self| item.send(event)),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        APP_ID.into()
    }

    fn title(&self) -> String {
        "Tokativ".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_name(&self) -> String {
        match self.recording {
            true => "media-record-symbolic".into(),
            false => "audio-input-microphone-symbolic".into(),
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        vec![pixmap(self.recording)]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Tokativ".into(),
            description: match self.recording {
                true => "Dictating".into(),
                false => format!("Ready, {}", self.mode.name()),
            },
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayEvent::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let toggle = match self.recording {
            true => "Stop dictation",
            false => "Start dictation",
        };
        vec![
            Self::entry(toggle, TrayEvent::Toggle),
            MenuItem::Separator,
            RadioGroup {
                selected: LanguageMode::ALL
                    .iter()
                    .position(|mode| *mode == self.mode)
                    .unwrap_or(0),
                select: Box::new(|item: &mut Self, index| {
                    if let Some(mode) = LanguageMode::ALL.get(index) {
                        item.mode = *mode;
                        item.send(TrayEvent::Lang(*mode));
                    }
                }),
                options: LanguageMode::ALL
                    .iter()
                    .map(|mode| RadioItem {
                        label: mode.name().into(),
                        ..Default::default()
                    })
                    .collect(),
            }
            .into(),
            MenuItem::Separator,
            Self::entry("Settings", TrayEvent::Settings),
            Self::entry("Quit", TrayEvent::Quit),
        ]
    }
}

/// A plain icon for hosts without the themed names: a hollow ring at rest, a solid red dot
/// while recording. ARGB in network byte order, as the protocol wants.
fn pixmap(recording: bool) -> ksni::Icon {
    let center = (SIZE as f32 - 1.) / 2.;
    let outer = SIZE as f32 * 0.42;
    let inner = SIZE as f32 * 0.18;
    let mut data = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let distance = ((x as f32 - center).powi(2) + (y as f32 - center).powi(2)).sqrt();
            let coverage = (outer + 0.5 - distance).clamp(0., 1.);
            let (alpha, rgb) = match recording {
                true => (coverage, [0xE5, 0x3E, 0x3E]),
                false => {
                    let ring = coverage * (distance - (outer - 3.) + 0.5).clamp(0., 1.);
                    let dot = (inner + 0.5 - distance).clamp(0., 1.);
                    (ring.max(dot), [0xF2, 0xF2, 0xF2])
                }
            };
            data.extend([(alpha * 255.) as u8, rgb[0], rgb[1], rgb[2]]);
        }
    }
    ksni::Icon {
        width: SIZE,
        height: SIZE,
        data,
    }
}
