//! Used style and variable definitions for a local subtree.
mod format;
use crate::{
    FigstudioServer,
    node_id::{NodeId, parse_node_id},
    tools::tool_error,
};
use figstudio_fig::{
    Document, Guid, Val,
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

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VariablesInput {
    /// Local node ID or Figma URL; instance paths are not yet resolved.
    node_id: String,
    /// Registered file key; optional when just one file is registered.
    file_key: Option<String>,
}

#[tool_router(router = variables_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Get a flat JSON map of variable and style names used by a local subtree. Aliases resolve, additional modes use name (Mode) keys. Instance paths are not yet resolved."
    )]
    async fn get_variable_defs(
        &self,
        Parameters(input): Parameters<VariablesInput>,
    ) -> CallToolResult {
        let doc = match self.files().open(input.file_key.as_deref()).await {
            Ok(doc) => doc,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || variable_defs(&doc, &input.node_id)).await {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }
}

/// Build a map from local stored references, without expanding symbol instances.
pub fn variable_defs(doc: &Document, node_id: &str) -> CallToolResult {
    let Ok(NodeId::Node(root)) = parse_node_id(node_id) else {
        return CallToolResult::error(vec![ContentBlock::text(
            "Invalid or unresolved instance node ID",
        )]);
    };
    if doc.node(root).is_none() {
        return CallToolResult::error(vec![ContentBlock::text("Unknown node ID")]);
    }
    let tokens = Tokens::new(doc);
    let mut styles = BTreeSet::new();
    let mut variables = BTreeSet::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    let mut skipped = 0;
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(node) = doc.node(id) {
            skipped += references(&node.data, &tokens, &mut styles, &mut variables);
            stack.extend(doc.children(id));
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
            "approximations: {skipped} unresolved or duplicate token definitions"
        )));
    }
    result
}

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
                stack.extend(o.values());
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
