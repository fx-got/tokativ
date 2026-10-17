//! Rectangles with continuous corners, the squircle shape iOS uses. A plain rounded corner joins
//! the edge with a visible bend where the arc starts. This one eases in from further along the
//! edge, so the outline curves without a seam.

use gpui::{Bounds, PathBuilder, Pixels, Point, point};

/// How far along each edge a corner of radius 1 starts, and the control points of the three
/// cubic curves it is drawn with. These are the constants of the iOS 7 continuous corner.
const START: f32 = 1.528_664_8;
const FIRST_A: f32 = 1.088_493_2;
const FIRST_B: f32 = 0.868_406_9;
const JOIN_ALONG: f32 = 0.631_494;
const JOIN_ACROSS: f32 = 0.074_911;
const MIDDLE_ALONG: f32 = 0.372_823_9;
const MIDDLE_ACROSS: f32 = 0.169_051_8;

/// Traces a squircle filling `bounds` with corners of `radius` into `builder` and builds it, or
/// gives `None` when the path cannot be built. The radius shrinks to fit a rectangle too small
/// for it.
pub(crate) fn path(
    bounds: Bounds<Pixels>,
    radius: Pixels,
    mut builder: PathBuilder,
) -> Option<gpui::Path<Pixels>> {
    let size = bounds.size;
    let fit = size.width.min(size.height) / 2. / START;
    let radius = radius.min(fit).max(Pixels::ZERO);
    let (left, top) = (bounds.origin.x, bounds.origin.y);
    let (right, bottom) = (left + size.width, top + size.height);
    // Each corner as the point it rounds off, the way the outline arrives and the way it leaves.
    let corners = [
        (point(right, top), (1., 0.), (0., 1.)),
        (point(right, bottom), (0., 1.), (-1., 0.)),
        (point(left, bottom), (-1., 0.), (0., -1.)),
        (point(left, top), (0., -1.), (1., 0.)),
    ];
    let at = |corner: Point<Pixels>, arrive: (f32, f32), leave: (f32, f32), back: f32, on: f32| {
        point(
            corner.x + radius * (leave.0 * on - arrive.0 * back),
            corner.y + radius * (leave.1 * on - arrive.1 * back),
        )
    };
    builder.move_to(point(left + radius * START, top));
    for (corner, arrive, leave) in corners {
        let p = |back: f32, on: f32| at(corner, arrive, leave, back, on);
        builder.line_to(p(START, 0.));
        builder.cubic_bezier_to(p(JOIN_ALONG, JOIN_ACROSS), p(FIRST_A, 0.), p(FIRST_B, 0.));
        builder.cubic_bezier_to(
            p(JOIN_ACROSS, JOIN_ALONG),
            p(MIDDLE_ALONG, MIDDLE_ACROSS),
            p(MIDDLE_ACROSS, MIDDLE_ALONG),
        );
        builder.cubic_bezier_to(p(0., START), p(0., FIRST_B), p(0., FIRST_A));
    }
    builder.close();
    builder.build().ok()
}
