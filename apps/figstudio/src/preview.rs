//! Prototype Preview on the canvas: one screen alone, fitted; a click on a link follows it.
//! The tiles draw `Tab::root` (the screen), so CPU, GPU and the browser work unchanged.
use egui::{Pos2, Rect, Sense, Stroke, StrokeKind};
use figstudio_render::kurbo::Point;

use crate::canvas::{fit_node, to_color32};
use crate::tab::Tab;
use crate::theme;
use crate::tiles_view::Tiles;
use crate::worker::Workers;

/// How long the links flash after a click that hit none, in seconds.
const FLASH: f64 = 0.8;

/// Draws the Preview screen and handles its input. Returns the page-space cursor position.
pub fn show(
    ui: &mut egui::Ui,
    tab: &mut Tab,
    workers: &mut Workers,
    tiles: &mut Tiles,
    radius: f32,
) -> Option<Pos2> {
    let rect = ui.available_rect_before_wrap();
    let response = ui.allocate_rect(rect, Sense::click_and_drag());
    let origin = rect.min;
    tab.canvas_rect = Some(rect);
    if rect.width() > 10.0
        && let Some(id) = tab.fit_node.take()
    {
        fit_node(tab, rect, id);
    }
    tab.fit_pending = false;

    // A screen taller or wider than the window scrolls; pinch and ⌘-scroll zoom.
    if response.dragged() {
        tab.view.pan += response.drag_delta();
    }
    if response.hovered() {
        let (scroll, zoom, modifiers, pointer) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.zoom_delta(),
                i.modifiers,
                i.pointer.hover_pos(),
            )
        });
        let anchor = pointer.unwrap_or(rect.center());
        if zoom != 1.0 {
            tab.view.zoom_at(origin, anchor, zoom);
        } else if modifiers.command || modifiers.ctrl {
            tab.view.zoom_at(origin, anchor, (scroll.y / 200.0).exp());
        } else {
            tab.view.pan += scroll;
        }
    }
    tab.shown = tab.view;

    // ← or Backspace goes back, R restarts the flow.
    if !ui.ctx().egui_wants_keyboard_input() {
        let (back, restart) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
                    || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace),
                i.consume_key(egui::Modifiers::NONE, egui::Key::R),
            )
        });
        if back {
            tab.preview_go(Some(figstudio_render::prototype::Step::Back));
        } else if restart {
            tab.preview_go(None);
        }
    }

    let cursor = response.hover_pos().map(|p| tab.view.to_page(origin, p));
    let hit = |tab: &Tab| {
        let p = cursor?;
        let screen = &tab.preview.as_ref()?.screen;
        screen
            .hit(Point::new(f64::from(p.x), f64::from(p.y)))
            .cloned()
    };
    let hovered = hit(tab);
    let now = ui.input(|i| i.time);
    if response.clicked() {
        match hovered.as_ref().and_then(|l| l.step) {
            Some(step) => {
                tab.preview_go(Some(step));
            }
            None => {
                if let Some(preview) = tab.preview.as_mut() {
                    preview.missed = Some(now);
                }
            }
        }
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, radius, to_color32(tab.background()));
    let view = tab.view;
    tiles.show(
        &painter,
        (rect, radius),
        tab,
        view,
        workers,
        ui.pixels_per_point(),
    );

    let to_screen = |b: figstudio_render::kurbo::Rect| {
        Rect::from_min_max(
            tab.view
                .to_screen(origin, Pos2::new(b.x0 as f32, b.y0 as f32)),
            tab.view
                .to_screen(origin, Pos2::new(b.x1 as f32, b.y1 as f32)),
        )
    };
    let flash = tab
        .preview
        .as_ref()
        .and_then(|p| p.missed)
        .map_or(0.0, |at| (1.0 - (now - at) / FLASH).max(0.0) as f32);
    if flash > 0.0 {
        ui.ctx().request_repaint();
        if let Some(preview) = &tab.preview {
            for link in preview.screen.links.iter().filter(|l| l.step.is_some()) {
                painter.rect_filled(
                    to_screen(link.bounds),
                    2.0,
                    theme::ACCENT.gamma_multiply(0.25 * flash),
                );
            }
        }
    }
    let playable = hovered.as_ref().is_some_and(|l| l.step.is_some());
    if let Some(link) = hovered.as_ref().filter(|_| playable) {
        painter.rect_stroke(
            to_screen(link.bounds),
            2.0,
            Stroke::new(1.5, theme::ACCENT.gamma_multiply(0.7)),
            StrokeKind::Outside,
        );
    }
    if response.hovered() {
        ui.ctx().set_cursor_icon(if playable {
            egui::CursorIcon::PointingHand
        } else if response.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Default
        });
    }
    cursor
}
