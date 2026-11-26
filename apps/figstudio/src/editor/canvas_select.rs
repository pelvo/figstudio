//! Selection gestures on the canvas: move, resize and rotate every selected layer, Alt-drag to
//! duplicate, snapping with guides and distances. Each gesture is one undo step (its ticks
//! share a coalesce key).

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Stroke, StrokeKind, pos2, vec2,
};
use figstudio_fig::edit::{EditOp, Pivot};
use figstudio_fig::{Document, Guid};
use figstudio_render::absolute_transform;
use figstudio_render::kurbo::{Point, Rect as KRect};

use super::canvas_gizmos::{CanvasDrag, Handle, moved_to, resized_to};
use super::{apply_tab_ops, local_xywh};
use crate::tab::Tab;
use crate::theme;

/// Figma's guide red.
const GUIDE: Color32 = Color32::from_rgb(242, 72, 34);
/// Snap distance, in screen points.
const SNAP_PX: f64 = 5.0;

/// One layer as its gesture started.
#[derive(Clone, Debug)]
pub struct Start {
    pub id: Guid,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    rot: f64,
    /// Page space.
    centre: Point,
    bounds: KRect,
}

/// The pointer input of this frame.
pub(super) struct Pointer {
    pub pos: Option<Pos2>,
    pub pressed: bool,
    pub down: bool,
    pub released: bool,
    pub modifiers: egui::Modifiers,
    /// A primary double-click this frame.
    pub double: bool,
}

/// The selected layers a gesture moves: unlocked, not the page, none inside another one.
fn movable(tab: &Tab) -> Vec<Guid> {
    let sel = tab.selected();
    sel.iter()
        .copied()
        .filter(|&id| {
            tab.doc.node(id).is_some_and(|n| {
                !matches!(n.kind.as_str(), "DOCUMENT" | "CANVAS")
                    && n.data.flag("locked") != Some(true)
            }) && !ancestors(&tab.doc, id).any(|a| sel.contains(&a))
        })
        .collect()
}

fn ancestors(doc: &Document, id: Guid) -> impl Iterator<Item = Guid> + '_ {
    let mut cur = doc.node(id).and_then(|n| n.parent);
    let mut left = 4096;
    std::iter::from_fn(move || {
        let c = cur.filter(|_| left > 0)?;
        left -= 1;
        cur = doc.node(c).and_then(|n| n.parent);
        Some(c)
    })
}

fn starts(doc: &Document, ids: &[Guid]) -> Vec<Start> {
    ids.iter()
        .filter_map(|&id| {
            let n = doc.node(id)?;
            let (x, y, w, h) = local_xywh(&n.data);
            let b = figstudio_render::node_bounds(doc, id)?;
            Some(Start {
                id,
                x,
                y,
                w,
                h,
                rot: super::commands::current_rotation_of(doc, id),
                centre: absolute_transform(doc, id) * Point::new(w / 2.0, h / 2.0),
                bounds: b,
            })
        })
        .collect()
}

fn union(starts: &[Start]) -> Option<KRect> {
    starts.iter().map(|s| s.bounds).reduce(|a, b| a.union(b))
}

/// The selection box on screen as four corners (NW, NE, SE, SW): one layer's own rotated box,
/// or the page-aligned box around several.
fn quad(tab: &Tab, origin: Pos2, ids: &[Guid]) -> Option<[Pos2; 4]> {
    let to_screen = |p: Point| tab.view.to_screen(origin, pos2(p.x as f32, p.y as f32));
    if let [id] = ids {
        let n = tab.doc.node(*id)?;
        let (_, _, w, h) = local_xywh(&n.data);
        let t = absolute_transform(&tab.doc, *id);
        // Groups and others without a size use their drawn bounds.
        if w > 0.0 || h > 0.0 {
            return Some(
                [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
                    .map(|(x, y)| to_screen(t * Point::new(x, y))),
            );
        }
    }
    let b = union(&starts(&tab.doc, ids))?;
    Some(
        [(b.x0, b.y0), (b.x1, b.y0), (b.x1, b.y1), (b.x0, b.y1)]
            .map(|(x, y)| to_screen(Point::new(x, y))),
    )
}

fn handle_pos(h: Handle, q: &[Pos2; 4]) -> Pos2 {
    let mid = |a: Pos2, b: Pos2| a + (b - a) * 0.5;
    match h {
        Handle::NW => q[0],
        Handle::N => mid(q[0], q[1]),
        Handle::NE => q[1],
        Handle::E => mid(q[1], q[2]),
        Handle::SE => q[2],
        Handle::S => mid(q[2], q[3]),
        Handle::SW => q[3],
        Handle::W => mid(q[3], q[0]),
    }
}

/// `p` inside the convex quad `q`.
fn inside(q: &[Pos2; 4], p: Pos2) -> bool {
    let side = |a: Pos2, b: Pos2| (b - a).x * (p - a).y - (b - a).y * (p - a).x;
    let s: Vec<f32> = (0..4).map(|i| side(q[i], q[(i + 1) % 4])).collect();
    s.iter().all(|v| *v >= 0.0) || s.iter().all(|v| *v <= 0.0)
}

fn paint_box(painter: &egui::Painter, q: &[Pos2; 4]) {
    let mut pts = q.to_vec();
    pts.push(q[0]);
    painter.add(egui::Shape::line(pts, Stroke::new(1.0, theme::ACCENT)));
    for h in Handle::ALL {
        let hr = Rect::from_center_size(handle_pos(h, q), vec2(7.0, 7.0));
        painter.rect_filled(hr, CornerRadius::same(1), Color32::WHITE);
        painter.rect_stroke(
            hr,
            CornerRadius::same(1),
            Stroke::new(1.5, theme::ACCENT),
            StrokeKind::Inside,
        );
    }
}

/// The page box `b` after dragging `handle` by `(dx, dy)`; the opposite side stays, and
/// `aspect` keeps the width-to-height ratio.
pub(super) fn box_resized(b: KRect, handle: Handle, dx: f64, dy: f64, aspect: bool) -> KRect {
    let (mut x0, mut y0, mut x1, mut y1) = (b.x0, b.y0, b.x1, b.y1);
    match handle {
        Handle::E | Handle::NE | Handle::SE => x1 = (x1 + dx).max(x0 + 1.0),
        Handle::W | Handle::NW | Handle::SW => x0 = (x0 + dx).min(x1 - 1.0),
        _ => {}
    }
    match handle {
        Handle::S | Handle::SE | Handle::SW => y1 = (y1 + dy).max(y0 + 1.0),
        Handle::N | Handle::NW | Handle::NE => y0 = (y0 + dy).min(y1 - 1.0),
        _ => {}
    }
    if aspect && b.width() > 0.0 && b.height() > 0.0 {
        let (sx, sy) = ((x1 - x0) / b.width(), (y1 - y0) / b.height());
        let s = match handle {
            Handle::E | Handle::W => sx,
            Handle::N | Handle::S => sy,
            _ if (sx - 1.0).abs() >= (sy - 1.0).abs() => sx,
            _ => sy,
        };
        let (w, h) = (b.width() * s, b.height() * s);
        match handle {
            Handle::W | Handle::NW | Handle::SW => x0 = x1 - w,
            _ => x1 = x0 + w,
        }
        match handle {
            Handle::N | Handle::NW | Handle::NE => y0 = y1 - h,
            _ => y1 = y0 + h,
        }
    }
    KRect::new(x0, y0, x1, y1)
}

/// A snapping guide: a line at `at` (x for vertical lines) from `from` to `to`, page space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Guide {
    pub vertical: bool,
    pub at: f64,
    pub from: f64,
    pub to: f64,
}

/// How far to shift `moving` so one of its edges or centre lines meets one of `others`
/// within `tol`, per axis, and the guides to draw.
pub(super) fn snap(moving: KRect, others: &[KRect], tol: f64) -> (f64, f64, Vec<Guide>) {
    let xs = |r: &KRect| [r.x0, (r.x0 + r.x1) / 2.0, r.x1];
    let ys = |r: &KRect| [r.y0, (r.y0 + r.y1) / 2.0, r.y1];
    let best = |mine: [f64; 3], theirs: &dyn Fn(&KRect) -> [f64; 3]| {
        let mut best: Option<f64> = None;
        for o in others {
            for t in theirs(o) {
                for m in mine {
                    let d = t - m;
                    if d.abs() <= tol && best.is_none_or(|b| d.abs() < b.abs()) {
                        best = Some(d);
                    }
                }
            }
        }
        best
    };
    let sx = best(xs(&moving), &xs).unwrap_or(0.0);
    let sy = best(ys(&moving), &ys).unwrap_or(0.0);
    let moved = moving + figstudio_render::kurbo::Vec2::new(sx, sy);
    let mut guides = Vec::new();
    for o in others {
        for t in xs(o) {
            if xs(&moved).iter().any(|m| (m - t).abs() < 1e-6) {
                guides.push(Guide {
                    vertical: true,
                    at: t,
                    from: moved.y0.min(o.y0),
                    to: moved.y1.max(o.y1),
                });
            }
        }
        for t in ys(o) {
            if ys(&moved).iter().any(|m| (m - t).abs() < 1e-6) {
                guides.push(Guide {
                    vertical: false,
                    at: t,
                    from: moved.x0.min(o.x0),
                    to: moved.x1.max(o.x1),
                });
            }
        }
    }
    (sx, sy, guides)
}

/// The gaps from `moving` to its nearest neighbour on each side that overlaps it across:
/// `(from, to)` page points of a measuring line.
pub(super) fn gaps(moving: KRect, others: &[KRect]) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    let across_y = |o: &KRect| o.y0 < moving.y1 && o.y1 > moving.y0;
    let across_x = |o: &KRect| o.x0 < moving.x1 && o.x1 > moving.x0;
    let mid = |a0: f64, a1: f64, b0: f64, b1: f64| (a0.max(b0) + a1.min(b1)) / 2.0;
    let near = |f: &dyn Fn(&KRect) -> Option<f64>| {
        others
            .iter()
            .filter_map(|o| f(o).map(|d| (d, *o)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, o)| o)
    };
    if let Some(o) = near(&|o| (across_y(o) && o.x0 >= moving.x1).then_some(o.x0 - moving.x1)) {
        let y = mid(moving.y0, moving.y1, o.y0, o.y1);
        out.push((Point::new(moving.x1, y), Point::new(o.x0, y)));
    }
    if let Some(o) = near(&|o| (across_y(o) && o.x1 <= moving.x0).then_some(moving.x0 - o.x1)) {
        let y = mid(moving.y0, moving.y1, o.y0, o.y1);
        out.push((Point::new(o.x1, y), Point::new(moving.x0, y)));
    }
    if let Some(o) = near(&|o| (across_x(o) && o.y0 >= moving.y1).then_some(o.y0 - moving.y1)) {
        let x = mid(moving.x0, moving.x1, o.x0, o.x1);
        out.push((Point::new(x, moving.y1), Point::new(x, o.y0)));
    }
    if let Some(o) = near(&|o| (across_x(o) && o.y1 <= moving.y0).then_some(moving.y0 - o.y1)) {
        let x = mid(moving.x0, moving.x1, o.x0, o.x1);
        out.push((Point::new(x, o.y1), Point::new(x, moving.y0)));
    }
    out
}

/// What the moving layers snap to: their siblings and the frame they sit in, page space.
fn snap_targets(doc: &Document, moving: &[Guid]) -> Vec<KRect> {
    let mut parents: Vec<Guid> = moving
        .iter()
        .filter_map(|id| doc.node(*id)?.parent)
        .collect();
    parents.dedup();
    let mut out = Vec::new();
    for p in parents {
        if doc.node(p).is_some_and(|n| n.kind != "CANVAS")
            && let Some(b) = figstudio_render::node_bounds(doc, p)
        {
            out.push(b);
        }
        for &c in doc.children(p) {
            if moving.contains(&c)
                || doc
                    .node(c)
                    .is_some_and(|n| n.data.flag("visible") == Some(false))
            {
                continue;
            }
            if let Some(b) = figstudio_render::node_bounds(doc, c) {
                out.push(b);
            }
        }
    }
    out
}

/// Neighbours for the distance lines: siblings only (not the parent frame).
fn siblings(doc: &Document, moving: &[Guid]) -> Vec<KRect> {
    let parent = moving.first().and_then(|id| doc.node(*id)?.parent);
    parent
        .map(|p| {
            doc.children(p)
                .iter()
                .filter(|c| {
                    !moving.contains(c)
                        && doc
                            .node(**c)
                            .is_some_and(|n| n.data.flag("visible") != Some(false))
                })
                .filter_map(|&c| figstudio_render::node_bounds(doc, c))
                .collect()
        })
        .unwrap_or_default()
}

fn paint_guides(
    painter: &egui::Painter,
    tab: &Tab,
    origin: Pos2,
    guides: &[Guide],
    gaps: &[(Point, Point)],
) {
    let s = |x: f64, y: f64| tab.view.to_screen(origin, pos2(x as f32, y as f32));
    let stroke = Stroke::new(1.0, GUIDE);
    for g in guides {
        let (a, b) = if g.vertical {
            (s(g.at, g.from), s(g.at, g.to))
        } else {
            (s(g.from, g.at), s(g.to, g.at))
        };
        painter.line_segment([a, b], stroke);
    }
    for (a, b) in gaps {
        let (sa, sb) = (s(a.x, a.y), s(b.x, b.y));
        painter.line_segment([sa, sb], stroke);
        let d = (b.x - a.x).abs().max((b.y - a.y).abs());
        let text = super::widgets::fmt((d * 10.0).round() / 10.0);
        let at = sa + (sb - sa) * 0.5;
        let galley = painter.layout_no_wrap(text, FontId::proportional(10.0), Color32::WHITE);
        let r = Align2::CENTER_CENTER.anchor_size(at, galley.size() + vec2(8.0, 4.0));
        painter.rect_filled(r, CornerRadius::same(3), GUIDE);
        painter.galley(r.center() - galley.size() / 2.0, galley, Color32::WHITE);
    }
}

/// Selection gestures with the Select tool. Returns the cursor when one owns the pointer.
pub(super) fn gestures(
    painter: &egui::Painter,
    rect: Rect,
    tab: &mut Tab,
    p: &Pointer,
    snapping: bool,
) -> Option<CursorIcon> {
    let origin = rect.min;
    if tab.vector_edit.is_some() {
        return super::vector_edit::gestures(painter, rect, tab, p);
    }
    // A double-click on a selected vector edits its points.
    if p.double
        && let (Some(id), true) = (tab.selection, tab.also.is_empty())
        && super::vector_edit::editable(tab, id)
    {
        super::vector_edit::enter(tab, id);
        return None;
    }
    // 1. A gesture under way.
    if let Some(drag) = tab.edit_drag.clone() {
        if p.down
            && let Some(pos) = p.pos
        {
            return drag_tick(
                painter,
                origin,
                tab,
                drag,
                tab.view.to_page(origin, pos),
                p,
                snapping,
            );
        }
        if p.released || !p.down {
            tab.history.seal_coalesce();
            tab.edit_drag = None;
        }
    }

    // 2. The selection box: handles resize, just outside a corner rotates, inside moves.
    let ids = movable(tab);
    let q = if ids.is_empty() {
        None
    } else {
        quad(tab, origin, &ids)
    };
    if let Some(q) = &q {
        paint_box(painter, q);
    }
    let pos = p.pos.filter(|pos| rect.contains(*pos))?;
    let page = tab.view.to_page(origin, pos);
    if let Some(q) = q {
        for h in Handle::ALL {
            if Rect::from_center_size(handle_pos(h, &q), vec2(10.0, 10.0)).contains(pos) {
                if p.pressed {
                    tab.edit_drag = Some(CanvasDrag::Resize {
                        layers: starts(&tab.doc, &ids),
                        handle: h,
                        start_pointer_page: page,
                    });
                }
                return Some(h.cursor());
            }
        }
        let near_corner = [q[0], q[1], q[2], q[3]]
            .iter()
            .any(|c| c.distance(pos) <= 18.0);
        if near_corner && !inside(&q, pos) {
            if p.pressed {
                let layers = starts(&tab.doc, &ids);
                let centre = union(&layers).map_or(Point::ZERO, |b| b.center());
                // One layer turns about its own centre, which may differ from its bounds'.
                let centre = if let [one] = layers.as_slice() {
                    one.centre
                } else {
                    centre
                };
                tab.edit_drag = Some(CanvasDrag::Rotate {
                    layers,
                    start_pointer_page: page,
                    centre,
                });
            }
            painter.text(
                pos + vec2(12.0, 12.0),
                Align2::LEFT_TOP,
                "↻",
                FontId::proportional(14.0),
                theme::ACCENT,
            );
            return Some(CursorIcon::Crosshair);
        }
        if p.pressed && inside(&q, pos) && !p.modifiers.shift {
            start_move(tab, &ids, page, None);
            return None;
        }
    }
    // Pressing an unselected layer drags it (it is selected once the drag starts). As in
    // Figma, the empty area of a top-level frame or section with layers draws a box instead.
    if p.pressed
        && !p.modifiers.shift
        && let Some(h) = tab.hover
        && !tab.selected().contains(&h)
        && !is_board(&tab.doc, h)
        && tab
            .doc
            .node(h)
            .is_some_and(|n| n.data.flag("locked") != Some(true))
    {
        start_move(tab, &[h], page, Some(h));
    }
    None
}

/// A frame or section with layers right on the page or in a section.
fn is_board(doc: &Document, id: Guid) -> bool {
    let Some(n) = doc.node(id) else {
        return false;
    };
    let parent_kind = n.parent.and_then(|p| doc.node(p)).map(|p| p.kind.as_str());
    matches!(n.kind.as_str(), "FRAME" | "SECTION" | "SYMBOL")
        && !doc.children(id).is_empty()
        && matches!(parent_kind, Some("CANVAS" | "SECTION"))
}

fn start_move(tab: &mut Tab, ids: &[Guid], page: Pos2, select: Option<Guid>) {
    let layers = starts(&tab.doc, ids);
    let key = format!(
        "gui:move:{}",
        ids.first().map(Guid::to_string).unwrap_or_default()
    );
    tab.edit_drag = Some(CanvasDrag::Move {
        layers,
        start_pointer_page: page,
        moved: false,
        key,
        select,
    });
}

fn drag_tick(
    painter: &egui::Painter,
    origin: Pos2,
    tab: &mut Tab,
    drag: CanvasDrag,
    cur: Pos2,
    p: &Pointer,
    snapping: bool,
) -> Option<CursorIcon> {
    let zoom = f64::from(tab.view.zoom);
    match drag {
        CanvasDrag::Move {
            mut layers,
            start_pointer_page,
            moved,
            key,
            select,
        } => {
            let (mut dx, mut dy) = (
                f64::from(cur.x - start_pointer_page.x),
                f64::from(cur.y - start_pointer_page.y),
            );
            if !moved && dx.hypot(dy) * zoom < 3.0 {
                return None;
            }
            if !moved {
                if let Some(id) = select {
                    tab.select(Some(id));
                }
                // Alt-drag moves copies; the originals stay. One undo step with the move.
                if p.modifiers.alt {
                    let ops: Vec<EditOp> = layers
                        .iter()
                        .map(|s| EditOp::DuplicateNode {
                            target: s.id,
                            dx: 0.0,
                            dy: 0.0,
                        })
                        .collect();
                    if let Ok(copies) =
                        apply_tab_ops(tab, &ops, Some("Duplicate layers"), Some(&key))
                        && copies.len() == layers.len()
                    {
                        for (s, c) in layers.iter_mut().zip(&copies) {
                            s.id = *c;
                        }
                        tab.select_all_of(&copies);
                    }
                }
            }
            let ids: Vec<Guid> = layers.iter().map(|s| s.id).collect();
            let mut guides = Vec::new();
            let mut spacing = Vec::new();
            if let Some(b) = union(&layers) {
                let moving = b + figstudio_render::kurbo::Vec2::new(dx, dy);
                // ⌘/Ctrl held moves freely.
                if snapping && !(p.modifiers.command || p.modifiers.ctrl) {
                    let (sx, sy, g) = snap(moving, &snap_targets(&tab.doc, &ids), SNAP_PX / zoom);
                    dx += sx;
                    dy += sy;
                    guides = g;
                }
                let moved_box = b + figstudio_render::kurbo::Vec2::new(dx, dy);
                spacing = gaps(moved_box, &siblings(&tab.doc, &ids));
            }
            let ops: Vec<EditOp> = layers
                .iter()
                .map(|s| {
                    let (x, y) = moved_to(&tab.doc, s.id, s.x, s.y, dx, dy);
                    EditOp::Move {
                        target: s.id,
                        x: x.round(),
                        y: y.round(),
                        relative: false,
                    }
                })
                .collect();
            let label = if ops.len() == 1 {
                "Move layer"
            } else {
                "Move layers"
            };
            let _ = apply_tab_ops(tab, &ops, Some(label), Some(&key));
            paint_guides(painter, tab, origin, &guides, &spacing);
            tab.edit_drag = Some(CanvasDrag::Move {
                layers,
                start_pointer_page,
                moved: true,
                key,
                select: None,
            });
            Some(CursorIcon::Grabbing)
        }
        CanvasDrag::Resize {
            layers,
            handle,
            start_pointer_page,
        } => {
            let (dx, dy) = (
                f64::from(cur.x - start_pointer_page.x),
                f64::from(cur.y - start_pointer_page.y),
            );
            let key = format!(
                "gui:resize:{}",
                layers.first().map(|s| s.id.to_string()).unwrap_or_default()
            );
            let ops = resize_ops(&tab.doc, &layers, handle, dx, dy, p.modifiers.shift);
            let label = if layers.len() == 1 {
                "Resize layer"
            } else {
                "Resize layers"
            };
            let _ = apply_tab_ops(tab, &ops, Some(label), Some(&key));
            Some(handle.cursor())
        }
        CanvasDrag::Rotate {
            layers,
            start_pointer_page,
            centre,
        } => {
            let angle = |p: Pos2| (f64::from(p.y) - centre.y).atan2(f64::from(p.x) - centre.x);
            let mut delta = (angle(cur) - angle(start_pointer_page)).to_degrees();
            // Shift turns in 15° steps (of the first layer's angle).
            if p.modifiers.shift
                && let Some(first) = layers.first()
            {
                delta = ((first.rot + delta) / 15.0).round() * 15.0 - first.rot;
            }
            let key = format!(
                "gui:rotate:{}",
                layers.first().map(|s| s.id.to_string()).unwrap_or_default()
            );
            let ops = rotate_ops(&tab.doc, &layers, centre, delta);
            let label = if layers.len() == 1 {
                "Rotate layer"
            } else {
                "Rotate layers"
            };
            let _ = apply_tab_ops(tab, &ops, Some(label), Some(&key));
            let at = tab.view.to_screen(origin, cur) + vec2(14.0, 14.0);
            let text = format!(
                "{}°",
                super::widgets::fmt(
                    normalised(layers.first().map_or(0.0, |s| s.rot) + delta).round()
                )
            );
            let galley = painter.layout_no_wrap(text, FontId::proportional(11.0), Color32::WHITE);
            let r = Rect::from_min_size(at, galley.size() + vec2(8.0, 4.0));
            painter.rect_filled(r, CornerRadius::same(3), theme::ACCENT);
            painter.galley(r.min + vec2(4.0, 2.0), galley, Color32::WHITE);
            Some(CursorIcon::Crosshair)
        }
        CanvasDrag::Create { .. } => None,
    }
}

/// An angle in (-180, 180].
fn normalised(deg: f64) -> f64 {
    let d = deg.rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// One layer resizes along its own axes (rotated layers too); several scale inside their
/// shared page box.
pub(super) fn resize_ops(
    doc: &Document,
    layers: &[Start],
    handle: Handle,
    dx: f64,
    dy: f64,
    shift: bool,
) -> Vec<EditOp> {
    if let [s] = layers {
        let locked = doc
            .node(s.id)
            .is_some_and(|n| n.data.flag("proportionsLocked") == Some(true));
        let (nx, ny, nw, nh) = resized_to(
            doc,
            s.id,
            handle,
            (s.x, s.y, s.w, s.h),
            dx,
            dy,
            shift || locked,
        );
        return vec![
            EditOp::Move {
                target: s.id,
                x: nx.round(),
                y: ny.round(),
                relative: false,
            },
            EditOp::Resize {
                target: s.id,
                width: nw.round(),
                height: nh.round(),
            },
        ];
    }
    let Some(b0) = union(layers) else {
        return Vec::new();
    };
    let b1 = box_resized(b0, handle, dx, dy, shift);
    let (kx, ky) = (
        b1.width() / b0.width().max(1e-9),
        b1.height() / b0.height().max(1e-9),
    );
    // shortcut: a rotated layer among several keeps its own shape and only scales its box's
    // size along its axes; Figma would skew it. Upgrade with a matrix scale in the engine.
    layers
        .iter()
        .flat_map(|s| {
            let to = Point::new(
                b1.x0 + (s.bounds.x0 - b0.x0) * kx,
                b1.y0 + (s.bounds.y0 - b0.y0) * ky,
            );
            let (x, y) = moved_to(doc, s.id, s.x, s.y, to.x - s.bounds.x0, to.y - s.bounds.y0);
            [
                EditOp::Move {
                    target: s.id,
                    x: x.round(),
                    y: y.round(),
                    relative: false,
                },
                EditOp::Resize {
                    target: s.id,
                    width: (s.w * kx).round().max(1.0),
                    height: (s.h * ky).round().max(1.0),
                },
            ]
        })
        .collect()
}

/// Each layer turns by `delta` degrees about its own centre, and that centre turns about
/// `centre` (page space), so several layers turn as one.
pub(super) fn rotate_ops(
    doc: &Document,
    layers: &[Start],
    centre: Point,
    delta: f64,
) -> Vec<EditOp> {
    let (sin, cos) = delta.to_radians().sin_cos();
    let mut ops = Vec::new();
    for s in layers {
        ops.push(EditOp::Rotate {
            target: s.id,
            degrees: normalised(s.rot + delta),
            pivot: Pivot::Center,
        });
        let v = s.centre - centre;
        let want = centre
            + figstudio_render::kurbo::Vec2::new(cos * v.x - sin * v.y, sin * v.x + cos * v.y);
        // Where the centre is now (the rotation keeps it), so the move is relative.
        let (w, h) = doc.node(s.id).map_or((0.0, 0.0), |n| {
            let (_, _, w, h) = local_xywh(&n.data);
            (w, h)
        });
        let now = absolute_transform(doc, s.id) * Point::new(w / 2.0, h / 2.0);
        let (ox, oy) = moved_to(doc, s.id, 0.0, 0.0, want.x - now.x, want.y - now.y);
        if ox.abs() > 1e-9 || oy.abs() > 1e-9 {
            ops.push(EditOp::Move {
                target: s.id,
                x: ox,
                y: oy,
                relative: true,
            });
        }
    }
    ops
}

/// Arrow keys move the selection by 1 (Shift: 10) page units, one undo step each.
pub(crate) fn nudge(tab: &mut Tab, dx: f64, dy: f64) {
    let ids = movable(tab);
    let ops: Vec<EditOp> = ids
        .iter()
        .filter_map(|&id| {
            let (x, y, _, _) = local_xywh(&tab.doc.node(id)?.data);
            let (nx, ny) = moved_to(&tab.doc, id, x, y, dx, dy);
            Some(EditOp::Move {
                target: id,
                x: nx,
                y: ny,
                relative: false,
            })
        })
        .collect();
    if !ops.is_empty() {
        let _ = apply_tab_ops(tab, &ops, Some("Nudge"), None);
    }
}

#[cfg(test)]
#[path = "canvas_select_tests.rs"]
mod tests;
