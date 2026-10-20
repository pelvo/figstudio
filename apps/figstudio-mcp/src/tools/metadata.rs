//! Sparse metadata XML for pages and local node subtrees.
use crate::{
    FigstudioServer,
    tools::{to_call, tool_error},
};
use figstudio_fig::Document;
use figstudio_mcp_core::metadata as core;
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

pub use core::DEFAULT_OUTPUT_BYTES;

/// Inputs for sparse metadata retrieval.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetadataInput {
    /// Node ID or Figma URL; omit for the page list.
    node_id: Option<String>,
    /// Registered file stem or full path; optional for a single file.
    file_key: Option<String>,
    /// Maximum child depth, with the requested root at depth zero.
    depth: Option<u32>,
}

#[tool_router(router = metadata_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "Get sparse XML metadata: node IDs, names, types, parent-relative positions and sizes. Rotation is degrees, counter-clockwise positive (Figma convention). Call without nodeId for visible pages. Canvas/document elements omit geometry; unsized nodes omit dimensions. Instances list their component's children with overrides applied, with ids like I<instance>;<child> that every tool accepts; componentId names the main component. depth limits children. Output caps at 5000 nodes and 24 kB (the server's --max-output-bytes): truncated=\"true\" on <metadata> or <pages> marks omitted nodes, and then call again with a child nodeId or a smaller depth. Text caps at 200 characters plus an ellipsis when cut. Use get_design_context for implementation details when available."
    )]
    async fn get_metadata(&self, Parameters(input): Parameters<MetadataInput>) -> CallToolResult {
        let key = match self.file_for(input.file_key.as_deref(), input.node_id.as_deref()) {
            Ok(key) => key,
            Err(error) => return tool_error(error),
        };
        let document = match self.files().open(key.as_deref()).await {
            Ok(document) => document,
            Err(error) => return tool_error(error),
        };
        let max_bytes = self.max_output_bytes();
        match tokio::task::spawn_blocking(move || {
            metadata_within(&document, input.node_id.as_deref(), input.depth, max_bytes)
        })
        .await
        {
            Ok(result) => result,
            Err(error) => tool_error(crate::error::McpError::load_task(error)),
        }
    }
}

/// Return a page list or bounded subtree as XML; invalid IDs include the page list.
pub fn metadata(doc: &Document, node_id: Option<&str>, depth: Option<u32>) -> CallToolResult {
    metadata_within(doc, node_id, depth, DEFAULT_OUTPUT_BYTES)
}

/// [`metadata`] with an explicit output byte budget (`--max-output-bytes`).
pub fn metadata_within(
    doc: &Document,
    node_id: Option<&str>,
    depth: Option<u32>,
    max_bytes: usize,
) -> CallToolResult {
    to_call(core::metadata_within(doc, node_id, depth, max_bytes))
}
