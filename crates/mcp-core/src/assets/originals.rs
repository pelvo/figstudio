//! Original image-fill bytes inside a node, found by paint hash.
use super::{Budget, IMAGE_CAP_LABEL, MAX_IMAGE_BYTES, MAX_ORIGINALS, PAYLOAD_LABEL};
use crate::{
    node_id::{NodeId, parse_node_id},
    output::Block,
    rendering::Rendering,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::{
    Document, Guid, Val, hex,
    resolve::{
        InstancePath, MAX_RESOLVED_NODES, Resolved, definitions, resolve_instance, resolve_path,
        tree_path, walk_path,
    },
};
use std::{borrow::Cow, collections::HashSet, sync::Arc};

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

/// The original files behind the image fills of `id` that this call has not sent yet, as
/// image or blob items. Fails when the node has no image fills or none has stored bytes.
/// Images of a lazily opened document are read from the file only once they fit the budget.
pub(super) fn original_images(
    doc: &Document,
    id: &str,
    rendering: &Rendering,
    budget: &mut Budget,
) -> Result<Vec<Block>, String> {
    let hashes = image_hashes(doc, id)?;
    if hashes.is_empty() {
        return Err("no image fills inside this node".into());
    }
    let mut content = vec![];
    let (mut missing, mut omitted, mut repeated) = (0, 0, 0);
    let mut too_large = Vec::new();
    for hash in &hashes {
        if budget.sent.contains(hash) {
            repeated += 1;
            continue;
        }
        let bytes = if let Some(bytes) = doc.images.get(hash) {
            Cow::Borrowed(bytes.as_slice())
        } else if let Some(entry) = doc.lazy_images.get(hash) {
            // The directory's size, checked before reading so a huge image is never loaded.
            let size = usize::try_from(entry.size).unwrap_or(usize::MAX);
            if size > MAX_IMAGE_BYTES {
                too_large.push(format!(
                    "{id}: image {hash} omitted, {size} bytes is over the {IMAGE_CAP_LABEL} per-image limit"
                ));
                continue;
            }
            if budget.originals == 0 || !budget.payload.fits(size) {
                omitted += 1;
                continue;
            }
            match rendering.read(entry) {
                Some(bytes) => Cow::Owned(bytes),
                None => {
                    missing += 1;
                    continue;
                }
            }
        } else {
            missing += 1;
            continue;
        };
        if bytes.len() > MAX_IMAGE_BYTES {
            too_large.push(format!(
                "{id}: image {hash} omitted, {} bytes is over the {IMAGE_CAP_LABEL} per-image limit",
                bytes.len()
            ));
            continue;
        }
        if budget.originals == 0 || !budget.payload.take(bytes.len()) {
            omitted += 1;
            continue;
        }
        budget.originals -= 1;
        budget.sent.insert(hash.clone());
        let data = STANDARD.encode(&bytes);
        let mime = image::guess_format(&bytes).ok().map(|f| f.to_mime_type());
        content.push(match mime {
            Some(mime) => Block::image(data, mime),
            None => Block::Blob {
                data_b64: data,
                uri: format!("figma-image://{hash}"),
                mime: "application/octet-stream".into(),
            },
        });
        content.push(Block::text(format!(
            "{id}: image {hash} ({} bytes, {})",
            bytes.len(),
            mime.unwrap_or("unknown type")
        )));
    }
    if missing == hashes.len() {
        return Err(format!("{missing} image(s) not stored in the file"));
    }
    for line in too_large {
        content.push(Block::text(line));
    }
    if missing + omitted + repeated > 0 {
        content.push(Block::text(format!(
            "{id}: {missing} image(s) not stored in the file, {omitted} omitted (limit {MAX_ORIGINALS} images and {PAYLOAD_LABEL} per call), {repeated} already returned for another node"
        )));
    }
    Ok(content)
}
