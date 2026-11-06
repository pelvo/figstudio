//! `get_prototype`: flows, the screens they reach and the links on each.
use crate::{
    FigstudioServer,
    tools::{to_call, tool_error},
};
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

/// Inputs for a prototype listing.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PrototypeInput {
    /// A screen (top-level frame), or a layer in one, to start from; default: every flow.
    node_id: Option<String>,
    /// Registered file stem or full path; optional for a single file or a link.
    file_key: Option<String>,
    /// Most screens to list, 1 to 500; default 50.
    limit: Option<usize>,
}

#[tool_router(router = prototype_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Read the file's prototype: its flows (starting points) and the screens reachable from them, or from the screen holding nodeId (a top-level frame or a layer in one; pages are refused), by played navigate links, breadth first. Returns JSON {flows: [{name, description, page, startNodeId}], screens: [{nodeId, name, links: [{nodeId, name, trigger, action, destinationId, destinationMissing, url, transition, played, rect}], truncated}], truncated, notPlayed}. trigger and action are Figma's names (ON_CLICK, ON_HOVER, AFTER_TIMEOUT; NAVIGATE, BACK, OVERLAY, SWAP_STATE, URL, ...); played is true for the links figstudio's Preview follows (navigate, back and change-to on click, hover, press, mouse enter/leave/down/up, after delay and drag; navigate only to a visible frame); rect is [x, y, width, height] in page units. Render a screen with get_screenshot and its nodeId. Without flows, starts at the first page's first frame. limit: screens, default 50, at most 500."
    )]
    async fn get_prototype(&self, Parameters(input): Parameters<PrototypeInput>) -> CallToolResult {
        let key = match self.file_for(input.file_key.as_deref(), input.node_id.as_deref()) {
            Ok(key) => key,
            Err(error) => return tool_error(error),
        };
        let document = match self.files().open(key.as_deref()).await {
            Ok(document) => document,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || {
            figstudio_mcp_core::prototype_tool::prototype(
                &document,
                input.node_id.as_deref(),
                input.limit,
            )
        })
        .await
        {
            Ok(output) => to_call(output),
            Err(error) => tool_error(crate::error::McpError::load_task(error)),
        }
    }
}
