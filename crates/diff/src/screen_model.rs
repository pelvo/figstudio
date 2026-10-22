//! Structural screen ownership also works for reports made without rendering.
use crate::{
    PageDiff, ScreenDiff, ScreenStatus, screen_bounds,
    screens::{MatchBy, ScreenPair, match_screens},
};
use figstudio_fig::{Document, Guid};
use figstudio_render::absolute_transform;
use std::collections::{BTreeMap, BTreeSet};

/// Whether a change line counts for a screen. A screen recreated with new ids (matched by name
/// and size) has a `-` line for its old root and a `+` line for its new one: identity churn,
/// not a change. Every other line counts, including layers it gained or lost.
pub(crate) fn counts(
    line: &crate::html_lines::ChangeLine,
    matched_by: Option<MatchBy>,
    roots: (Option<Guid>, Option<Guid>),
) -> bool {
    matched_by != Some(MatchBy::NameSize)
        || line.sign == "~"
        || (Some(line.id) != roots.0 && Some(line.id) != roots.1)
}

/// Where a one-sided layer sits: the nearest layer both sides kept (`None` for the screen
/// root), then the name/kind path below it. The kept layer's id keeps same-named kept
/// layers apart.
type Path = (Option<Guid>, Vec<(String, String)>);

/// The layers only one side of the screen has, keyed by their [`Path`].
fn one_sided(doc: &Document, other: &Document, root: Guid) -> Vec<(Path, Guid)> {
    let mut out = Vec::new();
    let mut stack: Vec<(Guid, Path)> = doc
        .children(root)
        .iter()
        .rev()
        .map(|&id| (id, (None, Vec::new())))
        .collect();
    while let Some((id, mut key)) = stack.pop() {
        let Some(node) = doc.node(id) else { continue };
        if other.node(id).is_some() {
            key = (Some(id), Vec::new());
        } else {
            key.1.push((node.name.clone(), node.kind.clone()));
            out.push((key.clone(), id));
        }
        stack.extend(doc.children(id).iter().rev().map(|&c| (c, key.clone())));
    }
    out
}

/// A recreated screen's whole subtree has new ids, but only the new root is listed as added
/// (and the old root as removed). Layers that pair up by name and kind path are the same
/// churn; the rest were really added or removed, so their subtree roots are listed too.
/// Returns the change rows and property changes it added, for the report summary.
fn list_recreated_changes(
    docs: (&Document, &Document),
    page: &mut PageDiff,
    pair: ScreenPair,
) -> (usize, usize) {
    let (Some(old_root), Some(new_root)) = (pair.old, pair.new) else {
        return (0, 0);
    };
    let old = one_sided(docs.0, docs.1, old_root);
    let new = one_sided(docs.1, docs.0, new_root);
    let pairs = pair_up(docs, &old, &new);
    let paired: [BTreeSet<Guid>; 2] = [
        pairs.iter().map(|p| p.0).collect(),
        pairs.iter().map(|p| p.1).collect(),
    ];
    let edits = list_paired_edits(docs, page, (old_root, new_root), &pairs);
    for (doc, side, root, old_side) in [
        (docs.0, &old, old_root, true),
        (docs.1, &new, new_root, false),
    ] {
        let paired = &paired[usize::from(!old_side)];
        let ids: BTreeSet<Guid> = side
            .iter()
            .map(|(_, id)| *id)
            .filter(|id| !paired.contains(id))
            .collect();
        let list = if old_side {
            &mut page.removed
        } else {
            &mut page.added
        };
        // A layer that kept its id may already list a recreated child that paired: churn too.
        list.retain(|n| !side.iter().any(|(_, id)| *id == n.id) || ids.contains(&n.id));
        for (_, id) in side {
            let Some(node) = doc.node(*id) else { continue };
            let parent_listed = node.parent.is_some_and(|p| p != root && ids.contains(&p));
            if ids.contains(id) && !parent_listed && !list.iter().any(|n| n.id == *id) {
                list.push(crate::structure::reference(doc, node));
            }
        }
    }
    edits
}

/// Copies of one path compared pairwise at most; beyond this they pair in walk order.
const MAX_COPY_COMPARISONS: usize = 4096;

/// Pairs the one-sided layers that share a [`Path`], as `(old, new, compared)` in the new
/// side's walk order. Copies pair greedily by fewest changed fields, ties in walk order: an
/// inserted copy that differs from the rest is the one left over, but identical copies pair
/// from the front. A path with too many copies to compare pairs in walk order and is not
/// `compared`.
fn pair_up(
    docs: (&Document, &Document),
    old: &[(Path, Guid)],
    new: &[(Path, Guid)],
) -> Vec<(Guid, Guid, bool)> {
    let mut sides: BTreeMap<&Path, (Vec<Guid>, Vec<Guid>)> = BTreeMap::new();
    for (key, id) in old {
        sides.entry(key).or_default().0.push(*id);
    }
    for (key, id) in new {
        sides.entry(key).or_default().1.push(*id);
    }
    let mut pairs = Vec::new();
    for (a, b) in sides.values() {
        if a.len() * b.len() > MAX_COPY_COMPARISONS {
            pairs.extend(a.iter().zip(b).map(|(x, y)| (*x, *y, false)));
            continue;
        }
        let mut costs = Vec::with_capacity(a.len() * b.len());
        for (i, x) in a.iter().enumerate() {
            for (j, y) in b.iter().enumerate() {
                costs.push((edits(docs, (*x, *y), false).len(), i, j));
            }
        }
        costs.sort_unstable();
        let (mut used_a, mut used_b) = (vec![false; a.len()], vec![false; b.len()]);
        for (_, i, j) in costs {
            if !used_a[i] && !used_b[j] {
                used_a[i] = true;
                used_b[j] = true;
                pairs.push((a[i], b[j], true));
            }
        }
    }
    let order: BTreeMap<Guid, usize> = new
        .iter()
        .enumerate()
        .map(|(i, (_, id))| (*id, i))
        .collect();
    pairs.sort_by_key(|p| order[&p.1]);
    pairs
}

/// The property changes between two paired layers that count as edits. Ids always differ,
/// and sibling positions are regenerated with the new ids. The root's own transform is
/// where the screen sits on the canvas, so it shows only as a screen move; unlike a screen
/// that kept its id, it gets no transform row.
fn edits(
    docs: (&Document, &Document),
    (old, new): (Guid, Guid),
    root: bool,
) -> Vec<crate::PropertyChange> {
    let (Some(a), Some(b)) = (docs.0.node(old), docs.1.node(new)) else {
        return Vec::new();
    };
    let mut changes = crate::properties::changes(Some(&a.data), Some(&b.data));
    changes.retain(|c| {
        let head = c.field.split('.').next().unwrap_or(&c.field);
        !matches!(head, "guid" | "parentIndex" | "overrideKey") && !(root && head == "transform")
    });
    changes
}

/// Layers of a recreated screen that pair by [`Path`] have new ids, so the structural diff
/// never compares them. Their own edits (a new colour, new text) are listed as changes of
/// the new layer, and so are the screen root's, in tree order. Returns the rows and property
/// changes added.
fn list_paired_edits(
    docs: (&Document, &Document),
    page: &mut PageDiff,
    roots: (Guid, Guid),
    pairs: &[(Guid, Guid, bool)],
) -> (usize, usize) {
    let mut added = (0, 0);
    let compared = pairs.iter().filter(|p| p.2).map(|p| (p.0, p.1, false));
    for (old_id, new_id, root) in std::iter::once((roots.0, roots.1, true)).chain(compared) {
        let changes = edits(docs, (old_id, new_id), root);
        let (Some(a), Some(b)) = (docs.0.node(old_id), docs.1.node(new_id)) else {
            continue;
        };
        if changes.is_empty() || page.changed.iter().any(|n| n.id == b.id) {
            continue;
        }
        added = (added.0 + 1, added.1 + changes.len());
        page.changed.push(crate::NodeDiff {
            id: b.id,
            kind: b.kind.clone(),
            name: b.name.clone(),
            path: crate::structure::path(docs.1, b),
            moved: None,
            text: crate::properties::text_change(&a.data, &b.data, &changes),
            changes,
        });
    }
    added
}

pub(crate) fn moved(docs: (&Document, &Document), pair: ScreenPair) -> bool {
    match (
        pair.old.and_then(|id| docs.0.node(id)),
        pair.new.and_then(|id| docs.1.node(id)),
    ) {
        (Some(a), Some(b)) => {
            a.parent != b.parent
                || absolute_transform(docs.0, a.id) != absolute_transform(docs.1, b.id)
        }
        _ => false,
    }
}

/// Returns the change rows and property changes added for recreated screens.
pub(crate) fn prepare(docs: (&Document, &Document), page: &mut PageDiff) -> (usize, usize) {
    let pairs = match_screens(docs.0, docs.1, page.id);
    let mut edits = (0, 0);
    for pair in pairs
        .iter()
        .filter(|p| p.matched_by == Some(MatchBy::NameSize))
    {
        let (rows, props) = list_recreated_changes(docs, page, *pair);
        edits = (edits.0 + rows, edits.1 + props);
    }
    let lines = crate::html_changes::page_lines(page);
    let mut owned = BTreeSet::new();
    for pair in pairs {
        let nodes = screen_bounds::members(page, docs, pair);
        owned.extend(nodes.iter().copied());
        let real = lines
            .items
            .iter()
            .any(|l| nodes.contains(&l.id) && counts(l, pair.matched_by, (pair.old, pair.new)));
        let moved = moved(docs, pair);
        if pair.old.is_some() && pair.new.is_some() && !real {
            page.screens_unchanged += 1;
            page.screens_moved += usize::from(moved);
            continue;
        }
        let Some(node) = pair
            .new
            .and_then(|id| docs.1.node(id))
            .or_else(|| pair.old.and_then(|id| docs.0.node(id)))
        else {
            continue;
        };
        page.screens.push(ScreenDiff {
            old_id: pair.old,
            new_id: pair.new,
            name: node.name.clone(),
            kind: node.kind.clone(),
            status: match (pair.old, pair.new) {
                (None, _) => ScreenStatus::Added,
                (_, None) => ScreenStatus::Removed,
                _ => ScreenStatus::Changed,
            },
            matched_by: pair.matched_by,
            moved,
            width: 0,
            height: 0,
            pixels_changed: None,
            thumb_old: None,
            thumb_new: None,
            regions: Vec::new(),
            nodes,
        });
    }
    page.other_nodes = Some(
        page.added
            .iter()
            .map(|n| n.id)
            .chain(page.removed.iter().map(|n| n.id))
            .chain(page.changed.iter().map(|n| n.id))
            .filter(|id| !owned.contains(id))
            .collect(),
    );
    edits
}
