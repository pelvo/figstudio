//! The canvas: pan / zoom input, background rendering, selection and hover outlines.

use egui::{Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use figstudio_fig::Guid;
use figstudio_render::kurbo::{Affine, Point};
use figstudio_render::{
    Viewport, absolute_transform, color::AlphaColor, content_bounds, geom, hit_test,
    page_background,
};

use crate::tab::{Shown, Tab, View};
use crate::theme;
use crate::worker::{RenderJob, Workers};

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

/// Draws the canvas and handles its input. Returns the page-space cursor position.
pub fn show(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    workers: &Workers,
    tool: Tool,
    space_held: bool,
) -> Option<Pos2> {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let origin = rect.min;
    tab.canvas_rect = Some(rect);
    if tab.fit_pending && rect.width() > 10.0 {
        fit(tab, rect);
        tab.fit_pending = false;
    }

    // Input: pan.
    let panning = tool == Tool::Hand || space_held || ui.input(|i| i.pointer.middle_down());
    if response.dragged() && (panning || response.dragged_by(egui::PointerButton::Middle)) {
        tab.view.pan += response.drag_delta();
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
    if response.double_clicked()
        && tool == Tool::Select
        && let Some(sel) = tab.selection
        && let Some(b) = figstudio_render::node_bounds(&tab.doc, sel)
    {
        fit_rect(
            tab,
            rect,
            Rect::from_min_max(
                Pos2::new(b.x0 as f32, b.y0 as f32),
                Pos2::new(b.x1 as f32, b.y1 as f32),
            ),
        );
    }

    // Rendering: request a frame when the view changed, show the newest one we have.
    let ppp = ui.pixels_per_point();
    let px = (rect.size() * ppp).round();
    let key = format!(
        "{}|{}|{:.4}|{:.2}|{:.2}|{}x{}|{ppp}",
        tab.page,
        tab.doc.nodes.len(),
        tab.view.zoom,
        tab.view.pan.x,
        tab.view.pan.y,
        px.x,
        px.y
    );
    if tab.requested_key.as_deref() != Some(&key) && px.x >= 1.0 && px.y >= 1.0 {
        tab.generation += 1;
        let zoom = f64::from(tab.view.zoom * ppp);
        let transform = Affine::translate((
            f64::from(tab.view.pan.x * ppp),
            f64::from(tab.view.pan.y * ppp),
        )) * Affine::scale(zoom);
        workers.render(RenderJob {
            tab: tab.id,
            generation: tab.generation,
            doc: tab.doc.clone(),
            root: tab.page,
            viewport: Viewport {
                width: px.x.min(f32::from(u16::MAX)) as u16,
                height: px.y.min(f32::from(u16::MAX)) as u16,
                transform,
                background: Some(page_background(&tab.doc, tab.page)),
            },
            view: tab.view,
            size_points: px / ppp,
        });
        tab.requested_key = Some(key);
    }

    let painter = ui.painter_at(rect);
    let bg = page_background(&tab.doc, tab.page);
    painter.rect_filled(rect, 0.0, to_color32(bg));
    if let Some(shown) = tab.shown.as_ref().filter(|s| s.page == tab.page) {
        // Re-project the frame from the view it was rendered with to the current view.
        let scale = tab.view.zoom / shown.view.zoom;
        let min = origin + tab.view.pan - shown.view.pan * scale;
        let image_rect = Rect::from_min_size(min, shown.size_points * scale);
        painter.image(
            shown.texture.id(),
            image_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    // Overlays.
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

/// Outline of a node's frame, following its rotation.
fn outline(painter: &egui::Painter, tab: &Tab, origin: Pos2, id: Guid, stroke: Stroke) {
    let Some(node) = tab.doc.node(id) else { return };
    if tab.doc.page_of(id) != Some(tab.page) {
        return;
    }
    let tf = absolute_transform(&tab.doc, id);
    let (w, h) = geom::size(&node.data);
    let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| {
        let p = tf * Point::new(x, y);
        tab.view
            .to_screen(origin, Pos2::new(p.x as f32, p.y as f32))
    });
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

pub fn to_color32(c: AlphaColor<figstudio_render::color::Srgb>) -> Color32 {
    let [r, g, b, a] = c.components;
    Color32::from_rgba_unmultiplied(
        (r * 255.0) as u8,
        (g * 255.0) as u8,
        (b * 255.0) as u8,
        (a * 255.0) as u8,
    )
}

/// Stores a finished frame on the tab unless a newer one is already shown.
pub fn accept(ctx: &egui::Context, tab: &mut Tab, done: crate::worker::RenderDone) {
    if done.generation <= tab.shown_generation {
        return;
    }
    tab.shown_generation = done.generation;
    let image = egui::ColorImage::from_rgba_premultiplied(done.size, &done.rgba_premultiplied);
    match &mut tab.shown {
        Some(shown) => {
            shown.texture.set(image, egui::TextureOptions::LINEAR);
            shown.view = done.view;
            shown.size_points = done.size_points;
            shown.page = done.root;
        }
        None => {
            let texture = ctx.load_texture(
                format!("canvas-{}", tab.id),
                image,
                egui::TextureOptions::LINEAR,
            );
            tab.shown = Some(Shown {
                texture,
                view: done.view,
                size_points: done.size_points,
                page: done.root,
            });
        }
    }
    tab.stats = done.stats;
    tab.render_time = done.elapsed;
}
