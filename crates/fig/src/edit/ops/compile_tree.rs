//! Structural and transform [`EditOp`] compilation (create, delete, duplicate, move, resize,
//! rotate, reparent) with iterative post-order subtree traversal.

use std::collections::HashSet;

use crate::clipboard::paste_kiwi_into_with;
use crate::edit::geometry::{
    CreateShapeParams, create_shape_node, matrix_2x3, resize_node_with_constraints,
};
use crate::edit::text_runs::create_text_node;
use crate::edit::{GuidAlloc, Txn, fractional};
use crate::{Document, FigError, Guid, Obj, Val, intern};

use super::EditOp;
use super::batch::merge_step_into_batch;
use super::compile_props::compile_prop_op;

pub(super) fn compile_single_op(
    doc: &Document,
    txn: &mut Txn,
    alloc: &mut GuidAlloc,
    op: &EditOp,
    created_ids: &mut Vec<Guid>,
    layout_touched: &mut Vec<Guid>,
) -> Result<(), FigError> {
    match op {
        EditOp::CreateShape {
            id,
            kind,
            parent,
            name,
            x,
            y,
            width,
            height,
            corner_radius,
            fill_hex,
            count,
            star_inner_scale,
        } => {
            let gid = id.unwrap_or_else(|| alloc.next_guid());
            create_shape_node(
                doc,
                txn,
                gid,
                &CreateShapeParams {
                    kind: kind.clone(),
                    parent: *parent,
                    name: name.clone(),
                    x: *x,
                    y: *y,
                    width: *width,
                    height: *height,
                    corner_radius: *corner_radius,
                    fill_hex: fill_hex.clone(),
                    count: *count,
                    star_inner_scale: *star_inner_scale,
                },
            )?;
            created_ids.push(gid);
            layout_touched.push(*parent);
        }
        EditOp::CreateText {
            id,
            parent,
            x,
            y,
            characters,
            font_family,
            font_style,
            font_size,
            fill_hex,
        } => {
            let gid = id.unwrap_or_else(|| alloc.next_guid());
            create_text_node(
                doc,
                txn,
                gid,
                *parent,
                *x,
                *y,
                characters,
                font_family,
                font_style,
                *font_size,
                fill_hex,
            )?;
            created_ids.push(gid);
            layout_touched.push(*parent);
        }
        EditOp::CreatePage { id, name } => {
            let root = doc
                .root
                .ok_or_else(|| FigError::malformed("document has no root"))?;
            let gid = id.unwrap_or_else(|| alloc.next_guid());
            let last_pos = doc
                .pages
                .last()
                .and_then(|pid| doc.node(*pid))
                .map(|n| n.position.as_str());
            let pos = fractional::between(last_pos, None);
            txn.create_node(
                gid,
                Val::Obj(Obj::from([
                    ("guid", gid.to_val()),
                    ("phase", Val::Enum(intern("CREATED"))),
                    ("type", Val::Enum(intern("CANVAS"))),
                    ("name", Val::Str(name.clone())),
                    ("visible", Val::Bool(true)),
                    (
                        "parentIndex",
                        Val::Obj(Obj::from([
                            ("guid", root.to_val()),
                            ("position", Val::Str(pos)),
                        ])),
                    ),
                ])),
            );
            created_ids.push(gid);
        }
        EditOp::DeleteNode { target } => {
            let Some(node) = doc.node(*target) else {
                return Err(FigError::malformed(format!("missing node {target}")));
            };
            if let Some(p) = node.parent {
                layout_touched.push(p);
            }
            // Delete descendants bottom-up then `target` iteratively (bounded against deep chains).
            for id in collect_postorder_iterative(doc, *target) {
                txn.delete_node(id);
            }
        }
        EditOp::DuplicateNode { target, dx, dy } => {
            let Some(orig) = doc.node(*target) else {
                return Err(FigError::malformed(format!("missing node {target}")));
            };
            let parent = orig
                .parent
                .ok_or_else(|| FigError::malformed("cannot duplicate root node"))?;
            let kiwi = crate::clipboard::encode_subset_kiwi(doc, &[*target], 0, "dup")?;
            let (paste_txn, roots) = paste_kiwi_into_with(doc, &kiwi, parent, (*dx, *dy), alloc)?;
            created_ids.extend_from_slice(&roots);
            merge_step_into_batch(txn, paste_txn);
            layout_touched.push(parent);
        }
        EditOp::Rename { target, name } => {
            ensure_exists(doc, *target)?;
            txn.set_field(*target, "name", Val::Str(name.clone()));
        }
        EditOp::SetLayerProps {
            target,
            visible,
            locked,
            opacity,
            blend_mode,
            clip_content,
        } => {
            let node = ensure_exists(doc, *target)?;
            let mut set = Obj::new();
            if let Some(v) = visible {
                set.insert("visible", Val::Bool(*v));
                if let Some(p) = node.parent {
                    layout_touched.push(p);
                }
            }
            if let Some(l) = locked {
                set.insert("locked", Val::Bool(*l));
            }
            if let Some(op) = opacity {
                set.insert("opacity", Val::Float(op.clamp(0.0, 1.0)));
            }
            if let Some(bm) = blend_mode {
                set.insert("blendMode", Val::Enum(intern(bm)));
            }
            if let Some(clip) = clip_content {
                set.insert("frameMaskDisabled", Val::Bool(!*clip));
            }
            txn.modify(*target, set, Vec::new());
        }
        EditOp::Move {
            target,
            x,
            y,
            relative,
        } => {
            let node = ensure_exists(doc, *target)?;
            let mut tf = node
                .data
                .get("transform")
                .cloned()
                .unwrap_or_else(|| matrix_2x3(0.0, 0.0, 0.0))
                .into_owned();
            let (nx, ny) = if *relative {
                let cx = tf.num("m02").map_or(0.0, f64::from);
                let cy = tf.num("m12").map_or(0.0, f64::from);
                (cx + *x, cy + *y)
            } else {
                (*x, *y)
            };
            tf.set("m02", Val::Float(nx));
            tf.set("m12", Val::Float(ny));
            txn.set_field(*target, "transform", tf);
            // An Auto-Layout parent places its flow children, so it is re-solved.
            layout_touched.push(*target);
        }
        EditOp::Resize {
            target,
            width,
            height,
        } => {
            resize_node_with_constraints(doc, txn, *target, *width, *height)?;
            layout_touched.push(*target);
        }
        EditOp::Rotate {
            target,
            degrees,
            pivot,
        } => {
            let node = ensure_exists(doc, *target)?;
            txn.set_field(
                *target,
                "transform",
                super::compile_arrange::rotated(&node.data, *degrees, *pivot),
            );
            layout_touched.push(*target);
        }
        EditOp::Reparent {
            target,
            new_parent,
            index,
        } => {
            let node = ensure_exists(doc, *target)?;
            ensure_exists(doc, *new_parent)?;
            if let Some(old_p) = node.parent {
                layout_touched.push(old_p);
            }
            layout_touched.push(*new_parent);
            let others = doc
                .children(*new_parent)
                .iter()
                .filter(|&&id| id != *target)
                .count();
            super::compile_arrange::place(doc, txn, *target, *new_parent, index.unwrap_or(others))?;
        }
        EditOp::Group { .. }
        | EditOp::FrameSelection { .. }
        | EditOp::Ungroup { .. }
        | EditOp::Align { .. }
        | EditOp::Distribute { .. }
        | EditOp::Flip { .. }
        | EditOp::Reorder { .. } => super::compile_arrange::compile_arrange_op(
            doc,
            txn,
            alloc,
            op,
            created_ids,
            layout_touched,
        )?,
        other => compile_prop_op(doc, txn, alloc, other, created_ids, layout_touched)?,
    }
    Ok(())
}

pub(super) fn ensure_exists(doc: &Document, id: Guid) -> Result<&crate::Node, FigError> {
    doc.node(id)
        .ok_or_else(|| FigError::malformed(format!("node {id} does not exist")))
}

/// Iterative post-order traversal (children before parent) so deep hierarchies never overflow the
/// call stack (M-PANIC / Ground Rule 5).
fn collect_postorder_iterative(doc: &Document, root: Guid) -> Vec<Guid> {
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    let mut stack: Vec<(Guid, bool)> = vec![(root, false)];
    while let Some((id, expanded)) = stack.pop() {
        if expanded {
            out.push(id);
            continue;
        }
        if !visited.insert(id) {
            continue;
        }
        stack.push((id, true));
        for &child in doc.children(id).iter().rev() {
            if !visited.contains(&child) {
                stack.push((child, false));
            }
        }
    }
    out
}
