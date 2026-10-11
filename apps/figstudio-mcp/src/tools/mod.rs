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

/// Move a finished temporary file to `path` with `permissions`, or public read (0644 on unix)
/// for a new file, so the result does not depend on the process umask. Replaces a link at
/// `path` rather than following it.
pub(crate) fn persist_file(
    temp: tempfile::NamedTempFile,
    path: &std::path::Path,
    permissions: Option<std::fs::Permissions>,
) -> std::io::Result<()> {
    #[cfg(unix)]
    let permissions = permissions.or_else(|| {
        use std::os::unix::fs::PermissionsExt;
        Some(std::fs::Permissions::from_mode(0o644))
    });
    if let Some(permissions) = permissions {
        temp.as_file().set_permissions(permissions)?;
    }
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Map a local document/input error to a tool result, not a protocol failure.
pub fn tool_error(error: crate::error::McpError) -> CallToolResult {
    CallToolResult::error(vec![rmcp::model::ContentBlock::text(error.to_string())])
}
