//! Interactive canvas gizmos: 8-handle resize, direct layer drag, and shape/text creation tools.

use egui::{Color32, CornerRadius, CursorIcon, Pos2, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use figstudio_fig::edit::EditOp;
use figstudio_fig::{Document, Guid};
use figstudio_render::kurbo::{Affine, Point};
use figstudio_render::{absolute_transform, geom};

use crate::canvas::Tool;
use crate::tab::{Tab, View};
use crate::theme;

use super::{apply_tab_ops_ui, local_xywh};

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

    fn local_uv(self) -> (f64, f64) {
        match self {
            Self::NW => (0.0, 0.0),
            Self::N => (0.5, 0.0),
            Self::NE => (1.0, 0.0),
            Self::E => (1.0, 0.5),
            Self::SE => (1.0, 1.0),
            Self::S => (0.5, 1.0),
            Self::SW => (0.0, 1.0),
            Self::W => (0.0, 0.5),
        }
    }

    fn local_dir(self) -> (f64, f64) {
        match self {
            Self::NW => (-1.0, -1.0),
            Self::N => (0.0, -1.0),
            Self::NE => (1.0, -1.0),
            Self::E => (1.0, 0.0),
            Self::SE => (1.0, 1.0),
            Self::S => (0.0, 1.0),
            Self::SW => (-1.0, 1.0),
            Self::W => (-1.0, 0.0),
        }
    }

    /// Resize cursor rotated into screen space for a node with absolute transform `tf`.
    pub(super) fn cursor_for(self, tf: Affine) -> CursorIcon {
        let (ux, uy) = self.local_dir();
        let d = tf * Point::new(ux, uy) - tf * Point::ZERO;
        if d.hypot() < 1e-9 {
            return self.cursor();
        }
        let angle = d.y.atan2(d.x).rem_euclid(std::f64::consts::PI);
        let octant = (angle / std::f64::consts::FRAC_PI_4).round() as i32 % 4;
        match octant {
            0 => CursorIcon::ResizeHorizontal,
            1 => CursorIcon::ResizeNwSe,
            2 => CursorIcon::ResizeVertical,
            _ => CursorIcon::ResizeNeSw,
        }
    }
}

/// Active interactive gesture on the canvas.
#[derive(Clone, Debug)]
pub enum CanvasDrag {
    Move {
        target: Guid,
        start_pointer_page: Pos2,
        orig_x: f64,
        orig_y: f64,
        moved: bool,
    },
    Resize {
        target: Guid,
        handle: Handle,
        start_pointer_page: Pos2,
        orig_x: f64,
        orig_y: f64,
        orig_w: f64,
        orig_h: f64,
        mutated: bool,
        inside_node: bool,
    },
    Create {
        tool: Tool,
        start_page: Pos2,
        cur_page: Pos2,
    },
}

/// Per-frame gizmo arbitration outcome returned to `canvas::show` before click, text, and marquee handling.
#[derive(Clone, Copy, Debug, Default)]
pub struct GizmoFrame {
    pub cursor: Option<CursorIcon>,
    /// True when gizmos owned the click (creation tool, resize handle, or completed move drag).
    pub consumed_click: bool,
    /// True when canvas marquee dragging must not run this frame.
    pub blocks_marquee: bool,
    /// True when canvas text-selection input must not run this frame.
    pub blocks_text: bool,
}

struct SelectionHandles {
    tf: Affine,
    w: f64,
    h: f64,
    points: [Pos2; 8],
}

impl SelectionHandles {
    fn for_node(canvas_rect: Rect, tab: &Tab, node_id: Guid) -> Option<Self> {
        let node = tab.doc.node(node_id)?;
        if tab.doc.page_of(node_id) != Some(tab.page) {
            return None;
        }
        let tf = absolute_transform(&tab.doc, node_id);
        let (w, h) = geom::size(&node.data);
        let points = Handle::ALL.map(|handle| {
            let (u, v) = handle.local_uv();
            let p = tf * Point::new(u * w, v * h);
            tab.view
                .to_screen(canvas_rect.min, pos2(p.x as f32, p.y as f32))
        });
        Some(Self { tf, w, h, points })
    }

    fn contains_screen(&self, canvas_rect: Rect, view: View, pos: Pos2) -> bool {
        if self.tf.determinant().abs() > 1e-12 {
            let page = view.to_page(canvas_rect.min, pos);
            let local = self.tf.inverse() * Point::new(f64::from(page.x), f64::from(page.y));
            let axis_pad = |axis: Point, size: f64| -> f64 {
                if size >= 1.0 {
                    return 0.0;
                }
                let scale = (self.tf * axis - self.tf * Point::ZERO).hypot();
                if scale > 1e-9 {
                    4.0 / (f64::from(view.zoom) * scale)
                } else {
                    0.0
                }
            };
            let px = axis_pad(Point::new(1.0, 0.0), self.w);
            let py = axis_pad(Point::new(0.0, 1.0), self.h);
            return local.x >= -px
                && local.x <= self.w + px
                && local.y >= -py
                && local.y <= self.h + py;
        }
        let mut bounds = Rect::NOTHING;
        for p in self.points {
            bounds.extend_with(p);
        }
        bounds.contains(pos)
    }
}

/// Finishes any active canvas drag and seals coalesced undo history so the next drag
/// begins a fresh undo entry.
pub fn end_edit_drag(tab: &mut Tab) {
    if tab.edit_drag.take().is_some() {
        tab.history.seal_coalesce();
    }
}

/// Cancels an active canvas drag on Escape, reverting any in-progress Move or Resize
/// mutation while preserving the current selection. Returns whether a drag was active.
pub fn cancel_edit_drag(tab: &mut Tab) -> bool {
    let Some(drag) = tab.edit_drag.take() else {
        return false;
    };
    let mutated = matches!(
        drag,
        CanvasDrag::Move { moved: true, .. } | CanvasDrag::Resize { mutated: true, .. }
    );
    if mutated {
        let res = super::undo_tab(tab);
        let _ = super::record_tab_error(tab, res);
        tab.history.seal_coalesce();
    }
    true
}

/// Handles canvas creation tools, 8-handle resize dragging, and direct selection dragging
/// ahead of `canvas::show`'s click, text-selection, and marquee handlers.
pub fn canvas_gizmos_input(
    ui: &mut Ui,
    response: &egui::Response,
    rect: Rect,
    tab: &mut Tab,
    tool: &mut Tool,
    panning: bool,
) -> GizmoFrame {
    if panning || tab.preview.is_some() {
        end_edit_drag(tab);
        return GizmoFrame::default();
    }
    if tab.edit_drag.is_some() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancel_edit_drag(tab);
        return GizmoFrame {
            cursor: None,
            consumed_click: true,
            blocks_marquee: true,
            blocks_text: true,
        };
    }

    let pointer_pos = ui.input(|i| i.pointer.interact_pos());
    let primary_pressed = ui.input(|i| i.pointer.button_pressed(egui::PointerButton::Primary));
    let primary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let primary_released = ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary));
    let canvas_hovered = response.hovered();
    let canvas_owns_press =
        response.is_pointer_button_down_on() || response.clicked_by(egui::PointerButton::Primary);
    let canvas_owns_drag =
        response.is_pointer_button_down_on() || response.dragged_by(egui::PointerButton::Primary);

    // 1. Creation tools (`Frame`, `Section`, `Rectangle`, `Ellipse`, `Line`, `Text`).
    if tool.is_creation() {
        if !matches!(tab.edit_drag, Some(CanvasDrag::Create { .. })) {
            end_edit_drag(tab);
        }
        if primary_pressed
            && canvas_owns_press
            && let Some(pos) = pointer_pos
            && rect.contains(pos)
        {
            tab.marquee.clear();
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
                if let Some(ids) = apply_tab_ops_ui(tab, &[op], Some("Create layer"), None)
                    && let Some(&new_id) = ids.first()
                {
                    tab.select(Some(new_id));
                }
                *tool = Tool::Select;
            } else if !primary_down || !canvas_owns_drag {
                tab.edit_drag = None;
            }
        }
        return GizmoFrame {
            cursor: canvas_hovered.then_some(CursorIcon::Crosshair),
            consumed_click: true,
            blocks_marquee: true,
            blocks_text: true,
        };
    }

    if matches!(tab.edit_drag, Some(CanvasDrag::Create { .. })) {
        end_edit_drag(tab);
    }

    if *tool != Tool::Select || tab.text_select.is_some() || tab.inner.is_some() {
        end_edit_drag(tab);
        return GizmoFrame::default();
    }

    let click_dist = f64::from(ui.ctx().options(|o| o.input_options.max_click_dist));

    // 2. Continue an in-progress Resize or Move drag without walking hit_chain.
    if let Some(mut drag) = tab.edit_drag.clone() {
        let target = match drag {
            CanvasDrag::Move { target, .. } | CanvasDrag::Resize { target, .. } => target,
            CanvasDrag::Create { .. } => {
                end_edit_drag(tab);
                return GizmoFrame::default();
            }
        };
        if tab.selection != Some(target) || tab.doc.node(target).is_none() {
            end_edit_drag(tab);
            return GizmoFrame::default();
        }

        let mut cursor = None;
        let active_or_releasing = (primary_down && canvas_owns_drag) || primary_released;
        if active_or_releasing
            && let Some(pos) = pointer_pos
            && !step_edit_drag(
                tab,
                &mut drag,
                tab.view.to_page(rect.min, pos),
                click_dist,
                &mut cursor,
            )
        {
            end_edit_drag(tab);
            return GizmoFrame {
                cursor: None,
                consumed_click: true,
                blocks_marquee: true,
                blocks_text: true,
            };
        }

        let consumed = match drag {
            CanvasDrag::Resize {
                mutated,
                inside_node,
                ..
            } => mutated || !inside_node,
            CanvasDrag::Move { moved, .. } => moved,
            CanvasDrag::Create { .. } => true,
        };
        if primary_released || !primary_down || !canvas_owns_drag {
            end_edit_drag(tab);
            return GizmoFrame {
                cursor: None,
                consumed_click: consumed,
                blocks_marquee: true,
                blocks_text: consumed,
            };
        }
        tab.edit_drag = Some(drag);
        return GizmoFrame {
            cursor,
            consumed_click: consumed,
            blocks_marquee: true,
            blocks_text: consumed,
        };
    }

    // 3. Detect oriented resize-handle hover/press and direct body move press on the selected node.
    if !canvas_hovered || tab.marquee.is_dragging() {
        return GizmoFrame::default();
    }
    let Some(sel) = tab.selection else {
        return GizmoFrame::default();
    };
    let Some(node) = tab.doc.node(sel) else {
        return GizmoFrame::default();
    };
    if matches!(node.kind.as_str(), "DOCUMENT" | "CANVAS") || node.data.flag("locked") == Some(true)
    {
        return GizmoFrame::default();
    }
    let Some(handles) = SelectionHandles::for_node(rect, tab, sel) else {
        return GizmoFrame::default();
    };
    let Some(pos) = pointer_pos else {
        return GizmoFrame::default();
    };
    if !rect.contains(pos) {
        return GizmoFrame::default();
    }

    let inside_node = if figstudio_render::is_hollow_layer(&node.kind, &node.data) {
        let page = tab.view.to_page(rect.min, pos);
        figstudio_render::hits_layer(
            &tab.doc,
            sel,
            handles.tf,
            Point::new(f64::from(page.x), f64::from(page.y)),
            crate::canvas::hit_options(tab.view.zoom),
        )
    } else {
        handles.contains_screen(rect, tab.view, pos)
    };
    for (h, hp) in Handle::ALL.into_iter().zip(handles.points) {
        let hit = Rect::from_center_size(hp, vec2(10.0, 10.0));
        if hit.contains(pos) {
            let cursor = Some(h.cursor_for(handles.tf));
            if primary_pressed && canvas_owns_press {
                if !inside_node {
                    tab.marquee.clear();
                }
                if !primary_released {
                    let (ox, oy, ow, oh) = local_xywh(&node.data);
                    tab.edit_drag = Some(CanvasDrag::Resize {
                        target: sel,
                        handle: h,
                        start_pointer_page: tab.view.to_page(rect.min, pos),
                        orig_x: ox,
                        orig_y: oy,
                        orig_w: ow,
                        orig_h: oh,
                        mutated: false,
                        inside_node,
                    });
                }
                return GizmoFrame {
                    cursor,
                    consumed_click: !inside_node,
                    blocks_marquee: true,
                    blocks_text: !inside_node,
                };
            }
            return GizmoFrame {
                cursor,
                ..GizmoFrame::default()
            };
        }
    }

    if primary_pressed && canvas_owns_press && inside_node && !primary_released {
        let (ox, oy, _, _) = local_xywh(&node.data);
        tab.edit_drag = Some(CanvasDrag::Move {
            target: sel,
            start_pointer_page: tab.view.to_page(rect.min, pos),
            orig_x: ox,
            orig_y: oy,
            moved: false,
        });
        return GizmoFrame {
            cursor: None,
            consumed_click: false,
            blocks_marquee: true,
            blocks_text: false,
        };
    }

    GizmoFrame::default()
}

/// Applies one pointer update to an active Move or Resize drag. Returns `false` when an edit
/// operation fails and the caller must abort the drag.
fn step_edit_drag(
    tab: &mut Tab,
    drag: &mut CanvasDrag,
    cur_page: Pos2,
    click_dist: f64,
    cursor: &mut Option<CursorIcon>,
) -> bool {
    let start = match drag {
        CanvasDrag::Move {
            start_pointer_page, ..
        }
        | CanvasDrag::Resize {
            start_pointer_page, ..
        } => *start_pointer_page,
        CanvasDrag::Create { .. } => return true,
    };
    let dx = f64::from(cur_page.x - start.x);
    let dy = f64::from(cur_page.y - start.y);
    let screen_dist = dx.hypot(dy) * f64::from(tab.view.zoom);

    match drag {
        CanvasDrag::Move {
            target,
            orig_x,
            orig_y,
            moved,
            ..
        } => {
            if !*moved && screen_dist <= click_dist {
                return true;
            }
            tab.marquee.clear();
            let key = format!("gui:move:{target}");
            let (x, y) = moved_to(&tab.doc, *target, *orig_x, *orig_y, dx, dy);
            let ok = apply_tab_ops_ui(
                tab,
                &[EditOp::Move {
                    target: *target,
                    x: x.round(),
                    y: y.round(),
                    relative: false,
                }],
                Some("Move layer"),
                Some(&key),
            )
            .is_some();
            if ok {
                *moved = true;
                *cursor = Some(CursorIcon::Grabbing);
            }
            ok
        }
        CanvasDrag::Resize {
            target,
            handle,
            orig_x,
            orig_y,
            orig_w,
            orig_h,
            mutated,
            inside_node,
            ..
        } => {
            *cursor = Some(handle.cursor_for(absolute_transform(&tab.doc, *target)));
            let threshold = if *inside_node { click_dist } else { 2.0 };
            if !*mutated && screen_dist <= threshold {
                return true;
            }
            tab.marquee.clear();
            let (nx, ny, nw, nh) = resized_with(
                &tab.doc,
                *target,
                *handle,
                (*orig_x, *orig_y, *orig_w, *orig_h),
                dx,
                dy,
                true,
            );
            let changed = *mutated
                || (nx - *orig_x).abs() > 1e-6
                || (ny - *orig_y).abs() > 1e-6
                || (nw - *orig_w).abs() > 1e-6
                || (nh - *orig_h).abs() > 1e-6;
            if !changed {
                return true;
            }
            let key = format!("gui:resize:{target}");
            let ok = apply_tab_ops_ui(
                tab,
                &[
                    EditOp::Move {
                        target: *target,
                        x: nx,
                        y: ny,
                        relative: false,
                    },
                    EditOp::Resize {
                        target: *target,
                        width: nw,
                        height: nh,
                    },
                ],
                Some("Resize layer"),
                Some(&key),
            )
            .is_some();
            if ok {
                *mutated = true;
            }
            ok
        }
        CanvasDrag::Create { .. } => true,
    }
}

/// Paints the creation preview rectangle and oriented 8-handle selection gizmo on top of outlines.
pub fn canvas_gizmos_paint(painter: &egui::Painter, rect: Rect, tab: &Tab, tool: Tool) {
    if tab.preview.is_some() {
        return;
    }
    if let Some(CanvasDrag::Create {
        start_page,
        cur_page,
        ..
    }) = tab.edit_drag
    {
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
        return;
    }
    if tool != Tool::Select || tab.text_select.is_some() || tab.inner.is_some() {
        return;
    }
    let Some(sel) = tab.selection else {
        return;
    };
    let Some(node) = tab.doc.node(sel) else {
        return;
    };
    if matches!(node.kind.as_str(), "DOCUMENT" | "CANVAS") || node.data.flag("locked") == Some(true)
    {
        return;
    }
    let Some(handles) = SelectionHandles::for_node(rect, tab, sel) else {
        return;
    };
    for c in handles.points {
        let hr = Rect::from_center_size(c, vec2(7.0, 7.0));
        painter.rect_filled(hr, CornerRadius::same(1), Color32::WHITE);
        painter.rect_stroke(
            hr,
            CornerRadius::same(1),
            Stroke::new(1.5, theme::ACCENT),
            StrokeKind::Inside,
        );
    }
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
/// moves the translation along those axes so the opposite edge stays put.
#[cfg(test)]
pub(super) fn resized_to(
    doc: &Document,
    target: Guid,
    handle: Handle,
    orig: (f64, f64, f64, f64),
    dx: f64,
    dy: f64,
) -> (f64, f64, f64, f64) {
    resized_with(doc, target, handle, orig, dx, dy, false)
}

fn resized_with(
    doc: &Document,
    target: Guid,
    handle: Handle,
    orig: (f64, f64, f64, f64),
    dx: f64,
    dy: f64,
    snap_size: bool,
) -> (f64, f64, f64, f64) {
    let (orig_x, orig_y, orig_w, orig_h) = orig;
    let (lx, ly) = page_vector_in(doc, Some(target), dx, dy);
    let snap = |v: f64| if snap_size { v.round() } else { v };
    let (mut nw, mut nh) = (orig_w, orig_h);
    let (mut sx, mut sy) = (0.0, 0.0);
    match handle {
        Handle::E | Handle::NE | Handle::SE => nw = snap(orig_w + lx).max(1.0),
        Handle::W | Handle::NW | Handle::SW => {
            nw = snap(orig_w - lx).max(1.0);
            sx = orig_w - nw;
        }
        _ => {}
    }
    match handle {
        Handle::S | Handle::SE | Handle::SW => nh = snap(orig_h + ly).max(1.0),
        Handle::N | Handle::NW | Handle::NE => {
            nh = snap(orig_h - ly).max(1.0);
            sy = orig_h - nh;
        }
        _ => {}
    }
    // The shift along the layer's axes, in its parent's coordinates (the layer's own
    // rotation, skew, and scale, without its translation).
    let local = doc
        .node(target)
        .map_or(Affine::IDENTITY, |n| geom::affine(n.data.get("transform")));
    let shift = local * Point::new(sx, sy) - local * Point::ZERO;
    (orig_x + shift.x, orig_y + shift.y, nw, nh)
}

#[cfg(test)]
#[path = "canvas_gizmos_tests.rs"]
mod tests;
