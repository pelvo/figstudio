//! `get_code_connect_suggestions`: the components a selection uses that have no local Code
//! Connect mapping yet.
//!
//! Figma's tool asks its cloud which published components are connected. Offline, "connected"
//! means a mapping in the registered file's sidecar (`FILE.fig.codeconnect.json`), and every
//! local component counts, published or not. A variant counts as connected when it or its
//! component set is mapped (the lookup order of `get_code_connect_map`); unconnected variants
//! are suggested once, as their component set.
use crate::{
    node_id::{NodeId, parse_node_id},
    output::Output,
};
use figstudio_fig::{Document, Guid, Val, resolve};
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

use crate::codeconnect_context::component_set;

/// Most components one reply lists.
pub const MAX_SUGGESTIONS: usize = 100;
/// Most nodes the walk looks at, instance content included.
const MAX_VISITED: usize = 100_000;
/// The reply when the selection uses no component.
pub const NONE_FOUND: &str = "No components found in this selection.";
/// The reply when every component is already mapped (Figma's wording).
pub const ALL_CONNECTED: &str =
    "All component instances in this selection are already connected to code via Code Connect.";
const PROMPT: &str = "For each component, find the code component it matches: search the \
codebase for its name and compare the code props with its properties. Show the matches to the \
user and let them choose. Then call send_code_connect_mappings with the accepted ones as \
{nodeId: mainComponentNodeId, componentName, source, label}. Use get_context_for_code_connect \
for full properties and variants, and get_screenshot for a picture.";

/// Components used under `node_id` that `is_mapped` (given a component or component set ID)
/// does not report as connected. `exclude_prompt` leaves out properties and the guidance text.
pub fn suggestions(
    doc: &Document,
    node_id: &str,
    is_mapped: impl Fn(Guid) -> bool,
    exclude_prompt: bool,
) -> Output {
    let used = match used_components(doc, node_id) {
        Ok(used) => used,
        Err(message) => return Output::fault(message),
    };
    if used.components.is_empty() {
        return Output::structured(json!({"components": [], "message": NONE_FOUND}));
    }
    // Group unconnected components by what Code Connect maps: the set for a variant.
    let mut order: Vec<Guid> = Vec::new();
    let mut counts: HashMap<Guid, usize> = HashMap::new();
    for (component, instances) in &used.components {
        let set = component_set(doc, *component).map(|s| s.id);
        if is_mapped(*component) || set.is_some_and(&is_mapped) {
            continue;
        }
        let key = set.unwrap_or(*component);
        if !counts.contains_key(&key) {
            order.push(key);
        }
        *counts.entry(key).or_default() += instances;
    }
    if order.is_empty() {
        return Output::structured(json!({"components": [], "message": ALL_CONNECTED}));
    }
    let total = order.len();
    let components: Vec<Value> = order
        .iter()
        .take(MAX_SUGGESTIONS)
        .filter_map(|id| {
            let node = doc.node(*id)?;
            let is_set =
                node.kind == "COMPONENT_SET" || node.data.flag("isStateGroup") == Some(true);
            let mut entry = json!({
                "mainComponentNodeId": id.to_string(),
                "name": node.name,
                "type": if is_set { "COMPONENT_SET" } else { "COMPONENT" },
                "instanceCount": counts[id],
                "description": node.data.text("description"),
                "published": node.data.get("componentKey").is_some()
                    || node.data.get("publishID").is_some(),
                "fromLibrary": node.data.get("sourceLibraryKey").is_some(),
            });
            if !exclude_prompt {
                entry["properties"] = properties(node.data.get("componentPropDefs"));
                if is_set {
                    entry["variantCount"] = json!(
                        doc.children(*id)
                            .iter()
                            .filter(|c| doc.node(**c).is_some_and(|n| n.kind == "SYMBOL"))
                            .count()
                    );
                }
            }
            Some(entry)
        })
        .collect();
    let mut out = json!({
        "components": components,
        "total": total,
        "truncated": total > MAX_SUGGESTIONS || used.truncated,
    });
    if used.missing > 0 {
        out["missingComponents"] = json!(used.missing);
    }
    if !exclude_prompt {
        out["prompt"] = json!(PROMPT);
    }
    Output::structured(out)
}

/// Property names and types of `componentPropDefs`.
fn properties(defs: Option<&Val>) -> Value {
    defs.map_or(&[][..], Val::items)
        .iter()
        .take(100)
        .filter_map(|d| Some(json!({"name": d.text("name")?, "type": d.text("type")})))
        .collect()
}

/// Components used under a node, in walk order, with how many instances use each.
struct Used {
    components: Vec<(Guid, usize)>,
    /// Instances whose component is not in the file.
    missing: usize,
    truncated: bool,
}

impl Used {
    fn add(&mut self, doc: &Document, component: Guid, instance: bool) {
        if doc.node(component).is_none() {
            self.missing += 1;
            return;
        }
        let n = usize::from(instance);
        match self.components.iter_mut().find(|(id, _)| *id == component) {
            Some(entry) => entry.1 += n,
            None => self.components.push((component, n)),
        }
    }
}

fn used_components(doc: &Document, node_id: &str) -> Result<Used, String> {
    let mut used = Used {
        components: Vec::new(),
        missing: 0,
        truncated: false,
    };
    let definitions = Arc::new(resolve::definitions(doc));
    let resolved = |used: &mut Used, nodes: &[resolve::Resolved]| {
        for r in nodes {
            if r.data.text("type") == Some("INSTANCE")
                && let Some(component) = resolve::symbol_id(&r.data)
            {
                used.add(doc, component, true);
            }
        }
    };
    match parse_node_id(node_id).map_err(|e| e.to_string())? {
        NodeId::Node(id) => {
            let Some(node) = doc.node(id) else {
                return Err(format!("node {id} does not exist in this file"));
            };
            if node.kind == "DOCUMENT" {
                return Err("pick a page or a layer, not the document".into());
            }
            let mut stack = vec![id];
            let mut visited = 0;
            while let Some(id) = stack.pop() {
                visited += 1;
                if visited > MAX_VISITED {
                    used.truncated = true;
                    break;
                }
                let Some(node) = doc.node(id) else { continue };
                match node.kind.as_str() {
                    "SYMBOL" | "COMPONENT_SET" => used.add(doc, id, false),
                    _ if node.data.flag("isStateGroup") == Some(true) => used.add(doc, id, false),
                    "INSTANCE" => {
                        if let Some(component) = resolve::symbol_id(&node.data) {
                            used.add(doc, component, true);
                        }
                        if let Ok(walk) =
                            resolve::walk_instance(doc, &definitions, id, None, MAX_VISITED)
                        {
                            used.truncated |= walk.truncated;
                            resolved(&mut used, &walk.nodes);
                        }
                    }
                    _ => {}
                }
                stack.extend(doc.children(id).iter().rev());
            }
        }
        NodeId::InstancePath(path) => {
            let walk = resolve::walk_path(doc, &definitions, &path, None, MAX_VISITED)
                .map_err(|_| format!("instance path {path} does not resolve"))?;
            used.truncated |= walk.truncated;
            resolved(&mut used, &walk.nodes);
        }
    }
    Ok(used)
}
