//! Original image-fill bytes inside a node, found by paint hash.
use crate::node_id::{NodeId, parse_node_id};
use figstudio_fig::{
    Document, Guid, Val, hex,
    resolve::{
        InstancePath, MAX_RESOLVED_NODES, Resolved, definitions, resolve_instance, resolve_path,
        tree_path, walk_path,
    },
};
use std::{collections::HashSet, sync::Arc};

/// Paint lists that can carry an image fill.
const PAINT_KEYS: [&str; 2] = ["fillPaints", "strokePaints"];

fn shown(data: &Val) -> bool {
    data.flag("visible") != Some(false)
}

/// Hashes of visible image paints in `data`, in paint order.
fn hashes(data: &Val, out: &mut Vec<String>) {
    for key in PAINT_KEYS {
        for paint in data.get(key).map_or(&[][..], Val::items) {
            if paint.text("type") != Some("IMAGE") || paint.flag("visible") == Some(false) {
                continue;
            }
            if let Some(hash) = paint.path(&["image", "hash"]).and_then(Val::as_bytes) {
                let hash = hex(hash);
                if !out.contains(&hash) {
                    out.push(hash);
                }
            }
        }
    }
}

/// Hashes of the nodes of a resolved walk that are drawn: a hidden node (an override
/// included) hides everything below it.
fn walk_hashes(nodes: &[Resolved], out: &mut Vec<String>) {
    let mut hidden_at: Option<usize> = None;
    for n in nodes {
        if hidden_at.is_some_and(|level| n.level > level) {
            continue;
        }
        hidden_at = (!shown(&n.data)).then_some(n.level);
        if hidden_at.is_none() {
            hashes(&n.data, out);
        }
    }
}

/// Whether `id` and every ancestor of it is visible.
fn visible_chain(doc: &Document, id: Guid) -> bool {
    let mut seen = HashSet::new();
    let mut cur = Some(id);
    while let Some(node) = cur.and_then(|c| doc.node(c)) {
        if !shown(&node.data) {
            return false;
        }
        // A parent loop in a malformed file ends the walk.
        if !seen.insert(node.id) {
            break;
        }
        cur = node.parent;
    }
    true
}

/// Image hashes drawn by `node_id` and everything inside it (instances included), without
/// repeats. Hidden nodes, and nodes below hidden ancestors, draw nothing. `Err` is the message for a node that cannot be read.
pub(super) fn image_hashes(doc: &Document, node_id: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    match parse_node_id(node_id).map_err(|e| e.to_string())? {
        NodeId::Node(root) => {
            if doc.node(root).is_none() {
                return Err(format!("node {root} does not exist in this file"));
            }
            if !visible_chain(doc, root) {
                return Ok(out);
            }
            let mut stack: Vec<Guid> = vec![root];
            while let Some(id) = stack.pop() {
                let Some(node) = doc.node(id) else { continue };
                if !shown(&node.data) {
                    continue;
                }
                hashes(&node.data, &mut out);
                if node.kind == "INSTANCE"
                    && let Ok(walk) = resolve_instance(doc, id)
                {
                    walk_hashes(&walk.nodes, &mut out);
                }
                // Children in reverse so the pre-order matches the document order.
                stack.extend(doc.children(id).iter().rev());
            }
        }
        NodeId::InstancePath(path) => {
            let defs = Arc::new(definitions(doc));
            let walk = walk_path(doc, &defs, &path, None, MAX_RESOLVED_NODES)
                .map_err(|e| format!("{path}: {e}"))?;
            // shortcut: frames between nested instances are judged on their stored visibility, not
            // on overrides; upgrade if a file hides only those by override.
            // The instance, its document ancestors and every nested instance on the way.
            let keys = path.keys();
            let nested_hidden = (0..keys.len() - 1).any(|n| {
                let mut prefix = vec![path.instance()];
                prefix.extend_from_slice(&keys[..=n]);
                InstancePath::new(prefix)
                    .and_then(|p| resolve_path(doc, &p).ok())
                    .is_some_and(|r| !shown(&r.data))
            });
            let frames_hidden = !visible_chain(doc, path.instance())
                || tree_path(doc, &path).is_ok_and(|chain| {
                    // The target itself is judged on its resolved data by `walk_hashes`.
                    chain[..chain.len().saturating_sub(1)]
                        .iter()
                        .any(|g| doc.node(*g).is_some_and(|n| !shown(&n.data)))
                });
            if nested_hidden || frames_hidden {
                return Ok(out);
            }
            walk_hashes(&walk.nodes, &mut out);
        }
    }
    Ok(out)
}
