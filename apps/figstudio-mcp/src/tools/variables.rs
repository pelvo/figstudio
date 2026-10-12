//! Used style and variable definitions for a local subtree.
use crate::{
    FigstudioServer,
    tools::{to_call, tool_error},
};
use figstudio_fig::Document;
use figstudio_mcp_core::variables;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VariablesInput {
    /// Node ID, instance path (I1:2;3:4) or Figma URL. Required, except in the app's live mode,
    /// where it defaults to the selection.
    node_id: Option<String>,
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
        let key = self.file_for(input.file_key.as_deref(), input.node_id.as_deref());
        let (key, node_id) = match self.target(key, input.node_id) {
            Ok(target) => target,
            Err(error) => return tool_error(error),
        };
        let doc = match self.files().open(key.as_deref()).await {
            Ok(doc) => doc,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || variable_defs(&doc, &node_id)).await {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::token_task(error)),
        }
    }
}

/// Build a map from the references of a subtree; instances contribute their resolved content.
pub fn variable_defs(doc: &Document, node_id: &str) -> CallToolResult {
    to_call(variables::variable_defs(doc, node_id))
}
