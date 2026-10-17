//! The model list and the Hugging Face search, shared by the settings and setup windows.

use gpui::prelude::*;
use gpui::{AnyElement, App, ClickEvent, Entity, SharedString, Window, div, px};
use tokativ_engine::ModelId;
use tokativ_engine::models::{self, Repository};
use ui::{ActiveTheme as _, Button, Text};

use crate::chrome::{self, meter};
use crate::host::{Files, Library, ModelState};

type Choose = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// What a model row says about the model, beyond its state on disk.
pub(crate) struct Described {
    pub id: ModelId,
    pub name: String,
    pub maker: Option<String>,
    pub summary: Option<String>,
}

impl Described {
    /// The description for a model: the curated one for a suggested model, the repository and
    /// file for anything else.
    pub fn of(id: ModelId) -> Self {
        match models::suggested(&id) {
            Some(suggested) => Self {
                name: suggested.name.to_owned(),
                maker: Some(suggested.maker.to_owned()),
                summary: Some(suggested.summary.to_owned()),
                id,
            },
            None => Self {
                name: id.name(),
                maker: None,
                summary: Some(id.repo.clone()),
                id,
            },
        }
    }
}

/// The models to list before any search: everything installed, then the suggested models that
/// are not, then any other download in progress.
pub(crate) fn listed(library: &Library, also: &[ModelId]) -> Vec<Described> {
    let mut ids: Vec<ModelId> = library
        .installed_models()
        .iter()
        .map(|model| model.id.clone())
        .collect();
    for suggested in models::SUGGESTED {
        if let Some(id) = ModelId::parse(suggested.id)
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    for id in also {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    ids.into_iter().map(Described::of).collect()
}

/// One model's row: its name and maker, what it is good at, where it stands on disk, and the
/// button that changes that. `leading` goes in front, such as a radio. `choose` runs when an
/// installed model's row is clicked.
pub(crate) fn model_row(
    model: Described,
    library: &Entity<Library>,
    leading: Option<AnyElement>,
    choose: Option<Choose>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let state = library.read(cx).state(&model.id);
    let (status, progress) = match &state {
        ModelState::Missing => ("Not downloaded".to_string(), None),
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
    let key = model.id.key();
    let id = model.id.clone();
    let action = {
        let library = library.clone();
        match state {
            ModelState::Downloading { .. } => {
                Button::new(SharedString::from(format!("cancel-{key}")))
                    .label("Cancel")
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| library.update(cx, |library, _| library.cancel(&id)))
            }
            ModelState::Installed { .. } => {
                Button::new(SharedString::from(format!("delete-{key}")))
                    .label("Delete")
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| {
                        library.update(cx, |library, cx| library.delete(&id, cx))
                    })
            }
            ModelState::Missing | ModelState::Failed(_) => {
                Button::new(SharedString::from(format!("download-{key}")))
                    .label("Download")
                    .small()
                    .primary()
                    .on_click(move |_, _, cx| {
                        let id = id.clone();
                        library.update(cx, |library, cx| library.download(id, cx))
                    })
            }
        }
    };
    let installed = matches!(state, ModelState::Installed { .. });
    let small = theme.text(Text::Small);
    div()
        .id(SharedString::from(format!("model-{key}")))
        .flex()
        .items_start()
        .gap_3()
        .px(chrome::GROUP_PADDING)
        .py_3()
        .when_some(choose.filter(|_| installed), |this, choose| {
            this.cursor_pointer().on_click(choose)
        })
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
                        .child(model.name)
                        .when_some(model.maker, |this, maker| {
                            this.child(
                                div()
                                    .text_size(small)
                                    .text_color(theme.muted_foreground)
                                    .child(maker),
                            )
                        }),
                )
                .when_some(model.summary, |this, summary| {
                    this.child(
                        div()
                            .text_size(small)
                            .text_color(theme.muted_foreground)
                            .child(summary),
                    )
                })
                .when_some(progress, |this, progress| {
                    this.child(div().pt_1().child(meter(progress, px(6.), cx)))
                })
                .child(
                    div()
                        .text_size(small)
                        .text_color(theme.muted_foreground.opacity(0.75))
                        .child(status),
                ),
        )
        .child(div().flex_none().child(action))
        .into_any_element()
}

/// One repository found by search. Clicking it lists its files, one row per quantization.
pub(crate) fn repository_row(
    repository: &Repository,
    open: bool,
    library: &Entity<Library>,
    toggle: Choose,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let small = theme.text(Text::Small);
    let (owner, name) = repository
        .id
        .split_once('/')
        .unwrap_or(("", repository.id.as_str()));
    let facts = format!(
        "{}  ·  {} downloads",
        repository.architecture.as_deref().unwrap_or("unknown"),
        count(repository.downloads)
    );
    let header = div()
        .id(SharedString::from(format!("repo-{}", repository.id)))
        .flex()
        .items_center()
        .gap_3()
        .px(chrome::GROUP_PADDING)
        .py_3()
        .cursor_pointer()
        .on_click(toggle)
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
                        .child(name.to_owned())
                        .child(
                            div()
                                .text_size(small)
                                .text_color(theme.muted_foreground)
                                .child(owner.to_owned()),
                        ),
                )
                .child(
                    div()
                        .text_size(small)
                        .text_color(theme.muted_foreground)
                        .child(facts),
                ),
        )
        .child(
            gpui::svg()
                .path(icons::path(match open {
                    true => "icons/chevron-up.svg",
                    false => "icons/chevron-down.svg",
                }))
                .size(px(16.))
                .flex_none()
                .text_color(theme.muted_foreground),
        );

    let files = open.then(
        || match library.read(cx).search.files.get(&repository.id).cloned() {
            None | Some(Files::Loading) => vec![note("Listing files", cx)],
            Some(Files::Failed(error)) => vec![note(&format!("Cannot list files: {error}"), cx)],
            Some(Files::Loaded(files)) if files.is_empty() => {
                vec![note("No GGUF files in this repository", cx)]
            }
            Some(Files::Loaded(files)) => files
                .into_iter()
                .map(|file| {
                    let id = ModelId {
                        repo: repository.id.clone(),
                        file: file.name.clone(),
                    };
                    let name = match file.quantization() {
                        Some(quant) => format!("{quant}  ·  {}", chrome::size(file.size)),
                        None => format!("{}  ·  {}", file.name, chrome::size(file.size)),
                    };
                    model_row(
                        Described {
                            id,
                            name,
                            maker: None,
                            summary: None,
                        },
                        library,
                        None,
                        None,
                        cx,
                    )
                })
                .collect(),
        },
    );

    div()
        .child(header)
        .when_some(files, |this, files| {
            this.child(
                div()
                    .pl(chrome::GROUP_PADDING)
                    .border_t_1()
                    .border_color(theme.border.opacity(0.5))
                    .children(files),
            )
        })
        .into_any_element()
}

/// A muted line inside a group, for empty and waiting states.
pub(crate) fn note(text: &str, cx: &App) -> AnyElement {
    div()
        .px(chrome::GROUP_PADDING)
        .py_3()
        .text_size(cx.theme().text(Text::Small))
        .text_color(cx.theme().muted_foreground)
        .child(text.to_owned())
        .into_any_element()
}

/// A count as people read one: 1.9M, 640K, 812.
fn count(value: u64) -> String {
    match value {
        0..1_000 => value.to_string(),
        1_000..1_000_000 => format!("{:.0}K", value as f64 / 1_000.),
        _ => format!("{:.1}M", value as f64 / 1_000_000.),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_short() {
        assert_eq!(count(812), "812");
        assert_eq!(count(640_400), "640K");
        assert_eq!(count(1_951_595), "2.0M");
    }
}
