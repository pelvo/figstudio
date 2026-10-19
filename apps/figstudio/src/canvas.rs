//! The canvas: pan / zoom input, background rendering, selection and hover outlines.

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use figstudio_fig::Guid;
use figstudio_render::kurbo::Point;
use figstudio_render::{
    absolute_transform, color::AlphaColor, content_bounds, geom, hit_test, page_background,
};

use crate::copy_menu;
use crate::marquee;
use crate::tab::{Tab, View};
use crate::theme;
use crate::tiles_view::Tiles;
use crate::worker::Workers;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Hand,
    Zoom,
}

pub fn fit(tab: &mut Tab, rect: Rect) {
    let Some(b) = content_bounds(&tab.doc, tab.page) else {
        tab.view = View {
            zoom: 1.0,
            pan: rect.size() / 2.0,
        };
        return;
    };
    fit_rect(
        tab,
        rect,
        Rect::from_min_max(
            Pos2::new(b.x0 as f32, b.y0 as f32),
            Pos2::new(b.x1 as f32, b.y1 as f32),
        ),
    );
}

/// Fits node `id` into `rect`; nothing happens when it has no bounds.
fn fit_node(tab: &mut Tab, rect: Rect, id: Guid) {
    if let Some(b) = figstudio_render::node_bounds(&tab.doc, id) {
        let page = Rect::from_min_max(
            Pos2::new(b.x0 as f32, b.y0 as f32),
            Pos2::new(b.x1 as f32, b.y1 as f32),
        );
        fit_rect(tab, rect, page);
    }
}

pub fn fit_rect(tab: &mut Tab, rect: Rect, page_rect: Rect) {
    let margin = 40.0;
    let avail = (rect.size() - Vec2::splat(margin * 2.0)).max(Vec2::splat(10.0));
    let size = page_rect.size().max(Vec2::splat(1.0));
    let zoom = (avail.x / size.x)
        .min(avail.y / size.y)
        .clamp(crate::tab::MIN_ZOOM, crate::tab::MAX_ZOOM);
    let centre = page_rect.center().to_vec2() * zoom;
    tab.view = View {
        zoom,
        pan: rect.size() / 2.0 - centre,
    };
}

/// Draws the canvas (with rounded corners of `radius`) and handles its input. Returns the
/// page-space cursor position.
pub fn show(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    workers: &mut Workers,
    tiles: &mut Tiles,
    tool: Tool,
    space_held: bool,
    radius: f32,
) -> Option<Pos2> {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let origin = rect.min;
    tab.canvas_rect = Some(rect);
    if tab.fit_pending && rect.width() > 10.0 {
        fit(tab, rect);
        tab.fit_pending = false;
    }
    if rect.width() > 10.0
        && let Some(id) = tab.fit_node.take()
    {
        fit_node(tab, rect, id);
    }

    // Input: pan.
    let panning = tool == Tool::Hand || space_held || ui.input(|i| i.pointer.middle_down());
    let mut direct = false;
    if response.dragged() && (panning || response.dragged_by(egui::PointerButton::Middle)) {
        tab.view.pan += response.drag_delta();
        direct = true;
    }
    if response.hovered() {
        let (scroll, zoom_delta, modifiers, pointer) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.zoom_delta(),
                i.modifiers,
                i.pointer.hover_pos(),
            )
        });
        let anchor = pointer.unwrap_or(rect.center());
        direct |= zoom_delta != 1.0 || scroll != Vec2::ZERO;
        if zoom_delta != 1.0 {
            tab.view.zoom_at(origin, anchor, zoom_delta);
        } else if scroll != Vec2::ZERO {
            if modifiers.command || modifiers.ctrl {
                tab.view.zoom_at(origin, anchor, (scroll.y / 200.0).exp());
            } else {
                tab.view.pan += scroll;
            }
        }
    }

    // Input: selection / zoom tool.
    let cursor_page = response.hover_pos().map(|p| tab.view.to_page(origin, p));
    if response.hovered() && !response.dragged() {
        tab.hover = cursor_page.and_then(|p| {
            hit_test(
                &tab.doc,
                tab.page,
                Point::new(f64::from(p.x), f64::from(p.y)),
            )
        });
    } else if !response.hovered() {
        tab.hover = None;
    }
    if response.clicked() && !panning {
        match tool {
            Tool::Select => tab.select(tab.hover),
            Tool::Zoom => {
                if let Some(p) = response.interact_pointer_pos() {
                    let alt = ui.input(|i| i.modifiers.alt);
                    tab.view.zoom_at(origin, p, if alt { 0.5 } else { 2.0 });
                }
            }
            Tool::Hand => {}
        }
    }
    // Right-click selects the node under the pointer, then opens its Copy menu. Over empty
    // canvas nothing is selected, so no menu opens. Any tool, as the spec says.
    if response.secondary_clicked() && !marquee::right_click(&response, tab, origin) {
        tab.select(tab.hover);
    }
    marquee::update(ui, &response, tab, origin, tool == Tool::Select && !panning);
    if tab.marquee.menu {
        marquee::context(&response, tab);
    } else if let Some(id) = tab.selection {
        let mut picked = None;
        copy_menu::on_context(&response, tab, id, &mut picked);
        tab.pending_copy = picked.or(tab.pending_copy.take());
    }
    if response.double_clicked()
        && tool == Tool::Select
        && let Some(sel) = tab.selection
    {
        fit_node(tab, rect, sel);
    }

    // Rendering: cached tiles now, missing ones requested (tiles_view.rs). After a jump the
    // drawing eases to the new view; input above already uses it.
    let target = tab.view;
    tab.view = shown_view(ui, tab, direct);
    let painter = ui.painter_at(rect);
    let bg = page_background(&tab.doc, tab.page);
    painter.rect_filled(rect, radius, to_color32(bg));
    tiles.show(&painter, rect, radius, tab, workers, ui.pixels_per_point());

    // Overlays.
    crate::labels::paint(
        &painter,
        &crate::labels::labels(&tab.doc, tab.page, tab.view, origin, rect),
    );
    find_highlights(&painter, tab, origin);
    if let Some(h) = tab.hover.filter(|h| Some(*h) != tab.selection) {
        outline(
            &painter,
            tab,
            origin,
            h,
            Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.8)),
        );
    }
    if let Some(sel) = tab.selection {
        outline(&painter, tab, origin, sel, Stroke::new(1.5, theme::ACCENT));
    }
    marquee::paint(&painter, tab, origin);
    tab.view = target;

    let cursor = match tool {
        _ if panning => egui::CursorIcon::Grab,
        Tool::Zoom => egui::CursorIcon::ZoomIn,
        Tool::Hand => egui::CursorIcon::Grab,
        Tool::Select => egui::CursorIcon::Default,
    };
    if response.hovered() {
        ui.ctx().set_cursor_icon(if response.dragged() && panning {
            egui::CursorIcon::Grabbing
        } else {
            cursor
        });
    }
    cursor_page
}

/// The screen corners of a node's frame on the tab's page, following its rotation.
fn corners(tab: &Tab, origin: Pos2, id: Guid) -> Option<[Pos2; 4]> {
    let node = tab.doc.node(id)?;
    if tab.doc.page_of(id) != Some(tab.page) {
        return None;
    }
    let tf = absolute_transform(&tab.doc, id);
    let (w, h) = geom::size(&node.data);
    Some([(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| {
        let p = tf * Point::new(x, y);
        tab.view
            .to_screen(origin, Pos2::new(p.x as f32, p.y as f32))
    }))
}

/// The find bar's matches: a soft highlight on each, a stronger one on the current match.
fn find_highlights(painter: &egui::Painter, tab: &Tab, origin: Pos2) {
    for (i, hit) in tab.find_hits.iter().enumerate() {
        let c = match hit {
            crate::find::Hit::Node(id) => corners(tab, origin, *id),
            crate::find::Hit::Inside(page) => {
                let r = Rect::from_min_max(
                    tab.view.to_screen(origin, page.min),
                    tab.view.to_screen(origin, page.max),
                );
                Some([
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                ])
            }
        };
        let Some(c) = c else {
            continue;
        };
        let current = tab.find_current == Some(i);
        let (fill, stroke) = if current {
            (
                crate::find::CURRENT.gamma_multiply(0.35),
                Stroke::new(2.0, crate::find::CURRENT),
            )
        } else {
            (
                crate::find::HIT.gamma_multiply(0.3),
                Stroke::new(1.0, crate::find::HIT),
            )
        };
        painter.add(egui::Shape::convex_polygon(c.to_vec(), fill, stroke));
    }
}

/// Outline of a node's frame, following its rotation.
fn outline(painter: &egui::Painter, tab: &Tab, origin: Pos2, id: Guid, stroke: Stroke) {
    let Some(corners) = corners(tab, origin, id) else {
        return;
    };
    let axis_aligned =
        (corners[0].y - corners[1].y).abs() < 0.01 && (corners[0].x - corners[3].x).abs() < 0.01;
    if axis_aligned {
        painter.rect_stroke(
            Rect::from_two_pos(corners[0], corners[2]),
            0.0,
            stroke,
            StrokeKind::Middle,
        );
    } else {
        painter.add(egui::Shape::closed_line(corners.to_vec(), stroke));
    }
}

/// How long a jump of the view (fit, zoom buttons and keys, the Zoom tool) eases; pans and
/// zooms the pointer drives follow it exactly.
const VIEW_EASE: f64 = 0.28;

/// A view on its way from `from` to `to`, started at `start` (egui time) on `page`.
#[derive(Clone, Copy, Debug)]
struct ViewMove {
    from: View,
    to: View,
    start: f64,
    page: Guid,
}

/// The view between `from` and `to` at `t` (0..1). The zoom changes geometrically around the
/// page point both views show at the same place (the Zoom tool's click stays put); views at
/// the same zoom pan in a straight line.
pub(crate) fn view_between(from: View, to: View, t: f32) -> View {
    if t >= 1.0 {
        return to;
    }
    let zoom = (from.zoom.ln() + (to.zoom.ln() - from.zoom.ln()) * t).exp();
    if (to.zoom / from.zoom - 1.0).abs() < 1e-3 {
        return View {
            zoom,
            pan: from.pan + (to.pan - from.pan) * t,
        };
    }
    // pan_a + q·za = pan_b + q·zb: the page point that does not move.
    let fixed = (from.pan - to.pan) / (to.zoom - from.zoom);
    let screen = from.pan + fixed * from.zoom;
    View {
        zoom,
        pan: screen - fixed * zoom,
    }
}

/// The view to draw this frame: the tab's, or one on the way to it after a jump. `direct`
/// means the pointer moved the view this frame, which is never eased.
fn shown_view(ui: &egui::Ui, tab: &Tab, direct: bool) -> View {
    let ctx = ui.ctx();
    let id = egui::Id::new(("canvas-view", tab.id));
    let now = ctx.input(|i| i.time);
    let target = tab.view;
    let settled = ViewMove {
        from: target,
        to: target,
        start: now - VIEW_EASE,
        page: tab.page,
    };
    let at = |m: &ViewMove| {
        let t = ((now - m.start) / VIEW_EASE).clamp(0.0, 1.0) as f32;
        view_between(m.from, m.to, egui::emath::easing::cubic_out(t))
    };
    let mut state = ctx.data(|d| d.get_temp::<ViewMove>(id)).unwrap_or(settled);
    if direct || crate::anim::reduced(ctx) || state.page != tab.page {
        state = settled;
    } else if state.to != target {
        state = ViewMove {
            from: at(&state),
            to: target,
            start: now,
            page: tab.page,
        };
    }
    ctx.data_mut(|d| d.insert_temp(id, state));
    let shown = at(&state);
    if shown != target {
        ctx.request_repaint();
    }
    shown
}

pub fn to_color32(c: AlphaColor<figstudio_render::color::Srgb>) -> Color32 {
    let [r, g, b, a] = c.components;
    Color32::from_rgba_unmultiplied(
        (r * 255.0) as u8,
        (g * 255.0) as u8,
        (b * 255.0) as u8,
        (a * 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zoom_jump_keeps_its_anchor_still_and_ends_on_the_target() {
        let from = View {
            zoom: 1.0,
            pan: Vec2::new(10.0, 20.0),
        };
        let mut to = from;
        let anchor = Pos2::new(300.0, 200.0);
        to.zoom_at(Pos2::ZERO, anchor, 4.0);
        let page = from.to_page(Pos2::ZERO, anchor);
        for t in [0.0, 0.25, 0.5, 0.9] {
            let v = view_between(from, to, t);
            let at = v.to_screen(Pos2::ZERO, page);
            assert!((at - anchor).length() < 0.01, "t {t}: {at:?}");
            assert!(
                v.zoom >= from.zoom && v.zoom <= to.zoom,
                "t {t}: {}",
                v.zoom
            );
        }
        assert_eq!(view_between(from, to, 1.0), to);
        assert!(
            (view_between(from, to, 0.5).zoom - 2.0).abs() < 1e-4,
            "geometric"
        );
        let mut moved = from;
        moved.pan += Vec2::new(100.0, 0.0);
        assert_eq!(view_between(from, moved, 0.5).pan, Vec2::new(60.0, 20.0));
    }
}
