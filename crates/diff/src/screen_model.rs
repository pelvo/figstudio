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
fn list_recreated_changes(docs: (&Document, &Document), page: &mut PageDiff, pair: ScreenPair) {
    let (Some(old_root), Some(new_root)) = (pair.old, pair.new) else {
        return;
    };
    let old = one_sided(docs.0, docs.1, old_root);
    let new = one_sided(docs.1, docs.0, new_root);
    let mut tally: BTreeMap<&Path, (usize, usize)> = BTreeMap::new();
    for (key, _) in &old {
        tally.entry(key).or_default().0 += 1;
    }
    for (key, _) in &new {
        tally.entry(key).or_default().1 += 1;
    }
    // With n copies on one side and m on the other, the first min(n, m) pair up.
    let unpaired = |side: &[(Path, Guid)], old_side: bool| {
        let mut seen: BTreeMap<&Path, usize> = BTreeMap::new();
        let mut out = BTreeSet::new();
        for (key, id) in side {
            let (n, m) = tally[key];
            let rank = seen.entry(key).or_default();
            *rank += 1;
            if *rank > if old_side { m } else { n } {
                out.insert(*id);
            }
        }
        out
    };
    for (doc, side, root, old_side) in [
        (docs.0, &old, old_root, true),
        (docs.1, &new, new_root, false),
    ] {
        let ids = unpaired(side, old_side);
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

pub(crate) fn prepare(docs: (&Document, &Document), page: &mut PageDiff) {
    let pairs = match_screens(docs.0, docs.1, page.id);
    for pair in pairs
        .iter()
        .filter(|p| p.matched_by == Some(MatchBy::NameSize))
    {
        list_recreated_changes(docs, page, *pair);
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
}
