//! Two-way Figma canvas clipboard (`text/html` `figmeta` + `fig-kiwi` container).
//!
//! Supports copying any selection of nodes out of `figstudio` so it pastes straight into
//! Figma (`⌘V` or `⌘⌥A` + `⌘⇧R`), and pasting Figma clipboard HTML or raw `fig-kiwi` bytes
//! into an open [`Document`] as an undoable [`Txn`].

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use kiwi_schema::{DefKind, Schema};
use serde::{Deserialize, Serialize};

use crate::doc::Source;
use crate::edit::{GuidAlloc, Txn, fractional};
use crate::{Codec, Document, FigError, Guid, Node, Obj, Val};

/// Metadata carried in `<!--(figmeta)...(/figmeta)-->`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FigMeta {
    #[serde(rename = "fileKey", default)]
    pub file_key: String,
    #[serde(rename = "pasteID", default)]
    pub paste_id: i32,
    #[serde(rename = "dataType", default = "default_data_type")]
    pub data_type: String,
}

fn default_data_type() -> String {
    "scene".into()
}

/// Decoded Figma clipboard payload (`figmeta` JSON + raw `fig-kiwi` binary).
#[derive(Clone, Debug)]
pub struct ClipboardPayload {
    pub meta: FigMeta,
    pub kiwi_bytes: Vec<u8>,
}

/// Encodes `roots` and all of their transitive local dependencies (`SYMBOL` masters, styles,
/// variables, and referenced `blobs`) as a standalone `fig-kiwi` container.
pub fn encode_subset_kiwi(
    doc: &Document,
    roots: &[Guid],
    paste_id: i32,
    file_key: &str,
) -> Result<Vec<u8>, FigError> {
    if roots.is_empty() {
        return Err(FigError::malformed("cannot copy an empty selection"));
    }
    for &r in roots {
        if doc.node(r).is_none() {
            return Err(FigError::malformed(format!("missing root node {r}")));
        }
    }

    // 1. Collect full subtrees of selected roots + transitive local dependencies.
    let mut selected_set = HashSet::new();
    let mut ordered_scene = Vec::new();
    for &r in roots {
        collect_subtree_preorder(doc, r, &mut selected_set, &mut ordered_scene);
    }

    let mut internal_roots = Vec::new();
    let mut visited_deps = HashSet::new();
    let mut dep_queue: VecDeque<Guid> = ordered_scene.iter().copied().collect();

    // Build quick lookup from asset `(key, version)` or `guid` for local symbols/styles/vars.
    while let Some(nid) = dep_queue.pop_front() {
        let Some(node) = doc.node(nid) else {
            continue;
        };
        let mut refs = Vec::new();
        collect_guid_references(&node.data, &mut refs);
        for target_id in refs {
            if selected_set.contains(&target_id) || !visited_deps.insert(target_id) {
                continue;
            }
            let Some(target) = doc.node(target_id) else {
                continue;
            };
            // Include local SYMBOL masters (or enclosing STATE_GROUP), styles, and variables.
            let dep_root = if target.kind == "SYMBOL" {
                if let Some(p) = target.parent
                    && doc.node(p).is_some_and(|pn| {
                        pn.kind == "SYMBOL" || pn.data.flag("isStateGroup") == Some(true)
                    })
                {
                    p
                } else {
                    target.id
                }
            } else if target.data.get("styleType").is_some()
                || target.kind == "VARIABLE"
                || target.kind == "VARIABLE_SET"
            {
                target.id
            } else {
                continue;
            };

            if !selected_set.contains(&dep_root) && !internal_roots.contains(&dep_root) {
                internal_roots.push(dep_root);
                let mut dep_sub = Vec::new();
                collect_subtree_preorder(doc, dep_root, &mut HashSet::new(), &mut dep_sub);
                for sub_id in dep_sub {
                    dep_queue.push_back(sub_id);
                }
            }
        }
    }

    // 2. Build synthetic DOCUMENT (0:0), visible CANVAS (0:1), and optional internal CANVAS (0:2).
    let doc_guid = Guid(0, 0);
    let canvas_guid = Guid(0, 1);
    let internal_canvas_guid = Guid(0, 2);

    let mut out_nodes: Vec<Node> = Vec::new();
    let mut doc_obj = Obj::new();
    doc_obj.insert("guid", doc_guid.to_val());
    doc_obj.insert("type", Val::Enum(crate::intern("DOCUMENT")));
    doc_obj.insert("name", Val::Str("Document".into()));
    doc_obj.insert("phase", Val::Enum(crate::intern("CREATED")));
    out_nodes.push(Node::from_data(Val::Obj(doc_obj)).expect("DOCUMENT node"));

    let mut canvas_obj = Obj::new();
    canvas_obj.insert("guid", canvas_guid.to_val());
    canvas_obj.insert("type", Val::Enum(crate::intern("CANVAS")));
    canvas_obj.insert("name", Val::Str("Page 1".into()));
    canvas_obj.insert("phase", Val::Enum(crate::intern("CREATED")));
    canvas_obj.insert(
        "parentIndex",
        Val::Obj(Obj::from([
            ("guid", doc_guid.to_val()),
            ("position", Val::Str("!".into())),
        ])),
    );
    out_nodes.push(Node::from_data(Val::Obj(canvas_obj)).expect("CANVAS node"));

    if !internal_roots.is_empty() {
        let mut ic_obj = Obj::new();
        ic_obj.insert("guid", internal_canvas_guid.to_val());
        ic_obj.insert("type", Val::Enum(crate::intern("CANVAS")));
        ic_obj.insert("name", Val::Str("Internal Only Canvas".into()));
        ic_obj.insert("internalOnly", Val::Bool(true));
        ic_obj.insert("phase", Val::Enum(crate::intern("CREATED")));
        ic_obj.insert(
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", doc_guid.to_val()),
                ("position", Val::Str("~".into())),
            ])),
        );
        out_nodes.push(Node::from_data(Val::Obj(ic_obj)).expect("internal CANVAS node"));
    }

    // 3. Deduplicate & renumber all `*Blob` indices across included nodes.
    let mut new_blobs: Vec<Vec<u8>> = Vec::new();
    let mut blob_remap: HashMap<usize, usize> = HashMap::new();

    let internal_positions = fractional::n_between(None, None, internal_roots.len());
    let mut seen_internal = HashSet::new();
    for (idx, &iroot) in internal_roots.iter().enumerate() {
        let mut sub_ids = Vec::new();
        collect_subtree_preorder(doc, iroot, &mut seen_internal, &mut sub_ids);
        for nid in sub_ids {
            let Some(orig) = doc.node(nid) else { continue };
            let mut data = orig.data.clone().into_owned();
            remap_blobs_in_val(&mut data, &doc.blobs, &mut new_blobs, &mut blob_remap);
            if nid == iroot {
                data.set(
                    "parentIndex",
                    Val::Obj(Obj::from([
                        ("guid", internal_canvas_guid.to_val()),
                        ("position", Val::Str(internal_positions[idx].clone())),
                    ])),
                );
            }
            if let Some(n) = Node::from_data(data) {
                out_nodes.push(n);
            }
        }
    }

    let root_positions = fractional::n_between(None, None, roots.len());
    let root_pos_map: HashMap<Guid, &str> = roots
        .iter()
        .enumerate()
        .map(|(i, &g)| (g, root_positions[i].as_str()))
        .collect();

    for nid in ordered_scene {
        let Some(orig) = doc.node(nid) else { continue };
        let mut data = orig.data.clone().into_owned();
        remap_blobs_in_val(&mut data, &doc.blobs, &mut new_blobs, &mut blob_remap);
        if let Some(&pos) = root_pos_map.get(&nid) {
            data.set(
                "parentIndex",
                Val::Obj(Obj::from([
                    ("guid", canvas_guid.to_val()),
                    ("position", Val::Str(pos.to_string())),
                ])),
            );
        }
        if let Some(n) = Node::from_data(data) {
            out_nodes.push(n);
        }
    }

    let schema = doc.decoded_schema()?;
    let mut message_extra = BTreeMap::new();
    if let Some(msg_def) = schema.def("Message") {
        if msg_def.field("type").is_some() {
            message_extra.insert("type".into(), Val::Enum(crate::intern("NODE_CHANGES")));
        }
        if msg_def.field("sessionID").is_some() {
            message_extra.insert("sessionID".into(), Val::Int(0));
        }
        if msg_def.field("ackID").is_some() {
            message_extra.insert("ackID".into(), Val::Int(0));
        }
        if msg_def.field("pasteID").is_some() {
            message_extra.insert("pasteID".into(), Val::Int(i64::from(paste_id)));
        }
        if msg_def.field("pasteFileKey").is_some() && !file_key.is_empty() {
            message_extra.insert("pasteFileKey".into(), Val::Str(file_key.to_string()));
        }
    }

    let mut sub_doc = Document {
        instance: 0,
        image_instance: 0,
        revision: 0,
        prelude: "fig-kiwi".into(),
        format_version: doc.format_version,
        file_name: doc.file_name.clone(),
        nodes: Vec::new(),
        index: HashMap::new(),
        children: HashMap::new(),
        root: None,
        pages: Vec::new(),
        internal_pages: Vec::new(),
        blobs: new_blobs,
        images: HashMap::new(),
        lazy_images: HashMap::new(),
        thumbnail: None,
        source: Source {
            schema: doc.source.schema.clone(),
            codecs: vec![Codec::Deflate, Codec::Deflate],
            extra_chunks: Vec::new(),
            message_extra,
            had_blobs: true,
            archive_entries: None,
            lazy: false,
            decoder: None,
            cached_schema: std::sync::OnceLock::new(),
        },
    };
    sub_doc.set_nodes(out_nodes)?;
    sub_doc.encode_canvas()
}

/// Encodes `roots` into the exact `text/html` clipboard string Figma reads on `⌘V` / `⌘⇧R`.
pub fn encode_clipboard_html(
    doc: &Document,
    roots: &[Guid],
    paste_id: i32,
    file_key: Option<&str>,
) -> Result<String, FigError> {
    let key = file_key
        .map(str::to_owned)
        .or_else(|| {
            doc.source
                .message_extra
                .get("pasteFileKey")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "figstudio".into());

    let kiwi = encode_subset_kiwi(doc, roots, paste_id, &key)?;
    let meta = FigMeta {
        file_key: key,
        paste_id,
        data_type: "scene".into(),
    };
    let meta_json = serde_json::to_vec(&meta)
        .map_err(|e| FigError::malformed(format!("cannot serialize figmeta: {e}")))?;
    let meta_b64 = B64.encode(&meta_json);
    let kiwi_b64 = B64.encode(&kiwi);

    let fallback_label: Vec<&str> = roots
        .iter()
        .filter_map(|&id| doc.node(id).map(|n| n.name.as_str()))
        .filter(|s| !s.is_empty())
        .collect();
    let plain = html_escape_min(&fallback_label.join("\n"));

    Ok(format!(
        "<meta charset=\"utf-8\"><span data-metadata=\"<!--(figmeta){meta_b64}(/figmeta)-->\"></span><span data-buffer=\"<!--(figma){kiwi_b64}(/figma)-->\"></span><span style=\"white-space:pre-wrap;\">{plain}</span>"
    ))
}

/// Decodes a Figma HTML clipboard string (or raw `fig-kiwi` bytes if passed as base64/text).
pub fn decode_clipboard_html(html: &str) -> Result<ClipboardPayload, FigError> {
    let unescaped = html
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");

    let kiwi_b64 = extract_marker(&unescaped, "(figma)", "(/figma)")
        .ok_or_else(|| FigError::malformed("clipboard HTML has no (figma) data-buffer"))?;
    let kiwi_bytes = B64
        .decode(kiwi_b64.trim())
        .map_err(|e| FigError::malformed(format!("invalid base64 in (figma) buffer: {e}")))?;

    let meta = if let Some(meta_b64) = extract_marker(&unescaped, "(figmeta)", "(/figmeta)") {
        let meta_bytes = B64
            .decode(meta_b64.trim())
            .map_err(|e| FigError::malformed(format!("invalid base64 in (figmeta): {e}")))?;
        serde_json::from_slice(&meta_bytes)
            .map_err(|e| FigError::malformed(format!("invalid figmeta JSON: {e}")))?
    } else {
        FigMeta::default()
    };

    Ok(ClipboardPayload { meta, kiwi_bytes })
}

/// Pastes a decoded [`ClipboardPayload`] (or raw `fig-kiwi` bytes via [`Document::from_bytes`])
/// into `target_doc` under `target_parent` (a `CANVAS`, `FRAME`, `SECTION`, or `GROUP`),
/// shifting top-level pasted roots by `offset = (dx, dy)`.
///
/// Returns `(txn, pasted_root_guids)` ready to execute via [`History::execute`](crate::edit::History::execute).
pub fn paste_kiwi_into(
    target_doc: &Document,
    kiwi_bytes: &[u8],
    target_parent: Guid,
    offset: (f64, f64),
) -> Result<(Txn, Vec<Guid>), FigError> {
    if target_doc.node(target_parent).is_none() {
        return Err(FigError::malformed(format!(
            "target parent {target_parent} does not exist"
        )));
    }
    let src_doc = Document::from_bytes(kiwi_bytes)?;
    let target_schema = target_doc.decoded_schema()?;

    let mut alloc = GuidAlloc::for_document(target_doc);
    let mut txn = Txn::new("Paste from Figma clipboard");

    // Append source blobs at `blob_base`.
    let blob_base = target_doc.blobs.len();
    txn.add_blobs.clone_from(&src_doc.blobs);

    // Map existing local symbols/styles by `key` so repeated pastes reuse existing masters!
    let mut existing_by_key: HashMap<(String, String), Guid> = HashMap::new();
    for n in &target_doc.nodes {
        if let Some(key) = n.data.text("key")
            && !key.is_empty()
        {
            existing_by_key.insert((n.kind.clone(), key.to_string()), n.id);
        }
    }

    let mut guid_map: HashMap<Guid, Guid> = HashMap::new();
    let mut skip_subtrees: HashSet<Guid> = HashSet::new();

    // First pass: map internal canvas dependencies (`SYMBOL`, styles, variables).
    let mut internal_nodes_to_copy = Vec::new();
    for &ic in &src_doc.internal_pages {
        for &dep_root in src_doc.children(ic) {
            let Some(dep_node) = src_doc.node(dep_root) else {
                continue;
            };
            if let Some(key) = dep_node.data.text("key")
                && let Some(&existing_id) =
                    existing_by_key.get(&(dep_node.kind.clone(), key.to_string()))
            {
                guid_map.insert(dep_root, existing_id);
                collect_subtree_set(&src_doc, dep_root, &mut skip_subtrees);
                continue;
            }
            let mut sub = Vec::new();
            collect_subtree_preorder(&src_doc, dep_root, &mut HashSet::new(), &mut sub);
            for sid in sub {
                guid_map.insert(sid, alloc.next_guid());
                internal_nodes_to_copy.push((sid, sid == dep_root));
            }
        }
    }

    // Collect visible scene roots from source pages (or non-DOCUMENT/non-CANVAS orphans).
    let mut src_scene_roots = Vec::new();
    if !src_doc.pages.is_empty() {
        for &page in &src_doc.pages {
            src_scene_roots.extend_from_slice(src_doc.children(page));
        }
    } else {
        for n in &src_doc.nodes {
            if n.kind != "DOCUMENT"
                && n.kind != "CANVAS"
                && n.parent.is_none_or(|p| {
                    src_doc
                        .node(p)
                        .is_none_or(|pn| pn.kind == "DOCUMENT" || pn.kind == "CANVAS")
                })
            {
                src_scene_roots.push(n.id);
            }
        }
    }

    let mut scene_nodes_to_copy = Vec::new();
    for &sroot in &src_scene_roots {
        if skip_subtrees.contains(&sroot) {
            continue;
        }
        let mut sub = Vec::new();
        collect_subtree_preorder(&src_doc, sroot, &mut HashSet::new(), &mut sub);
        for sid in sub {
            guid_map.insert(sid, alloc.next_guid());
            scene_nodes_to_copy.push((sid, sid == sroot));
        }
    }

    // Ensure target document has an internal canvas if we need to paste internal dependencies.
    let target_internal_canvas = if !internal_nodes_to_copy.is_empty() {
        if let Some(&ic) = target_doc.internal_pages.first() {
            Some(ic)
        } else if let Some(root_id) = target_doc.root {
            let ic_id = alloc.next_guid();
            let last_pos = target_doc
                .children(root_id)
                .last()
                .and_then(|id| target_doc.node(*id))
                .map(|n| n.position.as_str());
            let pos = fractional::between(last_pos, None);
            let mut ic_obj = Obj::new();
            ic_obj.insert("guid", ic_id.to_val());
            ic_obj.insert("type", Val::Enum(crate::intern("CANVAS")));
            ic_obj.insert("name", Val::Str("Internal Only Canvas".into()));
            ic_obj.insert("internalOnly", Val::Bool(true));
            ic_obj.insert(
                "parentIndex",
                Val::Obj(Obj::from([
                    ("guid", root_id.to_val()),
                    ("position", Val::Str(pos)),
                ])),
            );
            let filtered = filter_node_to_schema(&target_schema, Val::Obj(ic_obj));
            txn.create_node(ic_id, filtered);
            Some(ic_id)
        } else {
            None
        }
    } else {
        None
    };

    if let Some(ic_target) = target_internal_canvas {
        let last_ic_pos = target_doc
            .children(ic_target)
            .last()
            .and_then(|id| target_doc.node(*id))
            .map(|n| n.position.as_str());
        let ic_root_count = internal_nodes_to_copy
            .iter()
            .filter(|(_, is_r)| *is_r)
            .count();
        let ic_positions = fractional::n_between(last_ic_pos, None, ic_root_count);
        let mut ic_pos_idx = 0;

        for (old_id, is_root) in internal_nodes_to_copy {
            let Some(orig) = src_doc.node(old_id) else {
                continue;
            };
            let new_id = guid_map[&old_id];
            let mut data = orig.data.clone().into_owned();
            offset_blobs_in_val(&mut data, blob_base);
            remap_guids_in_val(&mut data, &guid_map);
            data.set("guid", new_id.to_val());
            if is_root {
                data.set(
                    "parentIndex",
                    Val::Obj(Obj::from([
                        ("guid", ic_target.to_val()),
                        ("position", Val::Str(ic_positions[ic_pos_idx].clone())),
                    ])),
                );
                ic_pos_idx += 1;
            }
            let filtered = filter_node_to_schema(&target_schema, data);
            txn.create_node(new_id, filtered);
        }
    }

    let last_target_pos = target_doc
        .children(target_parent)
        .last()
        .and_then(|id| target_doc.node(*id))
        .map(|n| n.position.as_str());
    let scene_root_count = scene_nodes_to_copy.iter().filter(|(_, is_r)| *is_r).count();
    let root_positions = fractional::n_between(last_target_pos, None, scene_root_count);
    let mut root_pos_idx = 0;
    let mut pasted_roots = Vec::with_capacity(scene_root_count);

    for (old_id, is_root) in scene_nodes_to_copy {
        let Some(orig) = src_doc.node(old_id) else {
            continue;
        };
        let new_id = guid_map[&old_id];
        let mut data = orig.data.clone().into_owned();
        offset_blobs_in_val(&mut data, blob_base);
        remap_guids_in_val(&mut data, &guid_map);
        data.set("guid", new_id.to_val());
        if is_root {
            data.set(
                "parentIndex",
                Val::Obj(Obj::from([
                    ("guid", target_parent.to_val()),
                    ("position", Val::Str(root_positions[root_pos_idx].clone())),
                ])),
            );
            root_pos_idx += 1;
            if offset.0 != 0.0 || offset.1 != 0.0 {
                shift_transform(&mut data, offset.0, offset.1);
            }
            pasted_roots.push(new_id);
        }
        let filtered = filter_node_to_schema(&target_schema, data);
        txn.create_node(new_id, filtered);
    }

    Ok((txn, pasted_roots))
}

fn collect_subtree_preorder(
    doc: &Document,
    root: Guid,
    seen: &mut HashSet<Guid>,
    out: &mut Vec<Guid>,
) {
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        out.push(id);
        for &child in doc.children(id).iter().rev() {
            stack.push(child);
        }
    }
}

fn collect_subtree_set(doc: &Document, root: Guid, set: &mut HashSet<Guid>) {
    let mut dummy = Vec::new();
    collect_subtree_preorder(doc, root, set, &mut dummy);
}

fn collect_guid_references(val: &Val, out: &mut Vec<Guid>) {
    match val.view() {
        Val::Guid(g) => out.push(*g),
        Val::Obj(o) => {
            if let Some(g) = val.as_guid() {
                out.push(g);
                return;
            }
            for (k, v) in o.iter() {
                if k == "guid" || k == "parentIndex" {
                    continue;
                }
                collect_guid_references(v, out);
            }
        }
        Val::Arr(items) => {
            for item in items {
                collect_guid_references(item, out);
            }
        }
        _ => {}
    }
}

fn remap_blobs_in_val(
    val: &mut Val,
    src_blobs: &[Vec<u8>],
    new_blobs: &mut Vec<Vec<u8>>,
    remap: &mut HashMap<usize, usize>,
) {
    val.settle();
    match val {
        Val::Obj(o) => {
            let keys: Vec<&'static str> = o.keys().collect();
            for k in keys {
                let Some(fv) = o.get_mut(k) else { continue };
                if k.ends_with("Blob")
                    && let Val::Int(old_idx) = fv
                    && let Ok(u) = usize::try_from(*old_idx)
                    && let Some(bytes) = src_blobs.get(u)
                {
                    let mapped = *remap.entry(u).or_insert_with(|| {
                        let idx = new_blobs.len();
                        new_blobs.push(bytes.clone());
                        idx
                    });
                    *fv = Val::Int(mapped as i64);
                } else {
                    remap_blobs_in_val(fv, src_blobs, new_blobs, remap);
                }
            }
        }
        Val::Arr(items) => {
            for item in items {
                remap_blobs_in_val(item, src_blobs, new_blobs, remap);
            }
        }
        _ => {}
    }
}

fn offset_blobs_in_val(val: &mut Val, blob_base: usize) {
    if blob_base == 0 {
        return;
    }
    val.settle();
    match val {
        Val::Obj(o) => {
            let keys: Vec<&'static str> = o.keys().collect();
            for k in keys {
                let Some(fv) = o.get_mut(k) else { continue };
                if k.ends_with("Blob")
                    && let Val::Int(idx) = fv
                    && *idx >= 0
                {
                    *fv = Val::Int(*idx + blob_base as i64);
                } else {
                    offset_blobs_in_val(fv, blob_base);
                }
            }
        }
        Val::Arr(items) => {
            for item in items {
                offset_blobs_in_val(item, blob_base);
            }
        }
        _ => {}
    }
}

fn remap_guids_in_val(val: &mut Val, map: &HashMap<Guid, Guid>) {
    val.settle();
    match val {
        Val::Guid(g) => {
            if let Some(&new_g) = map.get(g) {
                *g = new_g;
            }
        }
        Val::Obj(o) => {
            if let Some(g) = o
                .get("sessionID")
                .zip(o.get("localID"))
                .and_then(|(s, l)| Some(Guid(s.as_u32()?, l.as_u32()?)))
            {
                if let Some(&new_g) = map.get(&g) {
                    *val = new_g.to_val();
                }
                return;
            }
            let keys: Vec<&'static str> = o.keys().collect();
            for k in keys {
                if let Some(fv) = o.get_mut(k) {
                    remap_guids_in_val(fv, map);
                }
            }
        }
        Val::Arr(items) => {
            for item in items {
                remap_guids_in_val(item, map);
            }
        }
        _ => {}
    }
}

fn shift_transform(data: &mut Val, dx: f64, dy: f64) {
    if let Some(Val::Obj(tf)) = data.get_mut("transform") {
        for (k, delta) in [("m02", dx), ("m12", dy)] {
            let cur = tf.get(k).and_then(Val::as_f32).map_or(0.0, f64::from);
            tf.insert(k, Val::Float(cur + delta));
        }
    }
}

/// Strips any unknown fields or unknown enum variants not supported by `schema` so clipboard
/// payloads from newer Figma schemas paste cleanly into older documents.
fn filter_node_to_schema(schema: &Schema, data: Val) -> Val {
    let Some(nc_idx) = schema
        .defs
        .iter()
        .position(|d| d.name == "NodeChange" && d.kind == DefKind::Message)
    else {
        return data;
    };
    filter_val_by_type(schema, nc_idx as i32, data).unwrap_or_else(|| Val::Obj(Obj::new()))
}

fn filter_val_by_type(schema: &Schema, type_id: i32, mut val: Val) -> Option<Val> {
    val.settle();
    if type_id < 0 {
        return Some(val);
    }
    let def = schema.defs.get(usize::try_from(type_id).ok()?)?;
    match def.kind {
        DefKind::Enum => {
            if let Val::Enum(v) = val {
                def.field(v).map(|_| Val::Enum(v))
            } else {
                None
            }
        }
        DefKind::Struct if matches!(val, Val::Guid(_)) => Some(val),
        DefKind::Struct => {
            let Val::Obj(mut o) = val else { return None };
            let mut out = Obj::with_capacity(def.fields.len());
            for f in &def.fields {
                let fv = o.remove(&f.name)?;
                let cleaned = filter_field_val(schema, f, fv)?;
                out.insert(&f.name, cleaned);
            }
            Some(Val::Obj(out))
        }
        DefKind::Message => {
            let Val::Obj(mut o) = val else { return None };
            let keys: Vec<&'static str> = o.keys().collect();
            let mut out = Obj::with_capacity(keys.len());
            for k in keys {
                let Some(f) = def.field(k) else { continue };
                let Some(fv) = o.remove(k) else { continue };
                if let Some(cleaned) = filter_field_val(schema, f, fv) {
                    out.insert(k, cleaned);
                }
            }
            Some(Val::Obj(out))
        }
    }
}

fn filter_field_val(schema: &Schema, f: &kiwi_schema::Field, mut fv: Val) -> Option<Val> {
    fv.settle();
    if !f.is_array {
        return filter_val_by_type(schema, f.type_id, fv);
    }
    match fv {
        Val::Bytes(b) if f.type_id == kiwi_schema::TYPE_BYTE => Some(Val::Bytes(b)),
        Val::Arr(items) => {
            let cleaned: Vec<Val> = items
                .into_iter()
                .filter_map(|it| filter_val_by_type(schema, f.type_id, it))
                .collect();
            Some(Val::Arr(cleaned))
        }
        _ => None,
    }
}

fn extract_marker<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let rest = &s[i..];
    let j = rest.find(end)?;
    Some(&rest[..j])
}

fn html_escape_min(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}
