use super::fault;
use crate::node_id::{NodeId, parse_node_id};
use figstudio_fig::{Document, Guid, Node, Val, resolve};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};

pub(super) fn is_component(node: &Node) -> bool {
    node.kind == "SYMBOL"
        || node.kind == "COMPONENT_SET"
        || node.data.flag("stateGroup") == Some(true)
}

pub(super) fn component_id(doc: &Document, input: &str) -> Result<Guid, &'static str> {
    let (id, data) = match parse_node_id(input).map_err(|_| "Invalid node ID")? {
        NodeId::Node(id) => (id, doc.node(id).ok_or("Unknown node ID")?.data.clone()),
        NodeId::InstancePath(path) => {
            let node = resolve::resolve_path(doc, &path).map_err(|_| "Unknown instance path")?;
            (node.node, node.data)
        }
    };
    let id = if data.text("type") == Some("INSTANCE") {
        resolve::symbol_id(&data).ok_or("Instance has no local component")?
    } else {
        id
    };
    doc.node(id)
        .filter(|node| is_component(node))
        .ok_or("Expected a local component, component set or instance")?;
    Ok(id)
}

/// Return schema-preserving component properties and bounded local child context.
pub fn component_context(doc: &Document, input: &str) -> CallToolResult {
    let id = match component_id(doc, input) {
        Ok(id) => id,
        Err(message) => return fault(message),
    };
    let Some(node) = doc.node(id) else {
        return fault("Unknown component");
    };
    let set = if node.data.flag("stateGroup") == Some(true) || node.kind == "COMPONENT_SET" {
        Some(node)
    } else {
        node.parent
            .and_then(|parent| doc.node(parent))
            .filter(|parent| {
                parent.data.flag("stateGroup") == Some(true) || parent.kind == "COMPONENT_SET"
            })
    };
    let variants: Vec<_> = doc
        .children(set.map_or(id, |set| set.id))
        .iter()
        .filter_map(|id| doc.node(*id))
        .filter(|node| node.kind == "SYMBOL")
        .take(100)
        .map(|n| json!({"nodeId":n.id.to_string(), "name":n.name}))
        .collect();
    let mut nested = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![(id, 0)];
    while let Some((parent, offset)) = stack.pop() {
        if seen.len() >= 1000 || nested.len() >= 100 {
            break;
        }
        let Some(child) = doc.children(parent).get(offset).copied() else {
            continue;
        };
        stack.push((parent, offset + 1));
        if !seen.insert(child) {
            continue;
        }
        if let Some(n) = doc.node(child) {
            if n.kind == "INSTANCE" {
                nested.push(json!({"nodeId":n.id.to_string(), "name":n.name,
                    "componentId":resolve::symbol_id(&n.data).map(|id| id.to_string())}));
            }
            stack.push((child, 0));
        }
    }
    CallToolResult::structured(json!({"nodeId":id.to_string(), "componentName":node.name,
        "description":node.data.text("description"),
        "componentPropDefs":node.data.get("componentPropDefs").or_else(|| set.and_then(|set| set.data.get("componentPropDefs"))).map(value).unwrap_or_else(|| json!([])),
        "variants":variants, "nestedInstances":nested,
        "truncated":!stack.is_empty() || doc.children(id).len() > 100}))
}

// Keep schema names/unknown enum values intact rather than guessing property types.
fn value(data: &Val) -> Value {
    match data {
        Val::Bool(v) => json!(v),
        Val::Int(v) => json!(v),
        Val::Float(v) => json!(v),
        Val::Str(v) => json!(v),
        Val::Enum(v) => json!(v),
        Val::Bytes(v) => json!(v),
        Val::Arr(v) => Value::Array(v.iter().map(value).collect()),
        Val::Obj(v) => Value::Object(v.iter().map(|(key, v)| (key.into(), value(v))).collect()),
    }
}
