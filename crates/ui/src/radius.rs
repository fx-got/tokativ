//! The one rule for nested corners. An element inset by `padding` on every side inside a
//! rounded parent takes the parent's radius minus that padding, so both curves share a center.

use gpui::Pixels;

/// The corner radius for an element inset by `padding` on all sides inside a parent rounded by
/// `outer`. Never negative, so a padding wider than the parent's curve gives a square corner.
pub fn nested_radius(outer: Pixels, padding: Pixels) -> Pixels {
    (outer - padding).max(Pixels::ZERO)
}

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::*;

    #[test]
    fn corners_stay_concentric() {
        assert_eq!(nested_radius(px(24.), px(6.)), px(18.));
        assert_eq!(nested_radius(px(8.), px(12.)), px(0.));
    }
}
