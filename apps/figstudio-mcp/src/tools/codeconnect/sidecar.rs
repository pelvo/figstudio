use super::{MapInput, Mapping, fault};
use crate::node_id::{NodeId, parse_node_id};
use figstudio_fig::Document;
use figstudio_mcp_core::codeconnect_context::{component_id, component_set};
use figstudio_mcp_core::codeconnect_store::{self, Store, validate};
use rmcp::model::CallToolResult;
use std::path::Path;

pub(super) fn map(
    doc: &Document,
    path: &Path,
    input: MapInput,
    add: Option<Mapping>,
) -> CallToolResult {
    let requested = match input.node_id.as_deref().map(parse_node_id).transpose() {
        Ok(id) => id.map(|id| match id {
            NodeId::Node(id) => id.to_string(),
            NodeId::InstancePath(path) => path.to_string(),
        }),
        Err(_) => return fault("Invalid node ID"),
    };
    let id = match input
        .node_id
        .as_deref()
        .map(|id| component_id(doc, id))
        .transpose()
    {
        Ok(id) => id,
        Err(message) => return fault(message),
    };
    // Held until this function returns, so the read below and the write are one step across
    // processes; the in-process mutex in `codeconnect.rs` covers threads of this server.
    let mut store = match Store::open(path) {
        Ok(store) => store,
        Err(message) => return fault(message),
    };
    let is_add = add.is_some();
    let mut mappings = std::mem::take(&mut store.mappings);
    if let Some(mapping) = add {
        let Some(id) = id.as_ref() else {
            return fault("nodeId is required");
        };
        if let Err(message) = validate(&mapping) {
            return fault(message);
        }
        mappings.insert(id.to_string(), mapping);
        store.mappings = mappings;
        let saved = store.save();
        mappings = std::mem::take(&mut store.mappings);
        if let Err(message) = saved {
            return fault(message);
        }
    }
    let output_id = if is_add {
        id.map(|id| id.to_string())
    } else {
        requested
    };
    if let (Some(requested), Some(id)) = (output_id, id) {
        let candidates = [
            Some(requested.clone()),
            Some(id.to_string()),
            component_set(doc, id).map(|set| set.id.to_string()),
        ];
        let selected = candidates
            .into_iter()
            .flatten()
            .find_map(|key| mappings.get(&key).cloned());
        mappings.clear();
        if let Some(mapping) = selected {
            mappings.insert(requested, mapping);
        }
    }
    mappings.retain(|_, mapping| {
        matches(input.client_frameworks.as_deref(), |filter| {
            mapping.label.eq_ignore_ascii_case(filter)
        }) && matches(input.client_languages.as_deref(), |filter| {
            language_matches(mapping, filter)
        })
    });
    match serde_json::to_value(mappings) {
        Ok(value) => CallToolResult::structured(value),
        Err(_) => fault("Cannot serialize Code Connect mappings"),
    }
}

/// Every mapping in the sidecar of the registered file at `path` (empty when there is none).
pub(super) fn mapped(path: &Path) -> Result<super::Mappings, &'static str> {
    codeconnect_store::mapped(path)
}

/// `send_code_connect_mappings`: see [`codeconnect_store::add_many`]. An error result only
/// when nothing was created.
pub(super) fn add_many(
    doc: &Document,
    path: &Path,
    items: Vec<(String, Mapping)>,
) -> CallToolResult {
    match codeconnect_store::add_many(doc, path, items) {
        Ok(value) => {
            let none = value["created"].as_array().is_none_or(Vec::is_empty);
            let mut result = CallToolResult::structured(value);
            if none {
                result.is_error = Some(true);
            }
            result
        }
        Err(message) => fault(message),
    }
}

fn matches(filters: Option<&[String]>, predicate: impl Fn(&str) -> bool) -> bool {
    filters
        .is_none_or(|filters| filters.is_empty() || filters.iter().any(|filter| predicate(filter)))
}

fn language_matches(mapping: &Mapping, filter: &str) -> bool {
    if mapping.label.eq_ignore_ascii_case(filter) {
        return true;
    }
    let source = mapping.source.split(['?', '#']).next().unwrap_or("");
    let ext = Path::new(source)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match filter.to_ascii_lowercase().as_str() {
        "typescript" | "ts" => matches!(ext.as_str(), "ts" | "tsx"),
        "javascript" | "js" => matches!(ext.as_str(), "js" | "jsx" | "mjs"),
        "python" => ext == "py",
        "rust" => ext == "rs",
        language => ext == language,
    }
}
