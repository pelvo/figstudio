//! Used style and variable definitions for a local subtree.
pub(crate) mod format;
use crate::{
    FigstudioServer,
    node_id::{NodeId, parse_node_id},
    tools::tool_error,
};
use figstudio_fig::{
    Document, Guid, Val,
    resolve::{self, InstancePath, walk_instance, walk_path},
    tokens::{Tokens, Variable},
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock},
    tool, tool_router,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VariablesInput {
    /// Node ID, instance path (I1:2;3:4) or Figma URL.
    node_id: String,
    /// Registered file key; optional when just one file is registered.
    file_key: Option<String>,
}

#[tool_router(router = variables_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Get a flat JSON map of variable and style names used by a local subtree, including the content of instances with their overrides applied. Aliases resolve, additional modes use name (Mode) keys. nodeId may be an instance path (I1:2;3:4)."
    )]
    async fn get_variable_defs(
        &self,
        Parameters(input): Parameters<VariablesInput>,
    ) -> CallToolResult {
        let key = self.file_for(input.file_key.as_deref(), Some(&input.node_id));
        let doc = match self.files().open(key.as_deref()).await {
            Ok(doc) => doc,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || variable_defs(&doc, &input.node_id)).await {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }
}

/// Build a map from the references of a subtree; instances contribute their resolved content.
pub fn variable_defs(doc: &Document, node_id: &str) -> CallToolResult {
    let unknown = || CallToolResult::error(vec![ContentBlock::text("Unknown node ID")]);
    let root = match parse_node_id(node_id) {
        Ok(NodeId::Node(id)) if doc.node(id).is_some() => Root::Node(id),
        Ok(NodeId::Node(_)) => return unknown(),
        Ok(NodeId::InstancePath(path)) => Root::Path(path),
        Err(_) => return CallToolResult::error(vec![ContentBlock::text("Invalid node ID")]),
    };
    let tokens = Tokens::new(doc);
    let mut styles = BTreeSet::new();
    let mut variables = BTreeSet::new();
    let mut skipped = 0;
    let definitions = Arc::new(resolve::definitions(doc));
    let mut add = |data: &Val| references(data, &tokens, &mut styles, &mut variables);
    let mut instances = Vec::new();
    match root {
        Root::Path(path) => {
            let Ok(walk) = walk_path(doc, &definitions, &path, None, MAX_RESOLVED) else {
                return unknown();
            };
            walk.nodes.iter().for_each(|r| skipped += add(&r.data));
            skipped += walk.skipped_instances + usize::from(walk.truncated);
        }
        Root::Node(root) => {
            let mut seen = HashSet::new();
            let mut stack = vec![root];
            while let Some(id) = stack.pop() {
                let Some(node) = doc.node(id).filter(|_| seen.insert(id)) else {
                    continue;
                };
                skipped += add(&node.data);
                if node.kind == "INSTANCE" {
                    instances.push(id);
                } else {
                    stack.extend(doc.children(id));
                }
            }
        }
    }
    // Every instance's content, as drawn: its component's children with overrides applied.
    for id in instances {
        match walk_instance(doc, &definitions, id, None, MAX_RESOLVED) {
            Ok(walk) => {
                walk.nodes.iter().for_each(|r| skipped += add(&r.data));
                skipped += walk.skipped_instances + usize::from(walk.truncated);
            }
            Err(_) => skipped += 1,
        }
    }
    // A used style can itself consume variables. Do not include unrelated definitions.
    for id in &styles {
        if let Some(style) = tokens.style(&id.to_val()) {
            skipped += references(&style.data, &tokens, &mut BTreeSet::new(), &mut variables);
        }
    }
    let mut map = BTreeMap::new();
    for id in styles {
        let Some(style) = tokens.style(&id.to_val()) else {
            continue;
        };
        match format::style(style, &tokens) {
            Some(value) => {
                insert(&mut map, &style.name, value, &mut skipped);
            }
            None => skipped += 1,
        }
    }
    add_variables(&variables, &tokens, &mut map, &mut skipped);
    let mut result = CallToolResult::structured(json!(map));
    if map.is_empty() {
        result.content.push(ContentBlock::text(
            "No resolved tokens used by this subtree.",
        ));
    }
    if skipped != 0 {
        result.content.push(ContentBlock::text(format!(
            "approximations: {skipped} unresolved or duplicate token definitions or unexpanded instances"
        )));
    }
    result
}

/// Where the references are collected: a document subtree or a node inside an instance.
enum Root {
    Node(Guid),
    Path(InstancePath),
}

/// Resolved nodes walked per instance; bounds work on exponentially nested files.
const MAX_RESOLVED: usize = resolve::MAX_RESOLVED_NODES;

/// Fields that store instance overrides or component property definitions and assignments.
const NOT_EFFECTIVE: [&str; 5] = [
    "symbolData",
    "derivedSymbolData",
    "componentPropAssignments",
    "componentPropDefs",
    "componentPropRefs",
];

const STYLE_FIELDS: [&str; 10] = [
    "styleIdForFill",
    "styleIdForStrokeFill",
    "styleIdForText",
    "styleIdForEffect",
    "styleIdForGrid",
    "inheritFillStyleID",
    "inheritStrokeStyleID",
    "inheritTextStyleID",
    "inheritEffectStyleID",
    "inheritGridStyleID",
];

fn references(
    data: &Val,
    tokens: &Tokens,
    styles: &mut BTreeSet<Guid>,
    variables: &mut BTreeSet<Guid>,
) -> usize {
    let mut unresolved = 0;
    let mut stack = vec![data];
    while let Some(value) = stack.pop() {
        match value {
            Val::Obj(o) => {
                for reference in STYLE_FIELDS.iter().filter_map(|field| value.get(field)) {
                    if reference
                        .as_guid()
                        .or_else(|| reference.get("guid").and_then(Val::as_guid))
                        == Some(Guid(u32::MAX, u32::MAX))
                    {
                        continue;
                    }
                    match tokens.style(reference) {
                        Some(style) => {
                            styles.insert(style.id);
                        }
                        None => unresolved += 1,
                    }
                }
                for reference in ["colorVar", "variableData"]
                    .iter()
                    .filter_map(|field| value.get(field).and_then(|v| v.path(&["value", "alias"])))
                {
                    match tokens.variable(reference) {
                        Some(variable) => {
                            variables.insert(variable.id);
                        }
                        None => unresolved += 1,
                    }
                }
                // Override and property storage is not what the node shows: overrides reach
                // the nodes they target through the resolver, unused ones must not count.
                stack.extend(
                    o.iter()
                        .filter(|(key, _)| !NOT_EFFECTIVE.contains(key))
                        .map(|(_, value)| value),
                );
            }
            Val::Arr(items) => stack.extend(items),
            _ => {}
        }
    }
    unresolved
}

fn add_variables(
    ids: &BTreeSet<Guid>,
    tokens: &Tokens,
    map: &mut BTreeMap<String, Value>,
    skipped: &mut usize,
) {
    let mut names: BTreeMap<&str, HashSet<Option<Guid>>> = BTreeMap::new();
    for id in ids {
        if let Some(v) = tokens.variable(&id.to_val()) {
            names.entry(&v.name).or_default().insert(v.collection);
        }
    }
    for id in ids {
        if let Some(variable) = tokens.variable(&id.to_val()) {
            let duplicate = names
                .get(variable.name.as_str())
                .is_some_and(|c| c.len() > 1);
            let name = if duplicate {
                let collection = variable.collection.and_then(|id| tokens.collection(id));
                format!(
                    "{} ({})",
                    variable.name,
                    collection.map_or("uncollected", |c| c.name.as_str())
                )
            } else {
                variable.name.clone()
            };
            add_variable(variable, &name, tokens, map, skipped);
        }
    }
}

fn add_variable(
    variable: &Variable,
    base_name: &str,
    tokens: &Tokens,
    map: &mut BTreeMap<String, Value>,
    skipped: &mut usize,
) {
    let collection = variable.collection.and_then(|id| tokens.collection(id));
    let default = collection
        .and_then(|c| c.default_mode)
        .or_else(|| variable.values.first().and_then(|v| v.mode));
    for entry in &variable.values {
        let name = if variable.values.len() == 1 || entry.mode == default {
            base_name.to_owned()
        } else {
            let mode_name = collection
                .and_then(|c| c.modes.iter().find(|m| Some(m.id) == entry.mode))
                .map(|m| m.name.clone())
                .unwrap_or_else(|| {
                    entry
                        .mode
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "unnamed".into())
                });
            format!("{base_name} ({mode_name})")
        };
        match tokens
            .resolve(&entry.data, entry.mode)
            .and_then(format::value)
        {
            Some(value) => insert(map, &name, value, skipped),
            None => *skipped += 1,
        }
    }
}

// Styles win exact name collisions with variables; within each kind, the lowest GUID wins.
fn insert(map: &mut BTreeMap<String, Value>, name: &str, value: Value, skipped: &mut usize) {
    if let std::collections::btree_map::Entry::Vacant(entry) = map.entry(name.into()) {
        entry.insert(value);
    } else {
        *skipped += 1;
    }
}
