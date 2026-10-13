//! `get_figjam`: `get_metadata` XML plus FigJam details, with optional small screenshots.
use crate::{
    FigstudioServer,
    tools::{rendering::Rendering, to_call, tool_error},
};
use figstudio_fig::Document;
use figstudio_mcp_core::figjam as core;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub use core::{MAX_IMAGE_SCALE, MAX_IMAGES};

/// Inputs for a FigJam board listing.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FigjamInput {
    /// Node ID or Figma URL; omit for the page list.
    node_id: Option<String>,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
    /// Maximum child depth, with the requested root at depth zero.
    depth: Option<u32>,
    /// Also return a small PNG of each sticky, shape, section, table, code block, widget and
    /// stamp (at most 20, in document order). Default false.
    include_images: Option<bool>,
    /// Pixels per design unit for those images, 0.01 to 0.5 (default 0.25).
    image_scale: Option<f64>,
}

#[tool_router(router = figjam_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Get a FigJam board as sparse XML (same shape as get_metadata) with FigJam details: sticky text and colour, shape-with-text shape and text, connector start/end node ids and magnets, section status, table cell and code block text, widget and stamp names. Text caps at 200 characters. includeImages=true adds a small PNG per sticky/shape/section/table/code block/widget/stamp (max 20, imageScale up to 0.5). FigJam kinds are not drawn by the renderer yet: images show them as plain boxes, and an XML comment counts those kinds."
    )]
    async fn get_figjam(&self, Parameters(input): Parameters<FigjamInput>) -> CallToolResult {
        let key = self.file_for(input.file_key.as_deref(), input.node_id.as_deref());
        let document = match self.files().open(key.as_deref()).await {
            Ok(document) => document,
            Err(error) => return tool_error(error),
        };
        let rendering =
            match self
                .files()
                .rendering(key.as_deref(), &document, !self.no_system_fonts())
            {
                Ok(rendering) => rendering,
                Err(error) => return tool_error(error),
            };
        let max_bytes = self.max_output_bytes();
        match tokio::task::spawn_blocking(move || {
            figjam_within(
                &document,
                input.node_id.as_deref(),
                input.depth,
                input.include_images.unwrap_or(false),
                input.image_scale,
                &rendering,
                crate::tools::assets::MAX_PAYLOAD_BYTES,
                max_bytes,
            )
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::load_task(error)),
        }
    }
}

/// The FigJam XML for `node_id`, then one text line and PNG per pictured node when asked.
pub fn figjam(
    doc: &Document,
    node_id: Option<&str>,
    depth: Option<u32>,
    include_images: bool,
    image_scale: Option<f64>,
    rendering: &Rendering,
) -> CallToolResult {
    to_call(core::figjam(
        doc,
        node_id,
        depth,
        include_images,
        image_scale,
        rendering,
    ))
}

/// [`figjam`] with explicit image payload and XML output budgets in bytes (before base64).
#[expect(clippy::too_many_arguments, reason = "mirrors the MCP tool inputs")]
pub fn figjam_within(
    doc: &Document,
    node_id: Option<&str>,
    depth: Option<u32>,
    include_images: bool,
    image_scale: Option<f64>,
    rendering: &Rendering,
    max_payload: usize,
    max_bytes: usize,
) -> CallToolResult {
    to_call(core::figjam_within(
        doc,
        node_id,
        depth,
        include_images,
        image_scale,
        rendering,
        max_payload,
        max_bytes,
    ))
}
