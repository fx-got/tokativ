//! The windows that float over the desktop while a session runs, the pill and the caption: a
//! layer surface on Wayland and a floating panel elsewhere, never taking focus from the app
//! being dictated into.

use gpui::{
    App, Bounds, DisplayId, Pixels, Point, Size, Window, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, point,
};
#[cfg(target_os = "linux")]
use gpui::{
    layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions},
    px,
};
use tokativ_config::{APP_ID, PillEdge};

/// The options for an overlay of `size` whose near side sits `offset` from the `edge` of the
/// focused output, or of the primary display off Linux, centred along that edge.
pub(crate) fn options(
    edge: PillEdge,
    offset: Pixels,
    size: Size<Pixels>,
    display: Option<DisplayId>,
    cx: &App,
) -> WindowOptions {
    let (kind, origin) = placement(edge, offset, size, display, cx);
    #[allow(
        unused_mut,
        reason = "only the floating window off Linux changes the options"
    )]
    let mut options = WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds { origin, size })),
        app_id: Some(APP_ID.into()),
        window_background: WindowBackgroundAppearance::Transparent,
        // An overlay never has focus, and without this its animations would drop to the
        // throttled frame rate GPUI gives inactive windows.
        inactive_frame_interval: None,
        display_id: display,
        kind,
        ..Default::default()
    };
    // A layer surface never takes focus. A floating window has to be told, or showing it would
    // pull focus from the app being dictated into.
    #[cfg(not(target_os = "linux"))]
    {
        options.focus = false;
        options.is_movable = false;
        options.is_resizable = false;
        options.is_minimizable = false;
    }
    options
}

/// Removes system decoration from the overlay and keeps it from taking keyboard focus.
/// With `click_through` the pointer reaches whatever is below.
pub(crate) fn prepare(window: &mut Window, click_through: bool) {
    if click_through {
        window.set_input_region(Some(&[]));
    }
    #[cfg(target_os = "macos")]
    frameless(window, click_through);
    #[cfg(windows)]
    prepare_windows(window, click_through);
}

/// A layer surface anchored to the focused output's edge.
#[cfg(target_os = "linux")]
fn placement(
    edge: PillEdge,
    offset: Pixels,
    _size: Size<Pixels>,
    _display: Option<DisplayId>,
    _cx: &App,
) -> (WindowKind, Point<Pixels>) {
    let (anchor, margin) = match edge {
        PillEdge::Bottom => (Anchor::BOTTOM, (px(0.), px(0.), offset, px(0.))),
        PillEdge::Top => (Anchor::TOP, (offset, px(0.), px(0.), px(0.))),
    };
    let kind = WindowKind::LayerShell(LayerShellOptions {
        namespace: APP_ID.into(),
        layer: Layer::Overlay,
        anchor,
        exclusive_zone: Some(px(0.)),
        margin: Some(margin),
        keyboard_interactivity: KeyboardInteractivity::None,
        ..Default::default()
    });
    (kind, point(px(0.), px(0.)))
}

/// A floating window on the primary display, inside the area the dock or taskbar leaves free.
#[cfg(not(target_os = "linux"))]
fn placement(
    edge: PillEdge,
    offset: Pixels,
    size: Size<Pixels>,
    display: Option<DisplayId>,
    cx: &App,
) -> (WindowKind, Point<Pixels>) {
    let area = display
        .and_then(|id| cx.find_display(id))
        .or_else(|| cx.primary_display())
        .map(|display| display.visible_bounds())
        .unwrap_or_default();
    let x = area.origin.x + (area.size.width - size.width) / 2.;
    let y = match edge {
        PillEdge::Bottom => area.origin.y + area.size.height - size.height - offset,
        PillEdge::Top => area.origin.y + offset,
    };
    (WindowKind::PopUp, point(x, y))
}

/// Strips the frame macOS draws around a title-less GPUI window: the rounded border and the
/// system shadow, which would box the overlay in. The overlay draws its own shadow. The window
/// stays a non-activating panel, so it still never takes focus.
#[cfg(target_os = "macos")]
fn frameless(window: &Window, click_through: bool) {
    use objc2_app_kit::{NSView, NSWindowStyleMask};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // GPUI's own `window_handle` names the window inside GPUI, not the platform window.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return;
    };
    // SAFETY: the pointer is the content view of a window this thread just created and still
    // owns, so it is a live NSView on the main thread.
    let view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let Some(native) = view.window() else {
        return;
    };
    native.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    native.setHasShadow(false);
    native.setIgnoresMouseEvents(click_through);
}

/// GPUI uses a transparent pop-up, but DWM still draws a border around its rectangular surface.
/// The pill/caption draw their own outline, corners and shadow; the native surface must not.
/// Also prevent activation and let the pointer pass through a caption.
#[cfg(windows)]
fn prepare_windows(window: &Window, click_through: bool) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMNCRP_DISABLED, DWMSBT_NONE, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE,
        DWMWA_NCRENDERING_POLICY, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_WINDOW_CORNER_PREFERENCE,
        DWMWCP_DONOTROUND, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TRANSPARENT,
    };

    // GPUI's own `window_handle` names the window inside GPUI, not the platform window.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let window = handle.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    let mut added = WS_EX_NOACTIVATE;
    if click_through {
        added |= WS_EX_TRANSPARENT | WS_EX_LAYERED;
    }
    // SAFETY: the handle belongs to a window this thread just created and still owns.
    unsafe {
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE);
        SetWindowLongPtrW(window, GWL_EXSTYLE, style | added as isize);
        for (attribute, value) in [
            (DWMWA_NCRENDERING_POLICY, DWMNCRP_DISABLED as u32),
            (DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE),
            (DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND as u32),
            (DWMWA_SYSTEMBACKDROP_TYPE, DWMSBT_NONE as u32),
        ] {
            // All these attributes take a four-byte enum or COLORREF. Older Windows versions
            // may reject the Windows 11 attributes; non-client rendering is disabled there too.
            let _ = DwmSetWindowAttribute(
                window,
                attribute as u32,
                (&value as *const u32).cast(),
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
}
