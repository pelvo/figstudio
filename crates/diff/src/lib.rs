//! Structural comparison of decoded Figma documents.
//!
//! This compares stored fields only, without resolving instances or rendering.

mod html;
mod html_changes;
mod html_component;
mod html_kind;
mod html_lines;
mod html_page;
mod html_pictures;
mod html_screen;
mod html_shared;
mod html_swatch;
mod properties;
mod regions;
mod render;
mod render_crops;
mod render_files;
pub use render_files::{RenderLinks, file_label, render_report_to_dir, write_render_images};
mod render_pair;
mod render_pixels;
mod render_screens;
mod report;
mod screen_bounds;
mod screen_model;
mod screen_types;
pub use html::render_html;
pub use regions::{MAX_REGIONS, find_regions};
pub use render::{
    DEFAULT_IMAGE_CACHE_BYTES, ImageReader, MAX_RENDER_PAGES, MAX_RENDER_PIXELS, RegionImages,
    RenderImages, RenderOpts, SHARED_IMAGES_ID, ScreenImages, attach_render_diffs,
    attach_render_diffs_with_sink, render_page_pair, render_page_pair_with_readers,
};
mod screen_limits;
pub use screen_limits::{
    MAX_DOCUMENT_SCREEN_PIXELS, MAX_PAGE_PICTURE_BYTES, MAX_PAGE_SCREENS, ScreenLimits,
};
pub use screen_types::{MatchedBy, ScreenDiff, ScreenStatus};
mod render_shared;
#[cfg(test)]
mod render_tests;
pub use render_shared::{MAX_COMPONENT_VIEW_PIXELS, MAX_COMPONENT_VIEWS};
mod screens;
mod shared;
mod shared_component;
mod shared_types;
pub use shared_types::{SharedChange, SharedComponent, SharedOccurrence, SharedSet, SharedVariant};
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
                    && let Some(dest) = new_pages.get(&id)
                    && old_pages.get(&id) != Some(dest)
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
        structure::sort_rows((old, new), page);
    }
    pages.retain(|p| p.status != PageStatus::Unchanged);
    let mut copies = screen_model::CopyBudget::default();
    let mut notes = Vec::new();
    for page in &mut pages {
        // Layers of recreated screens compared by name; their new ids still count as added.
        let uncompared = copies.uncompared;
        let (rows, props) = screen_model::prepare((old, new), page, &mut copies);
        summary.nodes_changed += rows;
        summary.properties_changed += props;
        if copies.uncompared > uncompared {
            notes.push(format!(
                "page {}: copy comparison limit reached; {} repeated layers on recreated \
                 screens paired in order without listing their edits",
                page.id,
                copies.uncompared - uncompared
            ));
        }
    }
    DiffReport {
        old: structure::meta(old),
        new: structure::meta(new),
        summary,
        pages,
        shared_changes: Vec::new(),
        complete: ungrouped == 0,
        truncated: {
            if ungrouped > 0 {
                notes.insert(
                    0,
                    format!(
                        "{ungrouped} added, removed or changed nodes outside pages are not grouped"
                    ),
                );
            }
            notes
        },
    }
}

fn subtree_root(node: &Node, other: &Document, pages: &HashSet<Guid>) -> bool {
    node.parent
        .is_none_or(|p| pages.contains(&p) || other.node(p).is_some())
}
