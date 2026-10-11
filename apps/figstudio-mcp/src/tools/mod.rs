//! Offline discovery and read-only document tools.
/// PNG or JPG renders and original image bytes.
pub mod assets;
/// FigJam boards as metadata XML with FigJam details.
pub mod figjam;
/// Sparse XML page lists and node subtrees.
pub mod metadata;
/// PNG renders of nodes.
pub mod screenshot;
/// Used variable and style definitions.
pub mod variables;
use crate::FigstudioServer;
use rmcp::{model::CallToolResult, tool, tool_router};
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

/// Map a local document/input error to a tool result, not a protocol failure.
pub fn tool_error(error: crate::error::McpError) -> CallToolResult {
    CallToolResult::error(vec![rmcp::model::ContentBlock::text(error.to_string())])
}
