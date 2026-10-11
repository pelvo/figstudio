//! Original image-fill bytes inside a node, found by paint hash.
use crate::node_id::{NodeId, parse_node_id};
use figstudio_fig::{
    Document, Guid, Val, hex,
    resolve::{MAX_RESOLVED_NODES, definitions, resolve_instance, walk_path},
};
use std::sync::Arc;

/// Paint lists that can carry an image fill.
const PAINT_KEYS: [&str; 2] = ["fillPaints", "strokePaints"];

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

/// Image hashes drawn by `node_id` and everything inside it (instances included), without
/// repeats. `Err` is the message for a node that cannot be read.
pub(super) fn image_hashes(doc: &Document, node_id: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    match parse_node_id(node_id).map_err(|e| e.to_string())? {
        NodeId::Node(root) => {
            if doc.node(root).is_none() {
                return Err(format!("node {root} does not exist in this file"));
            }
            let mut stack: Vec<Guid> = vec![root];
            while let Some(id) = stack.pop() {
                let Some(node) = doc.node(id) else { continue };
                hashes(&node.data, &mut out);
                if node.kind == "INSTANCE"
                    && let Ok(walk) = resolve_instance(doc, id)
                {
                    walk.nodes.iter().for_each(|n| hashes(&n.data, &mut out));
                }
                // Children in reverse so the pre-order matches the document order.
                stack.extend(doc.children(id).iter().rev());
            }
        }
        NodeId::InstancePath(path) => {
            let defs = Arc::new(definitions(doc));
            let walk = walk_path(doc, &defs, &path, None, MAX_RESOLVED_NODES)
                .map_err(|e| format!("{path}: {e}"))?;
            walk.nodes.iter().for_each(|n| hashes(&n.data, &mut out));
        }
    }
    Ok(out)
}
