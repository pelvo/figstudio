//! Bounded local design-system discovery; no team-library access.
use super::tool_error;
use crate::FigstudioServer;
use figstudio_fig::{Document, tokens::Tokens};
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const LIMIT: usize = 100;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchInput {
    query: String,
    kinds: Option<Vec<String>>,
    file_key: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibrariesInput {
    file_key: Option<String>,
}

#[tool_router(router = design_system_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Search this file's local components, component sets, styles and variables by case-insensitive name. kinds: component, component_set, style, variable. Exact names rank first; at most 100 results. No team libraries."
    )]
    async fn search_design_system(
        &self,
        Parameters(input): Parameters<SearchInput>,
    ) -> CallToolResult {
        let doc = match self.files().open(input.file_key.as_deref()).await {
            Ok(doc) => doc,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || {
            search(&doc, &input.query, input.kinds.as_deref())
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }

    #[tool(
        description = "Summarize this file's own components and tokens. Offline only: team and published libraries are unavailable. Lists are capped at 100 entries, with totals."
    )]
    async fn get_libraries(&self, Parameters(input): Parameters<LibrariesInput>) -> CallToolResult {
        let doc = match self.files().open(input.file_key.as_deref()).await {
            Ok(doc) => doc,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || CallToolResult::structured(summary(&doc))).await {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }
}

fn entries(doc: &Document) -> Vec<Value> {
    let tokens = Tokens::new(doc);
    let mut entries = Vec::new();
    for node in &doc.nodes {
        let kind = if node.kind == "COMPONENT_SET" || node.data.flag("isStateGroup") == Some(true) {
            "component_set"
        } else if node.kind == "SYMBOL" {
            "component"
        } else if tokens.style(&node.id.to_val()).is_some() {
            "style"
        } else if tokens.variable(&node.id.to_val()).is_some() {
            "variable"
        } else {
            continue;
        };
        entries.push(json!({"nodeId":node.id.to_string(), "name":node.name, "kind":kind}));
    }
    entries
}

/// Search local design objects by name, ranked exact, prefix, then substring.
pub fn search(doc: &Document, query: &str, kinds: Option<&[String]>) -> CallToolResult {
    let query = query.to_lowercase();
    let mut results: Vec<_> = entries(doc)
        .into_iter()
        .filter(|entry| {
            entry["name"]
                .as_str()
                .unwrap_or("")
                .to_lowercase()
                .contains(&query)
                && kinds.is_none_or(|kinds| {
                    kinds.is_empty()
                        || kinds.iter().any(|kind| {
                            entry["kind"]
                                .as_str()
                                .is_some_and(|k| k.eq_ignore_ascii_case(kind))
                        })
                })
        })
        .collect();
    results.sort_by_key(|entry| {
        let name = entry["name"].as_str().unwrap_or("").to_lowercase();
        let rank = if name == query {
            0
        } else if name.starts_with(&query) {
            1
        } else {
            2
        };
        (
            rank,
            name,
            entry["nodeId"].as_str().unwrap_or("").to_owned(),
        )
    });
    let total = results.len();
    results.truncate(LIMIT);
    CallToolResult::structured(
        json!({"results":results, "total":total, "truncated":total > LIMIT, "scope":"local file only"}),
    )
}

/// Return capped components, collections/modes, variables and styles for offline guidance.
pub fn summary(doc: &Document) -> Value {
    let entries = entries(doc);
    let tokens = Tokens::new(doc);
    let list = |kinds: &[&str]| {
        let mut items: Vec<_> = entries
            .iter()
            .filter(|entry| kinds.contains(&entry["kind"].as_str().unwrap_or("")))
            .cloned()
            .collect();
        items.sort_by_key(|entry| {
            (
                entry["name"].as_str().unwrap_or("").to_owned(),
                entry["nodeId"].as_str().unwrap_or("").to_owned(),
            )
        });
        let total = items.len();
        items.truncate(LIMIT);
        for item in &mut items {
            item["name"] = json!(
                item["name"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(200)
                    .collect::<String>()
            );
        }
        json!({"items":items, "total":total, "truncated":total > LIMIT})
    };
    let mut collections: Vec<_> = doc
        .nodes
        .iter()
        .filter_map(|node| tokens.collection(node.id))
        .collect();
    collections.sort_by_key(|collection| (&collection.name, collection.id));
    let total = collections.len();
    let collections: Vec<_> = collections.into_iter().take(LIMIT).map(|collection| json!({
        "nodeId":collection.id.to_string(), "name":collection.name.chars().take(200).collect::<String>(),
        "modes":collection.modes.iter().take(LIMIT).map(|mode| json!({"id":mode.id.to_string(),"name":mode.name.chars().take(200).collect::<String>()})).collect::<Vec<_>>(),
        "modeCount":collection.modes.len(), "truncated":collection.modes.len() > LIMIT,
    })).collect();
    json!({"scope":"local file only", "teamLibrariesAvailable":false,
        "components":list(&["component", "component_set"]),
        "variables":list(&["variable"]), "styles":list(&["style"]),
        "collections":{"items":collections,"total":total,"truncated":total > LIMIT}})
}
