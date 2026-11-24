//! Groups: Figma stores a group as a `FRAME` with `resizeToFit: true`, whose box always fits
//! its children. After an edit, [`fit_groups`] refits every group (and boolean) above a touched
//! node without moving anything on the page, and recomputes the booleans' results.

use std::collections::HashSet;

use crate::edit::Txn;
use crate::edit::space::{Bounds, Xform, bounds_in_parent};
use crate::{Document, FigError, Guid, Node, Obj, Val};

/// Whether `node` is a group (a resize-to-fit frame; `GROUP` from older editor builds too).
pub fn is_group(node: &Node) -> bool {
    node.kind == "GROUP" || (node.kind == "FRAME" && node.data.flag("resizeToFit") == Some(true))
}

/// Refits the groups above `touched` in `staged`, deepest first, applying each refit to
/// `staged` and appending it to `out`. Returns the refitted groups.
pub fn fit_groups(
    staged: &mut Document,
    touched: &[Guid],
    out: &mut Txn,
) -> Result<Vec<Guid>, FigError> {
    // Groups above each touched node, with their depth, so inner groups are fitted first.
    let mut groups: Vec<(usize, Guid)> = Vec::new();
    let mut seen = HashSet::new();
    for &start in touched {
        let mut chain = Vec::new();
        let mut walked = HashSet::new();
        let mut cur = staged.node(start).map(|n| n.id);
        while let Some(id) = cur {
            if !walked.insert(id) {
                break;
            }
            let Some(node) = staged.node(id) else { break };
            chain.push((id, is_group(node) || node.kind == "BOOLEAN_OPERATION"));
            cur = node.parent;
        }
        let depth = chain.len();
        for (i, (id, group)) in chain.into_iter().enumerate() {
            if group && seen.insert(id) {
                groups.push((depth - i, id));
            }
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.0));

    let mut fitted = Vec::new();
    for (_, group) in groups {
        let mut txn = Txn::new("fit group");
        if refit(staged, group, &mut txn) {
            staged.apply_txn(&txn)?;
            crate::edit::ops::merge_step_into_batch(out, txn);
            fitted.push(group);
        }
        // A boolean's result follows its operands (computed after its box is fitted).
        if staged
            .node(group)
            .is_some_and(|n| n.kind == "BOOLEAN_OPERATION")
        {
            let mut txn = Txn::new("boolean result");
            crate::edit::vector_ops::refresh_boolean(staged, &mut txn, group);
            if !txn.is_empty() {
                staged.apply_txn(&txn)?;
                crate::edit::ops::merge_step_into_batch(out, txn);
                fitted.push(group);
            }
        }
    }
    Ok(fitted)
}

/// Writes the patches that fit `group` to its children into `txn`; `false` when it already fits
/// (or has no children, which Figma would not keep but this pass leaves alone).
fn refit(doc: &Document, group: Guid, txn: &mut Txn) -> bool {
    let Some(node) = doc.node(group) else {
        return false;
    };
    let kids = doc.children(group);
    let Some(u) = kids
        .iter()
        .filter_map(|&k| doc.node(k).and_then(|n| bounds_in_parent(&n.data)))
        .reduce(Bounds::union)
    else {
        return false;
    };
    let (w, h) = crate::edit::space::size(&node.data);
    // Sizes are stored as f32: a looser tolerance keeps fitted groups from being rewritten.
    let eps = 1e-2;
    if u.x0.abs() < eps
        && u.y0.abs() < eps
        && (u.width() - w).abs() < eps
        && (u.height() - h).abs() < eps
    {
        return false;
    }
    // Move the group's origin to the union's corner and the children back by the same amount,
    // so every child keeps its page position.
    let shift = Xform::translate(u.x0, u.y0);
    let back = Xform::translate(-u.x0, -u.y0);
    let mut set = Obj::new();
    set.insert("transform", Xform::of(&node.data).compose(shift).to_val());
    set.insert(
        "size",
        Val::Obj(Obj::from([
            ("x", Val::Float(u.width())),
            ("y", Val::Float(u.height())),
        ])),
    );
    txn.modify(group, set, Vec::new());
    for &k in kids {
        if let Some(child) = doc.node(k) {
            txn.set_field(
                k,
                "transform",
                back.compose(Xform::of(&child.data)).to_val(),
            );
        }
    }
    true
}
