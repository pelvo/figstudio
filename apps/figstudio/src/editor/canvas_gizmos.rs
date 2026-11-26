//! Interactive canvas gizmos: 8-handle resize, direct layer drag, and shape/text creation tools.

use egui::{CursorIcon, Pos2, Rect, Stroke, StrokeKind, Ui, pos2};
use figstudio_fig::edit::EditOp;
use figstudio_fig::{Document, Guid};
use figstudio_render::absolute_transform;
use figstudio_render::kurbo::Point;

use crate::canvas::Tool;
use crate::tab::Tab;
use crate::theme;

use super::apply_tab_ops;

/// Which of the 8 resize handles around a selected layer is being dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    NW,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
}

impl Handle {
    pub const ALL: [Self; 8] = [
        Self::NW,
        Self::N,
        Self::NE,
        Self::E,
        Self::SE,
        Self::S,
        Self::SW,
        Self::W,
    ];

    pub fn cursor(self) -> CursorIcon {
        match self {
            Self::NW | Self::SE => CursorIcon::ResizeNwSe,
            Self::NE | Self::SW => CursorIcon::ResizeNeSw,
            Self::N | Self::S => CursorIcon::ResizeVertical,
            Self::E | Self::W => CursorIcon::ResizeHorizontal,
        }
    }
}

/// Active interactive gesture on the canvas.
#[derive(Clone, Debug)]
pub enum CanvasDrag {
    /// Moving the selection (or, once it moves, the unselected layer `select` pressed).
    Move {
        layers: Vec<super::canvas_select::Start>,
        start_pointer_page: Pos2,
        moved: bool,
        /// The undo step's coalesce key (an Alt-drag's copies share it).
        key: String,
        select: Option<Guid>,
    },
    Resize {
        layers: Vec<super::canvas_select::Start>,
        handle: Handle,
        start_pointer_page: Pos2,
    },
    /// Turning the selection about `centre` (page space).
    Rotate {
        layers: Vec<super::canvas_select::Start>,
        start_pointer_page: Pos2,
        centre: figstudio_render::kurbo::Point,
    },
    Create {
        tool: Tool,
        start_page: Pos2,
        cur_page: Pos2,
    },
}

/// Handles canvas creation tools, 8-handle resize dragging, and direct selection dragging,
/// returning `Some(cursor)` when an editor gizmo owns pointer interaction this frame.
pub fn canvas_gizmos_and_input(
    ui: &mut Ui,
    painter: &egui::Painter,
    rect: Rect,
    tab: &mut Tab,
    tool: &mut Tool,
    panning: bool,
    snapping: bool,
) -> Option<CursorIcon> {
    if panning || tab.preview.is_some() {
        return None;
    }
    // Another tool picked mid-path: the path so far is kept.
    if *tool != Tool::Pen && tab.pen.is_some() {
        super::vector_edit::finish_pen(tab, false);
    }

    let pointer_pos = ui.input(|i| i.pointer.interact_pos());
    let primary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
    let primary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let primary_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));

    let pointer = super::canvas_select::Pointer {
        pos: pointer_pos,
        pressed: primary_pressed,
        down: primary_down,
        released: primary_released,
        modifiers: ui.input(|i| i.modifiers),
        double: ui.input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
        }),
    };
    if *tool == Tool::Pen {
        let cursor = super::vector_edit::pen(painter, rect, tab, &pointer);
        if tab.reset_tool_to_select {
            *tool = Tool::Select;
        }
        return cursor;
    }
    // 1. Creation tools (`Frame`, `Section`, `Rectangle`, `Ellipse`, `Line`, `Text`).
    if tool.is_creation() {
        if primary_pressed
            && let Some(pos) = pointer_pos
            && rect.contains(pos)
        {
            let p = tab.view.to_page(rect.min, pos);
            tab.edit_drag = Some(CanvasDrag::Create {
                tool: *tool,
                start_page: p,
                cur_page: p,
            });
        }
        if let Some(CanvasDrag::Create {
            tool: create_tool,
            start_page,
            ref mut cur_page,
        }) = tab.edit_drag
        {
            if let Some(pos) = pointer_pos {
                *cur_page = tab.view.to_page(rect.min, pos);
            }
            let min_p = pos2(start_page.x.min(cur_page.x), start_page.y.min(cur_page.y));
            let max_p = pos2(start_page.x.max(cur_page.x), start_page.y.max(cur_page.y));
            let s0 = tab.view.to_screen(rect.min, min_p);
            let s1 = tab.view.to_screen(rect.min, max_p);
            painter.rect_stroke(
                Rect::from_min_max(s0, s1),
                0.0,
                Stroke::new(1.5, theme::ACCENT),
                StrokeKind::Outside,
            );
            if primary_released {
                let end_p = *cur_page;
                tab.edit_drag = None;
                let w =
                    f64::from((end_p.x - start_page.x).abs()).max(if create_tool == Tool::Text {
                        120.0
                    } else {
                        80.0
                    });
                let h = f64::from((end_p.y - start_page.y).abs())
                    .max(if create_tool == Tool::Line { 0.0 } else { 60.0 });
                let x = f64::from(start_page.x.min(end_p.x));
                let y = f64::from(start_page.y.min(end_p.y));
                let op = match create_tool {
                    Tool::Text => EditOp::CreateText {
                        id: None,
                        parent: tab.page,
                        x,
                        y,
                        characters: "Text".into(),
                        font_family: "Inter".into(),
                        font_style: "Regular".into(),
                        font_size: 20.0,
                        fill_hex: "#111827".into(),
                    },
                    other => EditOp::CreateShape {
                        id: None,
                        kind: other.shape_kind().unwrap_or("FRAME").into(),
                        parent: tab.page,
                        name: None,
                        x,
                        y,
                        width: w,
                        height: h,
                        corner_radius: 0.0,
                        fill_hex: None,
                        count: None,
                        star_inner_scale: None,
                    },
                };
                if let Ok(ids) = apply_tab_ops(tab, &[op], Some("Create layer"), None)
                    && let Some(&new_id) = ids.first()
                {
                    tab.select(Some(new_id));
                }
                *tool = Tool::Select;
            }
        }
        return Some(CursorIcon::Crosshair);
    }

    if *tool != Tool::Select || tab.text_select.is_some() {
        return None;
    }

    super::canvas_select::gestures(painter, rect, tab, &pointer, snapping)
}

/// The vector `(dx, dy)` in page units, in `node`'s own coordinates (`None`: inverse of the
/// parent, so the node's own transform is left out). A degenerate (zero-scale) transform
/// leaves it as is.
fn page_vector_in(doc: &Document, node: Option<Guid>, dx: f64, dy: f64) -> (f64, f64) {
    let Some(node) = node else {
        return (dx, dy);
    };
    let abs = absolute_transform(doc, node);
    if abs.determinant().abs() < 1e-12 {
        return (dx, dy);
    }
    let inv = abs.inverse();
    let v = inv * Point::new(dx, dy) - inv * Point::ZERO;
    (v.x, v.y)
}

/// Where a Move drag by page vector `(dx, dy)` puts `target`'s parent-local translation,
/// starting at `(orig_x, orig_y)`: the pointer delta goes through the parent's transform, so
/// the layer follows the pointer under rotated or scaled ancestors.
pub(super) fn moved_to(
    doc: &Document,
    target: Guid,
    orig_x: f64,
    orig_y: f64,
    dx: f64,
    dy: f64,
) -> (f64, f64) {
    let parent = doc.node(target).and_then(|n| n.parent);
    let (px, py) = page_vector_in(doc, parent, dx, dy);
    (orig_x + px, orig_y + py)
}

/// The translation and size after dragging `handle` by page vector `(dx, dy)`, from `orig`
/// `(x, y, w, h)`. The delta is measured along the layer's own axes, and a left or top handle
/// moves the translation along those axes so the opposite edge stays put; `aspect` keeps the
/// width-to-height ratio.
pub(super) fn resized_to(
    doc: &Document,
    target: Guid,
    handle: Handle,
    orig: (f64, f64, f64, f64),
    dx: f64,
    dy: f64,
    aspect: bool,
) -> (f64, f64, f64, f64) {
    let (orig_x, orig_y, orig_w, orig_h) = orig;
    let (lx, ly) = page_vector_in(doc, Some(target), dx, dy);
    // The layer's box in its own axes, resized like a page box, then read back.
    let b = super::canvas_select::box_resized(
        figstudio_render::kurbo::Rect::new(0.0, 0.0, orig_w, orig_h),
        handle,
        lx,
        ly,
        aspect,
    );
    let (nw, nh) = (b.width(), b.height());
    let (sx, sy) = (b.x0, b.y0);
    // The shift along the layer's axes, in its parent's coordinates (the layer's own
    // rotation and scale, without its translation).
    let parent = doc.node(target).and_then(|n| n.parent);
    let own = absolute_transform(doc, target);
    let local = match parent.map(|p| absolute_transform(doc, p)) {
        Some(p) if p.determinant().abs() > 1e-12 => p.inverse() * own,
        _ => own,
    };
    let shift = local * Point::new(sx, sy) - local * Point::ZERO;
    (orig_x + shift.x, orig_y + shift.y, nw, nh)
}

#[cfg(test)]
#[path = "canvas_gizmos_tests.rs"]
mod tests;
