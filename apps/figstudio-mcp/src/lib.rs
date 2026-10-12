//! Offline MCP access to registered local Figma documents.
/// Canonical local input and loading errors.
pub mod error;
mod file_for;
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
    max_output_bytes: usize,
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
            max_output_bytes: tools::metadata::DEFAULT_OUTPUT_BYTES,
            assets: tools::design::Assets::default(),
            codeconnect_lock: Default::default(),
            prompt_router: Self::prompt_router(),
            tool_router: Self::tool_router()
                + Self::metadata_router()
                + Self::screenshot_router()
                + Self::region_router()
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

    /// Set the text-output byte cap for `get_metadata`, `get_design_context` and
    /// `get_figjam` (`--max-output-bytes`).
    pub fn with_max_output_bytes(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }

    /// The text-output byte cap in effect.
    pub fn max_output_bytes(&self) -> usize {
        self.max_output_bytes
    }
}

#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for FigstudioServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().enable_prompts().build())
            .with_server_info(Implementation::new("figstudio-mcp", env!("CARGO_PKG_VERSION")))
            .with_instructions("Offline access to local .fig files only. fileKey is optional for one file; otherwise use its stem or listed full path. nodeId accepts 1:2, 1-2, Figma URLs with node-id, figstudio://<fileKey>?node-id=1-2 links, and instance paths I1:2;3:4 (nodes inside an instance, with overrides applied). A figstudio://<fileKey>?page=0-1&rect=x,y,w,h region link (from the app's Copy region) works in get_screenshot, which draws that part of the page, and in get_nodes_in_region, which lists the nodes in it. Writes go only to Code Connect sidecars and get_design_context asset files; the .fig is never changed. Cloud tools are unavailable.")
    }
}

/// Serve /mcp with rmcp's default sessions and a loopback Host and Origin allowlist.
/// Origin is enforced: a missing Origin passes, a web origin or `null` gets 403.
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
            StreamableHttpServerConfig::default()
                .with_allowed_origins([
                    "http://localhost:*",
                    "http://127.0.0.1:*",
                    "http://[::1]:*",
                ])
                .enforce_origin_validation()
                .with_cancellation_token(shutdown.clone()),
        );
    axum::serve(listener, axum::Router::new().nest_service("/mcp", service))
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
}
