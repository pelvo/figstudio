//! Offline MCP access to registered local Figma documents.
/// Canonical local input and loading errors.
pub mod error;
/// Lazy registered file cache.
pub mod files;
/// Accepted Figma node ID formats.
pub mod node_id;
pub mod tools;

use files::FileRegistry;
use rmcp::{
    ServerHandler,
    handler::server::router::tool::ToolRouter,
    model::{Implementation, ServerCapabilities, ServerConfig},
    tool_handler,
};
use std::path::PathBuf;

/// A cloneable handler sharing the lazy file registry across HTTP sessions.
#[derive(Clone, Debug)]
pub struct FigstudioServer {
    files: FileRegistry,
    no_system_fonts: bool,
    tool_router: ToolRouter<Self>,
}

impl FigstudioServer {
    /// Create a handler without decoding registered files.
    pub fn new(paths: Vec<PathBuf>, no_system_fonts: bool) -> Self {
        Self {
            files: FileRegistry::new(paths),
            no_system_fonts,
            tool_router: Self::tool_router()
                + Self::metadata_router()
                + Self::screenshot_router()
                + Self::variables_router(),
        }
    }

    /// Return the registry shared by all sessions.
    pub fn files(&self) -> &FileRegistry {
        &self.files
    }

    /// Whether rendering should avoid installed fonts.
    pub fn no_system_fonts(&self) -> bool {
        self.no_system_fonts
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FigstudioServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("figstudio-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions("Offline access to local .fig files only. fileKey is optional for one file; otherwise use its stem or listed full path. nodeId accepts 1:2, 1-2, Figma URLs with node-id, and instance paths I1:2;3:4 (not yet resolved). Write and cloud tools of the official Figma server are not available.")
    }
}

/// Serve /mcp with rmcp's default sessions and localhost Host allowlist.
pub async fn serve_http(
    listener: tokio::net::TcpListener,
    server: FigstudioServer,
    shutdown: tokio_util::sync::CancellationToken,
) -> std::io::Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    let service: StreamableHttpService<FigstudioServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(server.clone()),
            Default::default(),
            StreamableHttpServerConfig::default().with_cancellation_token(shutdown.clone()),
        );
    axum::serve(listener, axum::Router::new().nest_service("/mcp", service))
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
}
