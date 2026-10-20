//! `get_nodes_in_region`: the nodes of a page that a rectangle touches.
use crate::{
    FigstudioServer,
    tools::{to_call, tool_error},
};
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

/// Inputs for a region listing: a region link, or a page and a rectangle.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegionInput {
    /// A `figstudio://<fileKey>?page=<id>&rect=x,y,w,h` region link, as the app's Copy region
    /// gives. Use this or `page` + `rect`.
    region: Option<String>,
    /// Page (canvas) ID, such as 0:1; used with `rect`.
    page: Option<String>,
    /// `[x, y, width, height]` in page units; used with `page`. Width and height above 0.
    rect: Option<[f64; 4]>,
    /// Registered file stem or full path; optional for a single file or a region link.
    file_key: Option<String>,
    /// Most nodes to list, 1 to 500; default 50.
    limit: Option<usize>,
}

#[tool_router(router = region_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "List the visible nodes of a page whose box overlaps a rectangle, top-level nodes first. Give a figstudio region link in region (as the app's Copy region produces), or page and rect [x, y, width, height] in page units. Returns JSON {page, rect, nodes: [{nodeId, name, kind}], total, truncated}; truncated is true when more nodes touch the rectangle than limit (default 50, at most 500) or the walk hit its work budget."
    )]
    async fn get_nodes_in_region(
        &self,
        Parameters(input): Parameters<RegionInput>,
    ) -> CallToolResult {
        let key = match self.file_for(input.file_key.as_deref(), input.region.as_deref()) {
            Ok(key) => key,
            Err(error) => return tool_error(error),
        };
        let document = match self.files().open(key.as_deref()).await {
            Ok(document) => document,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || {
            figstudio_mcp_core::region_tool::nodes_in_region(
                &document,
                input.region.as_deref(),
                input.page.as_deref(),
                input.rect,
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
