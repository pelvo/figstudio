//! Offline MCP access to registered local Figma documents.
/// Canonical local input and loading errors.
pub mod error;
/// Lazy registered file cache.
pub mod files;
/// Accepted Figma node ID formats.
pub mod node_id;
mod prompts;
pub mod tools;

use files::FileRegistry;
use rmcp::{
    ServerHandler,
    handler::server::router::{prompt::PromptRouter, tool::ToolRouter},
    model::{Implementation, ServerCapabilities, ServerConfig},
    prompt_handler, tool_handler,
};
use std::path::PathBuf;

/// A cloneable handler sharing the lazy file registry across HTTP sessions.
#[derive(Clone, Debug)]
pub struct FigstudioServer {
    files: FileRegistry,
    no_system_fonts: bool,
    assets: tools::design::Assets,
    tool_router: ToolRouter<Self>,
    codeconnect_lock: std::sync::Arc<std::sync::Mutex<()>>,
    prompt_router: PromptRouter<Self>,
}

impl FigstudioServer {
    /// Create a handler without decoding registered files.
    pub fn new(paths: Vec<PathBuf>, no_system_fonts: bool) -> Self {
        Self {
            files: FileRegistry::new(paths),
            no_system_fonts,
            assets: tools::design::Assets::default(),
            codeconnect_lock: Default::default(),
            prompt_router: Self::prompt_router(),
            tool_router: Self::tool_router()
                + Self::metadata_router()
                + Self::screenshot_router()
                + Self::variables_router()
                + Self::design_router()
                + Self::assets_router()
                + Self::figjam_router()
                + Self::codeconnect_router()
                + Self::design_system_router(),
        }
    }

    /// Use `assets` for the PNG assets of `get_design_context`.
    pub fn with_assets(mut self, assets: tools::design::Assets) -> Self {
        self.assets = assets;
        self
    }

    /// Where `get_design_context` puts its assets.
    pub fn assets(&self) -> &tools::design::Assets {
        &self.assets
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
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for FigstudioServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_prompts().build())
            .with_server_info(Implementation::new("figstudio-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions("Offline access to local .fig files only. fileKey is optional for one file; otherwise use its stem or listed full path. nodeId accepts 1:2, 1-2, Figma URLs with node-id, and instance paths I1:2;3:4 (nodes inside an instance, with overrides applied). Writes go only to Code Connect sidecars and get_design_context asset files; the .fig is never changed. Cloud tools are unavailable.")
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
