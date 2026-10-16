//! Structural comparison of decoded Figma documents.
//!
//! This compares stored fields only, without resolving instances or rendering.

mod html;
mod properties;
mod render;
mod report;
pub use html::render_html;
pub use render::{
    ImageReader, MAX_RENDER_PAGES, MAX_RENDER_PIXELS, RenderImages, RenderOpts,
    attach_render_diffs, attach_render_diffs_with_sink, render_page_pair,
    render_page_pair_with_readers,
};
mod structure;
mod types;
pub use types::*;

use std::collections::{HashMap, HashSet};

use figstudio_fig::{Document, Guid, Node};

/// Compares stored node fields, matching identities across the whole document.
///
/// Moves appear once, on the destination page, and mark both pages changed.
/// Unchanged pages are omitted. Counts exclude page/document containers from
/// added/removed nodes; changed page fields are retained as ordinary node changes.
#[must_use]
pub fn diff_documents(old: &Document, new: &Document) -> DiffReport {
    let mut pages = structure::pages(old, new);
    let old_pages = structure::page_index(old, &pages);
    let new_pages = structure::page_index(new, &pages);
    let slots: HashMap<_, _> = pages.iter().enumerate().map(|(i, p)| (p.id, i)).collect();
    let page_ids: HashSet<_> = slots.keys().copied().collect();
    let mut summary = Summary::default();
    let mut affected = HashSet::new();
    let mut ungrouped = 0;
    for id in structure::ids(old, new) {
        let a = old.node(id);
        let b = new.node(id);
        match (a, b) {
            (Some(a), Some(b)) => {
                let moved = (a.parent != b.parent || a.position != b.position).then(|| Move {
                    old_parent: a.parent,
                    new_parent: b.parent,
                    old_position: a.position.clone(),
                    new_position: b.position.clone(),
                });
                let mut changes = properties::changes(Some(&a.data), Some(&b.data));
                if moved.is_some() {
                    changes.retain(|change| !change.field.starts_with("parentIndex"));
                }
                if changes.is_empty() && moved.is_none() {
                    continue;
                }
                affected.extend(old_pages.get(&id).copied());
                affected.extend(new_pages.get(&id).copied());
                if moved.is_some()
                    && old_pages.get(&id) != new_pages.get(&id)
                    && let Some(&slot) = old_pages.get(&id).and_then(|page| slots.get(page))
                {
                    pages[slot].moved_out.push(id);
                }
                summary.nodes_changed += 1;
                summary.nodes_moved += usize::from(moved.is_some());
                summary.properties_changed += changes.len();
                let page = new_pages.get(&id).or_else(|| old_pages.get(&id));
                let Some(&slot) = page.and_then(|p| slots.get(p)) else {
                    ungrouped += 1;
                    continue;
                };
                pages[slot].changed.push(NodeDiff {
                    id,
                    kind: b.kind.clone(),
                    name: b.name.clone(),
                    path: structure::path(new, b),
                    moved,
                    text: properties::text_change(&a.data, &b.data, &changes),
                    changes,
                });
            }
            (None, Some(node)) => {
                if page_ids.contains(&id) {
                    continue;
                }
                summary.nodes_added += 1;
                if let Some(&page) = new_pages.get(&id) {
                    affected.insert(page);
                    if subtree_root(node, old, &page_ids)
                        && let Some(&slot) = slots.get(&page)
                    {
                        pages[slot].added.push(structure::reference(new, node));
                    }
                } else {
                    ungrouped += 1;
                }
            }
            (Some(node), None) => {
                if page_ids.contains(&id) {
                    continue;
                }
                summary.nodes_removed += 1;
                if let Some(&page) = old_pages.get(&id) {
                    affected.insert(page);
                    if subtree_root(node, new, &page_ids)
                        && let Some(&slot) = slots.get(&page)
                    {
                        pages[slot].removed.push(structure::reference(old, node));
                    }
                } else {
                    ungrouped += 1;
                }
            }
            (None, None) => {}
        }
    }
    for page in &mut pages {
        if page.status == PageStatus::Unchanged && affected.contains(&page.id) {
            page.status = PageStatus::Changed;
        }
        match page.status {
            PageStatus::Added => summary.pages_added += 1,
            PageStatus::Removed => summary.pages_removed += 1,
            PageStatus::Changed => summary.pages_changed += 1,
            PageStatus::Unchanged => {}
        }
        let a = structure::order(old, page.id);
        let b = structure::order(new, page.id);
        let rank = |id: Guid, order: &HashMap<Guid, usize>| {
            (order.get(&id).copied().unwrap_or(usize::MAX), id)
        };
        page.added.sort_by_key(|n| rank(n.id, &b));
        page.removed.sort_by_key(|n| rank(n.id, &a));
        page.changed.sort_by_key(|n| rank(n.id, &b));
        page.moved_out.sort_by_key(|&id| rank(id, &a));
    }
    pages.retain(|p| p.status != PageStatus::Unchanged);
    DiffReport {
        old: structure::meta(old),
        new: structure::meta(new),
        summary,
        pages,
        complete: ungrouped == 0,
        truncated: if ungrouped == 0 {
            Vec::new()
        } else {
            vec![format!(
                "{ungrouped} added, removed or changed nodes outside pages are not grouped"
            )]
        },
    }
}

fn subtree_root(node: &Node, other: &Document, pages: &HashSet<Guid>) -> bool {
    node.parent
        .is_none_or(|p| pages.contains(&p) || other.node(p).is_some())
}
