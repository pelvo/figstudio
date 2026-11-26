//! Arrangement [`EditOp`]s: group, ungroup, frame selection, align, distribute, flip, rotate
//! about a pivot, and z-order.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::edit::geometry::matrix_2x3;
use crate::edit::groups::is_group;
use crate::edit::space::{Bounds, Xform, bounds_in_parent, layer_bounds, size, world_transform};
use crate::edit::{GuidAlloc, Txn, fractional};
use crate::{Document, FigError, Guid, Obj, Val, intern};

use super::EditOp;
use super::compile_tree::ensure_exists;

/// Which edge or centre line [`EditOp::Align`] lines layers up on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignEdge {
    Left,
    HCenter,
    Right,
    Top,
    VCenter,
    Bottom,
}

/// What [`EditOp::Align`] aligns to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlignTo {
    /// The union of the targets (one target aligns to its parent instead).
    #[default]
    Selection,
    /// Each target's parent frame.
    Parent,
}

/// A horizontal or vertical axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}

/// The point [`EditOp::Rotate`] turns a layer about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pivot {
    /// The layer's origin (its top-left corner before rotation).
    #[default]
    TopLeft,
    /// The layer's centre, as Figma's rotate handle and field do.
    Center,
}

/// Where [`EditOp::Reorder`] moves a layer among its siblings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZOrder {
    Front,
    Back,
    Forward,
    Backward,
}

pub(super) fn compile_arrange_op(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    op: &EditOp,
    created_ids: &mut Vec<Guid>,
    layout_touched: &mut Vec<Guid>,
) -> Result<(), FigError> {
    match op {
        EditOp::Group { targets, name } => {
            let id = wrap(doc, txn, alloc, targets, name.as_deref(), true)?;
            created_ids.push(id);
            layout_touched.push(id);
        }
        EditOp::FrameSelection { targets, name } => {
            let id = wrap(doc, txn, alloc, targets, name.as_deref(), false)?;
            created_ids.push(id);
            layout_touched.push(id);
        }
        EditOp::Ungroup { target } => {
            let parent = ungroup(doc, txn, *target)?;
            layout_touched.push(parent);
        }
        EditOp::Align { targets, edge, to } => {
            align(doc, txn, targets, *edge, *to)?;
            layout_touched.extend_from_slice(targets);
        }
        EditOp::Distribute {
            targets,
            axis,
            spacing,
        } => {
            distribute(doc, txn, targets, *axis, *spacing)?;
            layout_touched.extend_from_slice(targets);
        }
        EditOp::Flip { target, axis } => {
            let node = ensure_exists(doc, *target)?;
            let (w, h) = size(&node.data);
            // Mirror about the layer's own centre line: T' = T ∘ F.
            let f = match axis {
                Axis::Horizontal => Xform {
                    m00: -1.0,
                    m02: w,
                    ..Xform::IDENTITY
                },
                Axis::Vertical => Xform {
                    m11: -1.0,
                    m12: h,
                    ..Xform::IDENTITY
                },
            };
            let t = finite(Xform::of(&node.data), *target)?;
            txn.set_field(*target, "transform", t.compose(f).to_val());
            layout_touched.push(*target);
        }
        EditOp::Reorder { target, to } => {
            let node = ensure_exists(doc, *target)?;
            let parent = node
                .parent
                .ok_or_else(|| FigError::malformed(format!("{target} has no parent")))?;
            let current = doc
                .children(parent)
                .iter()
                .position(|c| c == target)
                .unwrap_or(0);
            let others = doc.children(parent).len().saturating_sub(1);
            let index = match to {
                ZOrder::Front => others,
                ZOrder::Back => 0,
                ZOrder::Forward => (current + 1).min(others),
                ZOrder::Backward => current.saturating_sub(1),
            };
            place(doc, txn, *target, parent, index)?;
            layout_touched.push(parent);
        }
        _ => return Err(FigError::malformed("not an arrangement op")),
    }
    Ok(())
}

/// `transform` with a finite matrix, else an error naming `id` (files can hold NaN).
fn finite(t: Xform, id: Guid) -> Result<Xform, FigError> {
    let all = [t.m00, t.m01, t.m02, t.m10, t.m11, t.m12];
    if all.iter().all(|v| v.is_finite()) {
        Ok(t)
    } else {
        Err(FigError::malformed(format!(
            "{id} has a non-finite transform"
        )))
    }
}

/// Rotation for [`EditOp::Rotate`]: sets the rotation to `degrees` (dropping any scale or flip,
/// as the plain op always has) about `pivot`.
pub(super) fn rotated(data: &Val, degrees: f64, pivot: Pivot) -> Val {
    let t = Xform::of(data);
    match pivot {
        Pivot::TopLeft => matrix_2x3(t.m02, t.m12, degrees),
        Pivot::Center => {
            let (w, h) = size(data);
            let centre = t.apply((w / 2.0, h / 2.0));
            // Where the centre lands under the bare rotation (same convention as `matrix_2x3`).
            let (sin, cos) = degrees.to_radians().sin_cos();
            let off = (cos * w / 2.0 - sin * h / 2.0, sin * w / 2.0 + cos * h / 2.0);
            matrix_2x3(centre.0 - off.0, centre.1 - off.1, degrees)
        }
    }
}

/// The shared parent of `targets`, checked: non-empty, all present, no repeats.
fn shared_parent(doc: &Document, targets: &[Guid]) -> Result<Guid, FigError> {
    let first = targets
        .first()
        .ok_or_else(|| FigError::malformed("no targets"))?;
    let parent = ensure_exists(doc, *first)?
        .parent
        .ok_or_else(|| FigError::malformed(format!("{first} has no parent")))?;
    let mut seen = HashSet::new();
    for t in targets {
        if !seen.insert(*t) {
            return Err(FigError::malformed(format!("{t} is listed twice")));
        }
        if ensure_exists(doc, *t)?.parent != Some(parent) {
            return Err(FigError::malformed(
                "targets must share one parent to be grouped",
            ));
        }
    }
    if doc.node(parent).is_some_and(|p| p.kind == "DOCUMENT") {
        return Err(FigError::malformed("pages cannot be grouped"));
    }
    Ok(parent)
}

/// Wraps `targets` in a new group (`group`) or frame, at the topmost target's place; every
/// target keeps its page position and the targets keep their order.
pub(super) fn wrap(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    targets: &[Guid],
    name: Option<&str>,
    group: bool,
) -> Result<Guid, FigError> {
    let parent = shared_parent(doc, targets)?;
    let siblings = doc.children(parent);
    let mut ordered: Vec<Guid> = siblings
        .iter()
        .copied()
        .filter(|s| targets.contains(s))
        .collect();
    if ordered.len() != targets.len() {
        return Err(FigError::malformed("a target is not a child of its parent"));
    }
    let union = ordered
        .iter()
        .filter_map(|&t| doc.node(t).and_then(|n| bounds_in_parent(&n.data)))
        .reduce(Bounds::union)
        .ok_or_else(|| FigError::malformed("targets have no bounds"))?;
    // The topmost target's slot is free once it moves inside, so the wrapper takes it.
    let top = *ordered
        .last()
        .ok_or_else(|| FigError::malformed("no targets"))?;
    let position = ensure_exists(doc, top)?.position.clone();

    let id = alloc.next_guid();
    let mut obj = Obj::new();
    obj.insert("guid", id.to_val());
    obj.insert("phase", Val::Enum(intern("CREATED")));
    obj.insert("type", Val::Enum(intern("FRAME")));
    let default = if group { "Group" } else { "Frame" };
    obj.insert("name", Val::Str(name.unwrap_or(default).to_string()));
    obj.insert("visible", Val::Bool(true));
    obj.insert("opacity", Val::Float(1.0));
    obj.insert("transform", Xform::translate(union.x0, union.y0).to_val());
    obj.insert(
        "size",
        Val::Obj(Obj::from([
            ("x", Val::Float(union.width())),
            ("y", Val::Float(union.height())),
        ])),
    );
    obj.insert(
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", parent.to_val()),
            ("position", Val::Str(position)),
        ])),
    );
    obj.insert("horizontalConstraint", Val::Enum(intern("MIN")));
    obj.insert("verticalConstraint", Val::Enum(intern("MIN")));
    if group {
        obj.insert("resizeToFit", Val::Bool(true));
    } else {
        obj.insert("frameMaskDisabled", Val::Bool(false));
        obj.insert(
            "fillPaints",
            Val::Arr(vec![crate::edit::styles_vars::hex_paint("#FFFFFF")?]),
        );
    }
    txn.create_node(id, Val::Obj(obj));

    let back = Xform::translate(-union.x0, -union.y0);
    let positions = fractional::n_between(None, None, ordered.len());
    for (t, pos) in ordered.drain(..).zip(positions) {
        let node = ensure_exists(doc, t)?;
        let mut set = Obj::new();
        set.insert("transform", back.compose(Xform::of(&node.data)).to_val());
        set.insert(
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", id.to_val()),
                ("position", Val::Str(pos)),
            ])),
        );
        txn.modify(t, set, Vec::new());
    }
    Ok(id)
}

/// Moves `target`'s children to its parent at its place and deletes it. Returns the parent.
fn ungroup(doc: &Document, txn: &mut Txn, target: Guid) -> Result<Guid, FigError> {
    let node = ensure_exists(doc, target)?;
    if !is_group(node) {
        return Err(FigError::malformed(format!("{target} is not a group")));
    }
    let parent = node
        .parent
        .ok_or_else(|| FigError::malformed(format!("{target} has no parent")))?;
    let siblings = doc.children(parent);
    let at = siblings.iter().position(|s| *s == target).unwrap_or(0);
    let prev = at
        .checked_sub(1)
        .and_then(|i| siblings.get(i))
        .and_then(|id| doc.node(*id))
        .map(|n| n.position.as_str());
    let next = siblings
        .get(at + 1)
        .and_then(|id| doc.node(*id))
        .map(|n| n.position.as_str());
    let kids = doc.children(target);
    let mut positions = Vec::with_capacity(kids.len());
    let mut low = prev.map(str::to_string);
    for _ in kids {
        let p = fractional::try_between(low.as_deref(), next).ok_or_else(|| {
            FigError::malformed("the sibling positions around the group are malformed")
        })?;
        low = Some(p.clone());
        positions.push(p);
    }
    let outer = finite(Xform::of(&node.data), target)?;
    for (&k, pos) in kids.iter().zip(positions) {
        let child = ensure_exists(doc, k)?;
        let mut set = Obj::new();
        set.insert("transform", outer.compose(Xform::of(&child.data)).to_val());
        set.insert(
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", parent.to_val()),
                ("position", Val::Str(pos)),
            ])),
        );
        txn.modify(k, set, Vec::new());
    }
    txn.delete_node(target);
    Ok(parent)
}

/// Moves `target` to index `index` among `parent`'s other children (`Reparent`'s placement).
pub(super) fn place(
    doc: &Document,
    txn: &mut Txn,
    target: Guid,
    parent: Guid,
    index: usize,
) -> Result<(), FigError> {
    let siblings: Vec<Guid> = doc
        .children(parent)
        .iter()
        .copied()
        .filter(|&id| id != target)
        .collect();
    let idx = index.min(siblings.len());
    let pos_of = |i: Option<usize>| {
        i.and_then(|i| siblings.get(i))
            .and_then(|id| doc.node(*id))
            .map(|n| n.position.as_str())
    };
    let pos = fractional::try_between(pos_of(idx.checked_sub(1)), pos_of(Some(idx)))
        .ok_or_else(|| {
            FigError::malformed(format!(
                "cannot place {target} at index {idx}: the sibling positions there are out of order or malformed"
            ))
        })?;
    txn.set_field(
        target,
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", parent.to_val()),
            ("position", Val::Str(pos)),
        ])),
    );
    Ok(())
}

/// Page-space bounds of each target, checked.
fn page_bounds(doc: &Document, targets: &[Guid]) -> Result<Vec<(Guid, Bounds)>, FigError> {
    if targets.is_empty() {
        return Err(FigError::malformed("no targets"));
    }
    targets
        .iter()
        .map(|&t| {
            let page = doc
                .page_of(t)
                .ok_or_else(|| FigError::malformed(format!("{t} is not on a page")))?;
            layer_bounds(doc, t, page)
                .map(|b| (t, b))
                .ok_or_else(|| FigError::malformed(format!("{t} has no bounds")))
        })
        .collect()
}

/// Moves `target` by the page-space vector `(dx, dy)` (converted into its parent's space).
fn shift_in_page(
    doc: &Document,
    txn: &mut Txn,
    target: Guid,
    (dx, dy): (f64, f64),
) -> Result<(), FigError> {
    if dx.abs() < 1e-9 && dy.abs() < 1e-9 {
        return Ok(());
    }
    let node = ensure_exists(doc, target)?;
    let to_parent = node
        .parent
        .map_or(Some(Xform::IDENTITY), |p| world_transform(doc, p).inverse())
        .ok_or_else(|| FigError::malformed(format!("{target}'s parent cannot be inverted")))?;
    let (o, v) = (to_parent.apply((0.0, 0.0)), to_parent.apply((dx, dy)));
    let mut t = finite(Xform::of(&node.data), target)?;
    t.m02 += v.0 - o.0;
    t.m12 += v.1 - o.1;
    txn.set_field(target, "transform", t.to_val());
    Ok(())
}

fn align(
    doc: &Document,
    txn: &mut Txn,
    targets: &[Guid],
    edge: AlignEdge,
    to: AlignTo,
) -> Result<(), FigError> {
    let items = page_bounds(doc, targets)?;
    let parent_box = |t: Guid| -> Result<Bounds, FigError> {
        let parent = ensure_exists(doc, t)?
            .parent
            .filter(|p| doc.node(*p).is_some_and(|n| n.kind != "CANVAS"))
            .ok_or_else(|| {
                FigError::malformed(format!(
                    "{t} sits on the page: there is no frame to align to"
                ))
            })?;
        let page = doc
            .page_of(parent)
            .ok_or_else(|| FigError::malformed(format!("{parent} is not on a page")))?;
        layer_bounds(doc, parent, page)
            .ok_or_else(|| FigError::malformed(format!("{parent} has no bounds")))
    };
    let selection = items
        .iter()
        .map(|(_, b)| *b)
        .reduce(Bounds::union)
        .ok_or_else(|| FigError::malformed("no targets"))?;
    let to_parent = to == AlignTo::Parent || items.len() == 1;
    for (t, b) in &items {
        let r = if to_parent {
            parent_box(*t)?
        } else {
            selection
        };
        let d = match edge {
            AlignEdge::Left => (r.x0 - b.x0, 0.0),
            AlignEdge::Right => (r.x1 - b.x1, 0.0),
            AlignEdge::HCenter => ((r.x0 + r.x1 - b.x0 - b.x1) / 2.0, 0.0),
            AlignEdge::Top => (0.0, r.y0 - b.y0),
            AlignEdge::Bottom => (0.0, r.y1 - b.y1),
            AlignEdge::VCenter => (0.0, (r.y0 + r.y1 - b.y0 - b.y1) / 2.0),
        };
        shift_in_page(doc, txn, *t, d)?;
    }
    Ok(())
}

fn distribute(
    doc: &Document,
    txn: &mut Txn,
    targets: &[Guid],
    axis: Axis,
    spacing: Option<f64>,
) -> Result<(), FigError> {
    let mut items = page_bounds(doc, targets)?;
    let need = if spacing.is_some() { 2 } else { 3 };
    if items.len() < need {
        return Err(FigError::malformed(format!(
            "distribute needs at least {need} layers"
        )));
    }
    if spacing.is_some_and(|s| !s.is_finite()) {
        return Err(FigError::malformed("spacing must be finite"));
    }
    let horizontal = axis == Axis::Horizontal;
    let start = |b: &Bounds| if horizontal { b.x0 } else { b.y0 };
    let len = |b: &Bounds| if horizontal { b.width() } else { b.height() };
    items.sort_by(|a, b| start(&a.1).total_cmp(&start(&b.1)));
    let gap = spacing.unwrap_or_else(|| {
        let first = start(&items[0].1);
        let last = items.last().map_or(first, |(_, b)| start(b) + len(b));
        let total: f64 = items.iter().map(|(_, b)| len(b)).sum();
        (last - first - total) / (items.len() - 1) as f64
    });
    let mut cursor = start(&items[0].1);
    for (t, b) in &items {
        let d = cursor - start(b);
        let v = match axis {
            Axis::Horizontal => (d, 0.0),
            Axis::Vertical => (0.0, d),
        };
        shift_in_page(doc, txn, *t, v)?;
        cursor += len(b) + gap;
    }
    Ok(())
}
