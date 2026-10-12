//! Bounded local design-system discovery; no team-library access.
use super::{to_call, tool_error};
use crate::FigstudioServer;
use figstudio_fig::Document;
use figstudio_mcp_core::{design_system, output::Output};
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

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
        match tokio::task::spawn_blocking(move || to_call(Output::structured(summary(&doc)))).await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }
}

/// Search local design objects by name, ranked exact, prefix, then substring.
pub fn search(doc: &Document, query: &str, kinds: Option<&[String]>) -> CallToolResult {
    to_call(design_system::search(doc, query, kinds))
}

/// Return capped components, collections/modes, variables and styles for offline guidance.
pub fn summary(doc: &Document) -> Value {
    design_system::summary(doc)
}
