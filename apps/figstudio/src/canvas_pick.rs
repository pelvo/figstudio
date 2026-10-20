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
    let hits = instance_pick::hit_chain(&tab.doc, instance, point?);
    instance_pick::click(&tab.doc, &hits, tab.inner.as_ref(), deep, false)
}

pub fn outline(
    painter: &egui::Painter,
    tab: &Tab,
    origin: egui::Pos2,
    path: &InstancePath,
    stroke: egui::Stroke,
) {
    if let Some(bounds) = figstudio_render::Renderer::new().path_bounds(&tab.doc, path) {
        let rect = egui::Rect::from_min_max(
            tab.view
                .to_screen(origin, egui::pos2(bounds.x0 as f32, bounds.y0 as f32)),
            tab.view
                .to_screen(origin, egui::pos2(bounds.x1 as f32, bounds.y1 as f32)),
        );
        painter.rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Middle);
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
        let hits = instance_pick::hit_chain(&tab.doc, instance, point);
        let path = instance_pick::click(&tab.doc, &hits, tab.inner.as_ref(), deep, drill);
        tab.set_inner(path);
    }
    tab.inner != old_inner
}
