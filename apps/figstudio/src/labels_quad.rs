//! Narrow phase: a transformed frame's convex quad against an axis-aligned label.
use egui::{Pos2, Rect, Vec2};

pub(super) fn overlaps(rect: Rect, quad: &[Pos2; 4]) -> bool {
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    // Screen x/y axes are already checked by the grid's AABB pre-filter.
    (0..2).all(|i| {
        let edge = quad[i + 1] - quad[i];
        let axis = Vec2::new(-edge.y, edge.x);
        if axis == Vec2::ZERO {
            return false;
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
