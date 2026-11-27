//! Vector edit mode (double-click a vector or press Enter: drag its points and their handles,
//! Delete removes a point, arrows nudge it, Enter or Escape leaves) and the Pen tool (click
//! adds a point, dragging pulls out its handles, clicking the first point closes the path,
//! Enter or Escape finishes it). Edits are `EditVectorNetwork` / `SetVectorNetwork` batches.

use egui::{Color32, CursorIcon, Key, Pos2, Rect, Stroke, Vec2, pos2, vec2};
use figstudio_fig::Guid;
use figstudio_fig::edit::vector_net::{VectorNetwork, VectorRegion, VectorSegment, VectorVertex};
use figstudio_fig::edit::{EditOp, PaintSlot, VectorEdit as Edit};
use figstudio_render::absolute_transform;
use figstudio_render::kurbo::{Affine, Point};

use super::apply_tab_ops;
use super::canvas_select::Pointer;
use crate::tab::Tab;
use crate::theme;

/// Pick distance for points and handles, in screen points.
const HIT: f32 = 7.0;

/// Editing one vector's points.
#[derive(Clone, Debug, PartialEq)]
pub struct VectorEditing {
    pub node: Guid,
    pub selected: Option<u32>,
    drag: Option<Drag>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    Vertex(u32),
    /// A segment's handle at its start (`true`) or end.
    Handle(u32, bool),
}

/// The Pen tool's path so far: page points and each one's outgoing handle.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PenPath {
    pub points: Vec<(Pos2, Vec2)>,
    pub dragging: bool,
}

/// The vector's network and the transform from network units to the page.
fn network(tab: &Tab, node: Guid) -> Option<(VectorNetwork, Affine)> {
    let n = tab.doc.node(node)?;
    let (net, scale) = figstudio_fig::edit::outline::network(&tab.doc, &n.data)?;
    Some((net, absolute_transform(&tab.doc, node) * scale))
}

/// Whether `node` can be edited point by point.
pub(crate) fn editable(tab: &Tab, node: Guid) -> bool {
    tab.doc.node(node).is_some_and(|n| n.kind == "VECTOR") && network(tab, node).is_some()
}

pub(crate) fn enter(tab: &mut Tab, node: Guid) {
    if editable(tab, node) {
        tab.edit_drag = None;
        tab.vector_edit = Some(VectorEditing {
            node,
            selected: None,
            drag: None,
        });
    }
}

fn apply(tab: &mut Tab, node: Guid, edits: Vec<Edit>, key: bool) {
    let key = key.then(|| format!("vector:{node}"));
    let _ = apply_tab_ops(
        tab,
        &[EditOp::EditVectorNetwork {
            target: node,
            edits,
        }],
        Some("Edit vector"),
        key.as_deref(),
    );
}

/// The handle points of the segments at `vertex`: `(segment, at_start, point)` in network units.
fn handles(net: &VectorNetwork, vertex: u32) -> Vec<(u32, bool, (f32, f32))> {
    let mut out = Vec::new();
    let Some(v) = net.vertices.get(vertex as usize) else {
        return out;
    };
    for (i, s) in net.segments.iter().enumerate() {
        if s.start == vertex {
            out.push((
                i as u32,
                true,
                (v.x + s.tangent_start.0, v.y + s.tangent_start.1),
            ));
        }
        if s.end == vertex {
            out.push((
                i as u32,
                false,
                (v.x + s.tangent_end.0, v.y + s.tangent_end.1),
            ));
        }
    }
    out
}

/// Vector edit mode on the canvas: paints the points and handles and runs their drags.
pub(super) fn gestures(
    painter: &egui::Painter,
    rect: Rect,
    tab: &mut Tab,
    p: &Pointer,
) -> Option<CursorIcon> {
    let mut state = tab.vector_edit.clone()?;
    if tab.selection != Some(state.node) {
        tab.vector_edit = None;
        return None;
    }
    let Some((net, tf)) = network(tab, state.node) else {
        tab.vector_edit = None;
        return None;
    };
    let origin = rect.min;
    let screen = |x: f32, y: f32| {
        let q = tf * Point::new(f64::from(x), f64::from(y));
        tab.view.to_screen(origin, pos2(q.x as f32, q.y as f32))
    };
    // The path, its points, and the selected point's handles.
    let stroke = Stroke::new(1.0, theme::ACCENT);
    for s in &net.segments {
        let (Some(a), Some(b)) = (
            net.vertices.get(s.start as usize),
            net.vertices.get(s.end as usize),
        ) else {
            continue;
        };
        let pts = [
            screen(a.x, a.y),
            screen(a.x + s.tangent_start.0, a.y + s.tangent_start.1),
            screen(b.x + s.tangent_end.0, b.y + s.tangent_end.1),
            screen(b.x, b.y),
        ];
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            pts,
            false,
            Color32::TRANSPARENT,
            stroke,
        ));
    }
    // A straight point's handles sit on it: they are not shown, and the point stays draggable.
    let hs: Vec<_> = state
        .selected
        .and_then(|v| net.vertices.get(v as usize).map(|p| (v, *p)))
        .map(|(v, p)| {
            handles(&net, v)
                .into_iter()
                .filter(|(_, _, (x, y))| screen(*x, *y).distance(screen(p.x, p.y)) > HIT)
                .collect()
        })
        .unwrap_or_default();
    if let Some(v) = state.selected.and_then(|v| net.vertices.get(v as usize)) {
        for (_, _, (x, y)) in &hs {
            painter.line_segment([screen(v.x, v.y), screen(*x, *y)], stroke);
            painter.circle(screen(*x, *y), 3.5, Color32::WHITE, stroke);
        }
    }
    for (i, v) in net.vertices.iter().enumerate() {
        let r = Rect::from_center_size(screen(v.x, v.y), vec2(7.0, 7.0));
        let on = state.selected == Some(i as u32);
        painter.rect_filled(r, 1.0, if on { theme::ACCENT } else { Color32::WHITE });
        painter.rect_stroke(r, 1.0, stroke, egui::StrokeKind::Inside);
    }

    let pos = p.pos?;
    let inv = tf.inverse();
    let local = |pos: Pos2| {
        let page = tab.view.to_page(origin, pos);
        let q = inv * Point::new(f64::from(page.x), f64::from(page.y));
        (q.x as f32, q.y as f32)
    };
    // A drag under way.
    if let Some(drag) = state.drag {
        if p.down {
            let (x, y) = local(pos);
            let edit = match drag {
                Drag::Vertex(index) => Edit::MoveVertex { index, x, y },
                Drag::Handle(segment, at_start) => {
                    let s = net.segments[segment as usize];
                    let v = net.vertices[if at_start { s.start } else { s.end } as usize];
                    let t = [x - v.x, y - v.y];
                    let (start, end) = if at_start {
                        (t, [s.tangent_end.0, s.tangent_end.1])
                    } else {
                        ([s.tangent_start.0, s.tangent_start.1], t)
                    };
                    Edit::SetHandles {
                        segment,
                        start,
                        end,
                    }
                }
            };
            apply(tab, state.node, vec![edit], true);
            return Some(CursorIcon::Grabbing);
        }
        tab.history.seal_coalesce();
        state.drag = None;
        tab.vector_edit = Some(state);
        return None;
    }
    let near = |x: f32, y: f32| screen(x, y).distance(pos) <= HIT;
    let on_handle = hs
        .iter()
        .find(|(_, _, (x, y))| near(*x, *y))
        .map(|(s, a, _)| Drag::Handle(*s, *a));
    let on_vertex = net
        .vertices
        .iter()
        .position(|v| near(v.x, v.y))
        .map(|i| i as u32);
    if p.pressed {
        if let Some(d) = on_handle {
            state.drag = Some(d);
        } else if let Some(i) = on_vertex {
            state.selected = Some(i);
            state.drag = Some(Drag::Vertex(i));
        } else {
            // Away from the path's box: leave edit mode, as Figma does.
            let b = figstudio_render::node_bounds(&tab.doc, state.node);
            let page = tab.view.to_page(origin, pos);
            if b.is_none_or(|b| {
                !b.inflate(4.0, 4.0)
                    .contains(Point::new(f64::from(page.x), f64::from(page.y)))
            }) {
                tab.vector_edit = None;
                return None;
            }
            state.selected = None;
        }
        tab.vector_edit = Some(state);
        return Some(CursorIcon::Grabbing);
    }
    (on_handle.is_some() || on_vertex.is_some()).then_some(CursorIcon::PointingHand)
}

/// Edit-mode and Pen keys, taken before the app's shortcuts. Returns whether one was used.
pub(crate) fn keys(ctx: &egui::Context, tab: &mut Tab, tool_pen: bool) -> bool {
    if ctx.egui_wants_keyboard_input() {
        return false;
    }
    let pressed = |k: Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
    if tool_pen && tab.pen.is_some() {
        if pressed(Key::Enter) || pressed(Key::Escape) {
            finish_pen(tab, false);
            return true;
        }
        return false;
    }
    let Some(state) = tab.vector_edit.clone() else {
        // Enter on a selected vector starts editing its points.
        if let Some(id) = tab.selection
            && tab.also.is_empty()
            && editable(tab, id)
            && pressed(Key::Enter)
        {
            enter(tab, id);
            return true;
        }
        return false;
    };
    if pressed(Key::Enter) || pressed(Key::Escape) {
        tab.vector_edit = None;
        return true;
    }
    let Some(v) = state.selected else {
        return false;
    };
    if pressed(Key::Backspace) || pressed(Key::Delete) {
        apply(
            tab,
            state.node,
            vec![Edit::RemoveVertex { index: v }],
            false,
        );
        tab.vector_edit = Some(VectorEditing {
            selected: None,
            ..state
        });
        return true;
    }
    let step = if ctx.input(|i| i.modifiers.shift) {
        10.0
    } else {
        1.0
    };
    for (k, d) in [
        (Key::ArrowLeft, vec2(-step, 0.0)),
        (Key::ArrowRight, vec2(step, 0.0)),
        (Key::ArrowUp, vec2(0.0, -step)),
        (Key::ArrowDown, vec2(0.0, step)),
    ] {
        let hit = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, k) || i.consume_key(egui::Modifiers::SHIFT, k)
        });
        if hit && let Some((net, tf)) = network(tab, state.node) {
            // A page-space step, in network units.
            let inv = tf.inverse();
            let o = inv * Point::ZERO;
            let s = inv * Point::new(f64::from(d.x), f64::from(d.y));
            let vx = net.vertices[v as usize];
            let (x, y) = (vx.x + (s.x - o.x) as f32, vx.y + (s.y - o.y) as f32);
            apply(
                tab,
                state.node,
                vec![Edit::MoveVertex { index: v, x, y }],
                false,
            );
            return true;
        }
    }
    false
}

/// The Pen tool on the canvas. Returns the cursor.
pub(super) fn pen(
    painter: &egui::Painter,
    rect: Rect,
    tab: &mut Tab,
    p: &Pointer,
) -> Option<CursorIcon> {
    let origin = rect.min;
    let pos = p.pos.filter(|pos| rect.contains(*pos));
    let mut path = tab.pen.clone().unwrap_or_default();
    let screen = |q: Pos2| tab.view.to_screen(origin, q);
    let near_first = path.points.len() >= 2
        && pos.is_some_and(|pos| screen(path.points[0].0).distance(pos) <= HIT);
    if let Some(pos) = pos {
        let page = tab.view.to_page(origin, pos);
        if p.pressed {
            if near_first {
                tab.pen = Some(path);
                finish_pen(tab, true);
                // Its release would otherwise pick the layer under the pointer.
                tab.ignore_click = p.down;
                return Some(CursorIcon::Crosshair);
            }
            path.points.push((page, Vec2::ZERO));
            path.dragging = true;
        } else if path.dragging && p.down {
            // Dragging pulls the new point's handles out (mirrored on both sides).
            if let Some(last) = path.points.last_mut()
                && screen(last.0).distance(pos) > 2.0
            {
                last.1 = page - last.0;
            }
        }
        if !p.down {
            path.dragging = false;
        }
    }
    // The path so far, and a line to the pointer.
    let stroke = Stroke::new(1.5, theme::ACCENT);
    for w in path.points.windows(2) {
        let (a, b) = (w[0], w[1]);
        painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
            [
                screen(a.0),
                screen(a.0 + a.1),
                screen(b.0 - b.1),
                screen(b.0),
            ],
            false,
            Color32::TRANSPARENT,
            stroke,
        ));
    }
    if let (Some(last), Some(pos)) = (path.points.last(), pos)
        && !path.dragging
    {
        painter.line_segment(
            [screen(last.0), pos],
            Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.6)),
        );
    }
    for (i, (q, h)) in path.points.iter().enumerate() {
        if *h != Vec2::ZERO && i + 1 == path.points.len() {
            painter.line_segment([screen(*q - *h), screen(*q + *h)], stroke);
        }
        let big = i == 0 && near_first;
        let r = Rect::from_center_size(
            screen(*q),
            vec2(if big { 10.0 } else { 7.0 }, if big { 10.0 } else { 7.0 }),
        );
        painter.rect_filled(r, 1.0, Color32::WHITE);
        painter.rect_stroke(r, 1.0, stroke, egui::StrokeKind::Inside);
    }
    tab.pen = (!path.points.is_empty()).then_some(path);
    Some(CursorIcon::Crosshair)
}

/// The network for page points with handles, relative to `origin`.
pub(super) fn pen_network(points: &[(Pos2, Vec2)], closed: bool, origin: Pos2) -> VectorNetwork {
    let vertices = points
        .iter()
        .map(|(q, _)| VectorVertex {
            style: 0,
            x: q.x - origin.x,
            y: q.y - origin.y,
        })
        .collect();
    let n = points.len() as u32;
    let seg = |a: u32, b: u32| VectorSegment {
        style: 0,
        start: a,
        tangent_start: (points[a as usize].1.x, points[a as usize].1.y),
        end: b,
        tangent_end: (-points[b as usize].1.x, -points[b as usize].1.y),
    };
    let mut segments: Vec<VectorSegment> =
        (0..n.saturating_sub(1)).map(|i| seg(i, i + 1)).collect();
    let mut regions = Vec::new();
    if closed && n >= 3 {
        segments.push(seg(n - 1, 0));
        regions.push(VectorRegion {
            winding_nonzero: true,
            style_id: 0,
            loops: vec![(0..n).collect()],
        });
    }
    VectorNetwork {
        vertices,
        segments,
        regions,
    }
}

/// Makes the pen path a vector layer (two points or more): a 1 pt black stroke, no fill.
pub(crate) fn finish_pen(tab: &mut Tab, closed: bool) {
    let Some(path) = tab.pen.take() else { return };
    tab.reset_tool_to_select = true;
    if path.points.len() < 2 {
        return;
    }
    let (min, max) = path.points.iter().fold(
        (pos2(f32::MAX, f32::MAX), pos2(f32::MIN, f32::MIN)),
        |(lo, hi), (q, _)| (lo.min(*q), hi.max(*q)),
    );
    let page = tab.page;
    let key = format!("pen:{}", tab.doc.revision);
    let Ok(ids) = apply_tab_ops(
        tab,
        &[EditOp::CreateShape {
            id: None,
            kind: "VECTOR".into(),
            parent: page,
            name: None,
            x: f64::from(min.x),
            y: f64::from(min.y),
            width: f64::from((max.x - min.x).max(1.0)),
            height: f64::from((max.y - min.y).max(1.0)),
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
        Some("Draw path"),
        Some(&key),
    ) else {
        return;
    };
    let Some(&id) = ids.first() else { return };
    let _ = apply_tab_ops(
        tab,
        &[
            EditOp::SetVectorNetwork {
                target: id,
                network: pen_network(&path.points, closed, min),
            },
            EditOp::SetPaints {
                target: id,
                slot: PaintSlot::Fill,
                paints: Vec::new(),
            },
            EditOp::SetStroke {
                target: id,
                hex: Some("#000000".into()),
                weight: Some(1.0),
                align: Some("CENTER".into()),
            },
        ],
        Some("Draw path"),
        Some(&key),
    );
    tab.history.seal_coalesce();
    tab.select(Some(id));
}

#[cfg(test)]
#[path = "vector_edit_tests.rs"]
mod tests;
