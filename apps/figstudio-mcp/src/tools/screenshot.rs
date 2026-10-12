//! `get_screenshot`: a PNG render of one node, drawn by the figstudio renderer.
use crate::{
    FigstudioServer,
    tools::{rendering::Rendering, to_call, tool_error},
};
use figstudio_fig::Document;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub use figstudio_mcp_core::screenshot::{DEFAULT_SIDE_PX, MAX_SIDE_PX, SCALE_RANGE};

/// Inputs for a node screenshot.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScreenshotInput {
    /// Node ID or Figma URL with `node-id`.
    node_id: String,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
    /// Pixels per design unit, 0.01 to 4. Without it the scale is 1 and the longest side is
    /// capped at 2048 px; with it, at 4096 px.
    scale: Option<f64>,
}

#[tool_router(router = screenshot_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Render a node to a PNG image. Pages (canvases) are drawn on the page background colour; every other node on a transparent background. scale defaults to 1 (0.01 to 4) and the longest side is then capped at 2048 px; an explicit scale is capped at 4096 px. The output is cropped to the node's own box (no padding), so shadows and strokes outside it are cut off. The text reply lists what was approximated or not drawn. An instance path (I1:2;3:4) renders that node alone (no siblings, no ancestor clips) as laid out inside its instance."
    )]
    async fn get_screenshot(
        &self,
        Parameters(input): Parameters<ScreenshotInput>,
    ) -> CallToolResult {
        let key = self.file_for(input.file_key.as_deref(), Some(&input.node_id));
        let document = match self.files().open(key.as_deref()).await {
            Ok(document) => document,
            Err(error) => return tool_error(error),
        };
        let rendering = match self
            .files()
            .rendering(key.as_deref(), !self.no_system_fonts())
        {
            Ok(rendering) => rendering,
            Err(error) => return tool_error(error),
        };
        match tokio::task::spawn_blocking(move || {
            screenshot(&document, &input.node_id, input.scale, &rendering)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::load_task(error)),
        }
    }
}

/// Render `node_id` to a base64 PNG plus a size and approximations line.
pub fn screenshot(
    doc: &Document,
    node_id: &str,
    scale: Option<f64>,
    rendering: &Rendering,
) -> CallToolResult {
    to_call(figstudio_mcp_core::screenshot::screenshot(
        doc, node_id, scale, rendering,
    ))
}
