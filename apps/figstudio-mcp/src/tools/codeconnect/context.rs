use super::fault;
use crate::node_id::{NodeId, parse_node_id};
use figstudio_fig::{Document, Guid, Node, Val, resolve};
use rmcp::model::CallToolResult;
use serde_json::{Value, json};

pub(super) fn component_set(doc: &Document, id: Guid) -> Option<&Node> {
    let node = doc.node(id)?;
    let is_set =
        |node: &&Node| node.kind == "COMPONENT_SET" || node.data.flag("isStateGroup") == Some(true);
    Some(node)
        .filter(is_set)
        .or_else(|| node.parent.and_then(|id| doc.node(id)).filter(is_set))
}

pub(super) fn is_component(node: &Node) -> bool {
    node.kind == "SYMBOL"
        || node.kind == "COMPONENT_SET"
        || node.data.flag("isStateGroup") == Some(true)
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
    let set = component_set(doc, id);
    let defs = node
        .data
        .get("componentPropDefs")
        .or_else(|| set.and_then(|set| set.data.get("componentPropDefs")))
        .map_or(&[][..], Val::items);
    let options: serde_json::Map<String, Value> = set
        .and_then(|set| set.data.get("stateGroupPropertyValueOrders"))
        .map_or(&[][..], Val::items)
        .iter()
        .take(100)
        .filter_map(|order| Some((order.text("property")?.into(), value(order.get("values")?))))
        .collect();
    let variants: Vec<_> = doc
        .children(set.map_or(id, |set| set.id))
        .iter()
        .filter_map(|id| doc.node(*id))
        .filter(|node| node.kind == "SYMBOL")
        .take(100)
        .map(|n| {
            let properties: serde_json::Map<String, Value> = n
                .data
                .get("variantPropSpecs")
                .map_or(&[][..], Val::items)
                .iter()
                .take(100)
                .filter_map(|spec| {
                    let prop = spec.get("propDefId")?.as_guid()?;
                    let name = defs
                        .iter()
                        .find(|def| def.get("id").and_then(Val::as_guid) == Some(prop))?
                        .text("name")?;
                    Some((name.into(), value(spec.get("value")?)))
                })
                .collect();
            json!({"nodeId":n.id.to_string(), "name":n.name, "variantProperties":properties})
        })
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
        "componentPropDefs":defs.iter().take(100).map(value).collect::<Vec<_>>(),
        "variantOptions":options, "variants":variants, "nestedInstances":nested,
        "truncated":!stack.is_empty() || defs.len() > 100 || doc.children(set.map_or(id, |set| set.id)).len() > 100}))
}

// Keep schema names/unknown enum values intact rather than guessing property types.
fn value(data: &Val) -> Value {
    if let Some(id) = data.as_guid() {
        return json!(id.to_string());
    }
    match data {
        Val::Bool(v) => json!(v),
        Val::Int(v) => json!(v),
        Val::Float(v) => json!(v),
        Val::Str(v) => json!(v),
        Val::Enum(v) => json!(v),
        Val::Bytes(_) => Value::Null,
        Val::Arr(v) => Value::Array(v.iter().take(100).map(value).collect()),
        Val::Obj(v) => Value::Object(
            v.iter()
                .filter(|(_, v)| !matches!(v, Val::Bytes(_)))
                .map(|(key, v)| (key.into(), value(v)))
                .collect(),
        ),
    }
}
