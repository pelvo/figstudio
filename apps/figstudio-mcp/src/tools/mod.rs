//! Offline document tools and local Code Connect sidecar storage.
/// PNG or JPG renders and original image bytes.
pub mod assets;
/// Local Code Connect context and mappings.
pub mod codeconnect;
/// React + Tailwind design context with rendered assets.
pub mod design;
/// Local component and token discovery.
pub mod design_system;
/// FigJam boards as metadata XML with FigJam details.
pub mod figjam;
/// Sparse XML page lists and node subtrees.
pub mod metadata;
/// Render settings and lazy image reads shared by the rendering tools.
pub mod rendering;
/// PNG renders of nodes.
pub mod screenshot;
/// Used variable and style definitions.
pub mod variables;
use crate::FigstudioServer;
use figstudio_mcp_core::output::{Block, Output};
use rmcp::{
    model::{CallToolResult, ContentBlock, ResourceContents},
    tool, tool_router,
};
use serde_json::json;

#[tool_router(vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(description = "Identify this offline server and its registered local files")]
    fn whoami(&self) -> CallToolResult {
        CallToolResult::structured(
            json!({"mode": "offline", "server": "figstudio-mcp", "files": self.files.keys()}),
        )
    }

    #[tool(
        description = "List registered files with keys, paths, and page counts when already opened"
    )]
    async fn list_files(&self) -> CallToolResult {
        CallToolResult::structured(json!({"files": self.files.list().await}))
    }
}

pub(crate) use figstudio_mcp_core::fs::persist_file;

/// Map a local document/input error to a tool result, not a protocol failure.
pub fn tool_error(error: crate::error::McpError) -> CallToolResult {
    CallToolResult::error(vec![rmcp::model::ContentBlock::text(error.to_string())])
}

/// Map a core [`Output`] to the protocol result; content blocks keep their order.
pub fn to_call(output: Output) -> CallToolResult {
    let content = output
        .content
        .into_iter()
        .map(|block| match block {
            Block::Text(text) => ContentBlock::text(text),
            Block::Image { data_b64, mime } => ContentBlock::image(data_b64, mime),
            Block::Blob {
                data_b64,
                uri,
                mime,
            } => ContentBlock::resource(ResourceContents::blob(data_b64, uri).with_mime_type(mime)),
        })
        .collect();
    let mut result = if output.is_error {
        CallToolResult::error(content)
    } else {
        CallToolResult::success(content)
    };
    result.structured_content = output.structured;
    result
}
