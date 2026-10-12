//! `download_assets`: PNG or JPEG renders of nodes, or the original bytes of their images.
use crate::{
    FigstudioServer,
    tools::{rendering::Rendering, to_call, tool_error},
};
use figstudio_fig::Document;
use figstudio_mcp_core::assets as core;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub use core::{
    IMAGE_CAP_LABEL, MAX_IMAGE_BYTES, MAX_NODES, MAX_ORIGINALS, MAX_PAYLOAD_BYTES, PAYLOAD_LABEL,
    Payload,
};

/// Output format of a rendered asset.
#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AssetFormat {
    /// Lossless, keeps transparency.
    #[default]
    Png,
    /// Lossy; transparent areas are flattened onto white.
    Jpg,
}

impl From<AssetFormat> for core::AssetFormat {
    fn from(format: AssetFormat) -> Self {
        match format {
            AssetFormat::Png => Self::Png,
            AssetFormat::Jpg => Self::Jpg,
        }
    }
}

/// Inputs for asset download.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AssetsInput {
    /// 1 to 20 node IDs or Figma URLs with `node-id`; instance paths (I1:2;3:4) are accepted.
    /// In the app's live mode an empty or missing list means the selection.
    #[serde(default)]
    node_ids: Vec<String>,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
    /// `png` (default) or `jpg`.
    format: Option<AssetFormat>,
    /// Pixels per design unit, 0.01 to 4 (default 1). Without it the longest side is capped at
    /// 2048 px; with it, at 4096 px.
    scale: Option<f64>,
    /// Return the original bytes of the image fills inside each node instead of a render.
    original: Option<bool>,
}

#[tool_router(router = assets_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Download up to 20 nodes as PNG or JPG images (format, scale 0.01 to 4; longest side capped at 2048 px without scale, 4096 px with it). JPG has no transparency: nodes are flattened onto white, pages keep their page background. With original=true the call returns the source files of the image fills inside each node (up to 12 per call, each once) with their detected MIME type instead of a render. Each node gets a text line; a node that fails is reported there and does not fail the call. Instance paths (I1:2;3:4) are accepted. SVG and PDF export are not available offline."
    )]
    async fn download_assets(
        &self,
        Parameters(mut input): Parameters<AssetsInput>,
    ) -> CallToolResult {
        // A link in the first node ID picks the file for the whole call.
        let mut key = self.file_for(
            input.file_key.as_deref(),
            input.node_ids.first().map(String::as_str),
        );
        // Standalone, an empty list stays empty: the usual "1 to 20 ids" error.
        if input.node_ids.is_empty()
            && let crate::files::Files::Live(_) = self.files()
        {
            match self.target(key, None) {
                Ok((file_key, node_id)) => {
                    key = file_key;
                    input.node_ids.push(node_id);
                }
                Err(error) => return tool_error(error),
            }
        }
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
        match tokio::task::spawn_blocking(move || {
            download(
                &document,
                &input.node_ids,
                input.format.unwrap_or_default(),
                input.scale,
                input.original.unwrap_or(false),
                &rendering,
            )
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::load_task(error)),
        }
    }
}

/// Renders or extracts the assets for `node_ids`; the result is an error only when the
/// request is invalid or every node failed.
pub fn download(
    doc: &Document,
    node_ids: &[String],
    format: AssetFormat,
    scale: Option<f64>,
    original: bool,
    rendering: &Rendering,
) -> CallToolResult {
    to_call(core::download(
        doc,
        node_ids,
        format.into(),
        scale,
        original,
        rendering,
    ))
}

/// [`download`] with an explicit payload budget in bytes (before base64).
pub fn download_within(
    doc: &Document,
    node_ids: &[String],
    format: AssetFormat,
    scale: Option<f64>,
    original: bool,
    rendering: &Rendering,
    max_payload: usize,
) -> CallToolResult {
    to_call(core::download_within(
        doc,
        node_ids,
        format.into(),
        scale,
        original,
        rendering,
        max_payload,
    ))
}
