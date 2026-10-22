//! Narrow phase: a transformed frame's convex quad against an axis-aligned label.
use egui::{Pos2, Rect, Vec2};

pub(super) fn overlaps(rect: Rect, quad: &[Pos2; 4]) -> bool {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    if !rect.is_finite() || quad.iter().any(|p| !p.is_finite()) {
        return false;
    }
    let normal = |edge: Vec2| Vec2::new(-edge.y, edge.x);
    [
        Vec2::X,
        Vec2::Y,
        normal(quad[1] - quad[0]),
        normal(quad[2] - quad[1]),
    ]
    .into_iter()
    .all(|axis| {
        if axis == Vec2::ZERO {
            return true;
        }
        let project = |points: &[Pos2; 4]| {
            points
                .iter()
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                    let d = p.to_vec2().dot(axis);
                    (lo.min(d), hi.max(d))
                })
        };
        let (a, b) = project(quad);
        let (c, d) = project(&corners);
        a < d && c < b
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapsed_frame_segments_and_points_block_only_where_they_cross() {
        let rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(10.0, 10.0));
        for (a, b, expected) in [
            ((5.0, -5.0), (5.0, 15.0), true),
            ((-5.0, 5.0), (15.0, 5.0), true),
            ((-5.0, -5.0), (15.0, 15.0), true),
            ((-5.0, 8.0), (8.0, 15.0), false),
            ((5.0, 5.0), (5.0, 5.0), true),
            ((15.0, 5.0), (15.0, 5.0), false),
            ((0.0, -5.0), (0.0, 15.0), false),
            ((0.0, 0.0), (0.0, 0.0), false),
        ] {
            let a = Pos2::from(a);
            let b = Pos2::from(b);
            assert_eq!(overlaps(rect, &[a, b, b, a]), expected, "{a:?} to {b:?}");
        }
    }

    #[test]
    fn broad_phase_keeps_zero_extent_frame_bounds() {
        let mut grid = super::super::spatial::Grid::default();
        grid.insert(
            Rect::from_min_max(Pos2::new(5.0, -5.0), Pos2::new(5.0, 15.0)),
            (),
        );
        assert!(grid.any(
            Rect::from_min_max(Pos2::ZERO, Pos2::new(10.0, 10.0)),
            |_| true
        ));
    }
}
