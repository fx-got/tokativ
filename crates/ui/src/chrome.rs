//! The frame shared by the settings and setup windows: a client-drawn title bar with Sonora's
//! window controls, rounded corners, and the resize edges.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Decorations, MouseButton, Pixels, Size, Window,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowOptions, div, px,
};
use tokativ_config::APP_ID;
use ui::{ActiveTheme as _, Text, WindowControls, WindowFrame};

/// The options for a normal toplevel of `size` with client-side decorations.
pub fn options(title: &str, size: Size<Pixels>, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size, cx))),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some(title.to_owned().into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        app_id: Some(APP_ID.into()),
        window_background: WindowBackgroundAppearance::Transparent,
        window_decorations: Some(WindowDecorations::Client),
        window_min_size: Some(Size::new(px(480.), px(420.))),
        is_movable: true,
        is_resizable: true,
        app_owns_titlebar_drag: true,
        ..Default::default()
    }
}

/// The corner radius of the window itself. Square when tiled, maximized or decorated by the
/// compositor, since those sit flush against something.
pub fn window_radius(window: &Window, cx: &App) -> Pixels {
    match window.window_decorations() {
        Decorations::Client { tiling } if !tiling.is_tiled() && !window.is_maximized() => {
            cx.theme().radius
        }
        _ => Pixels::ZERO,
    }
}

/// Wraps `content` in the window frame with a title bar reading `title`.
pub fn frame(title: &str, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = cx.theme();
    let radius = window_radius(window, cx);
    let height = ui::snapped(theme.metrics.title_bar, window);
    let client = matches!(window.window_decorations(), Decorations::Client { .. });

    let title_bar = div()
        .id("title-bar")
        .flex()
        .flex_none()
        .items_center()
        .justify_between()
        .w_full()
        .h(height)
        .pl_4()
        .bg(theme.background)
        .border_b_1()
        .border_color(theme.title_bar_border)
        .window_control_area(gpui::WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            match event.click_count {
                2 => window.zoom_window(),
                _ => window.start_window_move(),
            }
        })
        .child(
            div()
                .text_size(theme.text(Text::Label))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(title.to_owned()),
        )
        .when(client, |this| {
            this.child(div().flex_none().pr_2().child(WindowControls::new(false)))
        });

    div()
        .size_full()
        .flex()
        .flex_col()
        .font_family("Inter")
        .text_size(theme.text(Text::Body))
        .text_color(theme.foreground)
        .bg(theme.background)
        .rounded(radius)
        .overflow_hidden()
        // Middle-button auto-scrolling, wired the way Sonora's root wires it: any press ends the
        // mode, a held middle drag ends on release, Escape ends it, and the pointer steers it.
        .capture_any_mouse_down(|_, window, cx| {
            if ui::cancel_middle_scroll(cx) {
                window.refresh();
                cx.stop_propagation();
            }
        })
        .capture_any_mouse_up(|event, window, cx| {
            if event.button == MouseButton::Middle && ui::release_middle_scroll(event.position, cx)
            {
                window.refresh();
            }
        })
        .on_mouse_up_out(MouseButton::Middle, |event, window, cx| {
            if ui::release_middle_scroll(event.position, cx) {
                window.refresh();
            }
        })
        .capture_key_down(|event, window, cx| {
            if event.keystroke.key == "escape" && ui::cancel_middle_scroll(cx) {
                window.refresh();
                cx.stop_propagation();
            }
        })
        .on_mouse_move(|event, window, cx| ui::update_middle_scroll(event.position, window, cx))
        .child(title_bar)
        .child(div().flex_1().min_h_0().child(content))
        .child(WindowFrame::new())
        .into_any_element()
}

/// A section label in Sonora's settings style.
pub fn section(label: &str, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .h(theme.metrics.row)
        .flex()
        .flex_col()
        .justify_end()
        .pb_2()
        .pl_1()
        .child(ui::eyebrow(label.to_owned(), cx))
}

/// The corner radius of a settings group. Anything inset in a group takes its corners from this
/// through [`crate::nested_radius`].
pub const GROUP_RADIUS: Pixels = px(14.);
/// How far a group's rows sit from its edges.
pub const GROUP_PADDING: Pixels = px(16.);

/// A labelled group of settings rows on a raised surface, with a hairline between rows.
pub fn group(label: &str, rows: Vec<AnyElement>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let count = rows.len();
    div().child(section(label, cx)).child(
        div()
            .flex()
            .flex_col()
            .rounded(GROUP_RADIUS)
            .bg(theme.secondary.opacity(0.45))
            .border_1()
            .border_color(theme.border.opacity(0.6))
            .children(rows.into_iter().enumerate().map(move |(index, row)| {
                div()
                    .when(index + 1 < count, |this| {
                        this.border_b_1().border_color(theme.border.opacity(0.5))
                    })
                    .child(row)
            })),
    )
}

/// A settings row: a title with a muted line under it on the left, the control on the right.
pub fn row(
    title: &str,
    detail: impl Into<gpui::SharedString>,
    action: AnyElement,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .py_3()
        .px(GROUP_PADDING)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .min_w_0()
                        .text_color(theme.muted_foreground)
                        .text_size(theme.text(Text::Small))
                        .child(detail.into()),
                ),
        )
        .child(div().flex_none().child(action))
}

/// A horizontal track filled to `fraction`, with the fill's corners nested inside the track's.
pub fn meter(fraction: f32, height: Pixels, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let inset = px(2.);
    let outer = height / 2.;
    div()
        .w_full()
        .h(height)
        .p(inset)
        .rounded(outer)
        .bg(theme.muted)
        .child(
            div()
                .h_full()
                .w(gpui::relative(fraction.clamp(0., 1.)))
                .min_w(height - inset * 2.)
                .rounded(crate::nested_radius(outer, inset))
                .bg(theme.progress_bar),
        )
}

/// A box of monospaced text, such as a config snippet to copy, shaped like the cards around it.
pub fn code(text: &str, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .w_full()
        .px(GROUP_PADDING)
        .py_3()
        .rounded(GROUP_RADIUS)
        .bg(theme.secondary.opacity(0.45))
        .border_1()
        .border_color(theme.border.opacity(0.6))
        .font_family("monospace")
        .text_size(theme.text(Text::Small))
        .child(text.to_owned())
}

/// Bytes as megabytes or gigabytes, the way the model list shows sizes.
pub fn size(bytes: u64) -> String {
    let megabytes = bytes as f64 / 1_000_000.;
    match megabytes >= 1000. {
        true => format!("{:.1} GB", megabytes / 1000.),
        false => format!("{megabytes:.0} MB"),
    }
}
