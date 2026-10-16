use std::collections::{BTreeSet, HashMap, HashSet};

use figstudio_fig::{Document, Guid, Node};

use crate::{FileMeta, NodeRef, PageDiff, PageStatus};

pub(crate) fn meta(doc: &Document) -> FileMeta {
    FileMeta {
        path_label: String::new(),
        format_version: doc.format_version,
        page_count: doc.pages.len() + doc.internal_pages.len(),
        node_count: doc.nodes.len(),
    }
}

pub(crate) fn pages(old: &Document, new: &Document) -> Vec<PageDiff> {
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for doc in [old, new] {
        for &id in doc.pages.iter().chain(&doc.internal_pages) {
            if seen.insert(id) {
                ids.push(id);
            }
        }
    }
    ids.into_iter()
        .map(|id| {
            let a = old.node(id);
            let b = new.node(id);
            PageDiff {
                id,
                old_name: a.map(|n| n.name.clone()),
                new_name: b.map(|n| n.name.clone()),
                status: match (a, b) {
                    (None, _) => PageStatus::Added,
                    (_, None) => PageStatus::Removed,
                    _ => PageStatus::Unchanged,
                },
                added: Vec::new(),
                removed: Vec::new(),
                changed: Vec::new(),
                moved_out: Vec::new(),
                render: None,
            }
        })
        .collect()
}

/// Iterative preorder, using the decoder's position/Guid-sorted sibling lists.
pub(crate) fn page_index(doc: &Document, pages: &[PageDiff]) -> HashMap<Guid, Guid> {
    let mut out = HashMap::new();
    for page in pages {
        let mut stack = vec![page.id];
        while let Some(id) = stack.pop() {
            if doc.node(id).is_none() || out.insert(id, page.id).is_some() {
                continue;
            }
            stack.extend(doc.children(id).iter().rev());
        }
    }
    out
}

pub(crate) fn order(doc: &Document, page: Guid) -> HashMap<Guid, usize> {
    let mut out = HashMap::new();
    let mut stack = vec![page];
    while let Some(id) = stack.pop() {
        if out.contains_key(&id) {
            continue;
        }
        out.insert(id, out.len());
        stack.extend(doc.children(id).iter().rev());
    }
    out
}

pub(crate) fn path(doc: &Document, node: &Node) -> Vec<String> {
    let mut names = Vec::new();
    let mut parent = node.parent;
    let mut seen = HashSet::from([node.id]);
    while let Some(id) = parent {
        if !seen.insert(id) {
            break;
        }
        let Some(ancestor) = doc.node(id) else {
            break;
        };
        names.push(ancestor.name.clone());
        parent = ancestor.parent;
    }
    names.reverse();
    names
}

pub(crate) fn reference(doc: &Document, node: &Node) -> NodeRef {
    NodeRef {
        id: node.id,
        kind: node.kind.clone(),
        name: node.name.clone(),
        path: path(doc, node),
        descendants: doc.descendant_count(node.id),
    }
}

pub(crate) fn ids(old: &Document, new: &Document) -> BTreeSet<Guid> {
    old.nodes.iter().chain(&new.nodes).map(|n| n.id).collect()
}
