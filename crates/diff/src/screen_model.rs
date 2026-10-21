//! Structural screen ownership also works for reports made without rendering.
use crate::{
    PageDiff, ScreenDiff, ScreenStatus, screen_bounds,
    screens::{MatchBy, ScreenPair, match_screens},
};
use figstudio_fig::Document;
use figstudio_render::absolute_transform;
use std::collections::BTreeSet;

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
    let lines = crate::html_changes::page_lines(page);
    let mut owned = BTreeSet::new();
    for pair in match_screens(docs.0, docs.1, page.id) {
        let nodes = screen_bounds::members(page, docs, pair);
        owned.extend(nodes.iter().copied());
        let real = lines.items.iter().any(|l| {
            nodes.contains(&l.id) && (pair.matched_by != Some(MatchBy::NameSize) || l.sign == "~")
        });
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
