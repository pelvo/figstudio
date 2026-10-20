//! Canvas click dispatch keeps the external selection at an instance's outer node.
use crate::{instance_pick, pick, tab::Tab};
use figstudio_fig::{Guid, resolve::InstancePath};
use figstudio_render::kurbo::Point;

pub fn hover(tab: &Tab, point: Option<Point>, deep: bool) -> Option<InstancePath> {
    let instance = tab.hover?;
    if !tab.doc.node(instance).is_some_and(|n| n.kind == "INSTANCE")
        || !(deep || (tab.selection == tab.hover && tab.inner.is_some()))
    {
        return None;
    }
    tab.pick_cache.hover(tab, point?, deep)
}

pub fn outline(
    painter: &egui::Painter,
    tab: &Tab,
    origin: egui::Pos2,
    path: &InstancePath,
    stroke: egui::Stroke,
) {
    if let Some(corners) = tab.pick_cache.corners(&tab.doc, path) {
        let corners = corners.map(|p| {
            tab.view
                .to_screen(origin, egui::pos2(p.x as f32, p.y as f32))
        });
        outline_corners(painter, corners, stroke);
    }
}

pub fn outline_corners(painter: &egui::Painter, corners: [egui::Pos2; 4], stroke: egui::Stroke) {
    let axis_aligned =
        (corners[0].y - corners[1].y).abs() < 0.01 && (corners[0].x - corners[3].x).abs() < 0.01;
    if axis_aligned {
        painter.rect_stroke(
            egui::Rect::from_two_pos(corners[0], corners[2]),
            0.0,
            stroke,
            egui::StrokeKind::Middle,
        );
    } else {
        painter.add(egui::Shape::closed_line(corners.to_vec(), stroke));
    }
}

/// Returns whether this click changed the inner target (a drill is not text editing yet).
pub fn click(tab: &mut Tab, chain: &[Guid], point: Option<Point>, deep: bool, drill: bool) -> bool {
    let previous = tab.selection;
    let old_inner = tab.inner.clone();
    let picked = if drill {
        pick::deeper(chain, tab.hover)
    } else {
        tab.hover
    };
    tab.select(picked);
    if let Some(instance) = picked
        && tab.doc.node(instance).is_some_and(|n| n.kind == "INSTANCE")
        && (deep || (previous == picked && (drill || tab.inner.is_some())))
        && let Some(point) = point
    {
        let path = tab.pick_cache.with(&tab.doc, |renderer, definitions| {
            let hits = instance_pick::hit_chain_with(&tab.doc, renderer, instance, point);
            instance_pick::click(
                &tab.doc,
                definitions,
                &hits,
                tab.inner.as_ref(),
                deep,
                drill,
            )
        });
        tab.set_inner(path);
    }
    tab.inner != old_inner
}
