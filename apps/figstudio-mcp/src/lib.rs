//! Offline MCP access to registered local Figma documents.
/// Who may reach `/mcp`: loopback, or a token-guarded remote bind.
pub mod access;
/// Local browser bridge transport.
pub mod bridge;
mod bridge_upload;
mod bridge_ws;
/// Canonical local input and loading errors.
pub mod error;
mod file_for;
/// Lazy registered file cache, or the app's open tabs.
pub mod files;
/// Live mode: the app's open tabs and selection.
pub mod live;
/// Accepted Figma node ID formats.
pub mod node_id;
mod prompts;
/// Local zero-upload browser proxy (`--browser-proxy`).
pub mod proxy;
pub mod tools;
/// The `--origin` web app served from the browser proxy's loopback port.
pub mod web_mirror;

use files::{FileRegistry, Files};
use rmcp::{
    ServerHandler,
    handler::server::router::{prompt::PromptRouter, tool::ToolRouter},
    model::{Implementation, ServerCapabilities, ServerConfig},
    prompt_handler, tool_handler,
};
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::Arc,
};

/// A cloneable handler sharing the lazy file registry across HTTP sessions.
#[derive(Clone, Debug)]
pub struct FigstudioServer {
    files: Files,
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
        Self::with_files(Files::Registry(FileRegistry::new(paths)), no_system_fonts)
    }

    /// Serve the app's open tabs: a missing `fileKey` is the active tab, a missing `nodeId`
    /// its selection, and `list_open_tabs`, `get_selection` and `reveal_node` are added.
    pub fn live(files: Arc<live::LiveFiles>, no_system_fonts: bool) -> Self {
        let mut server = Self::with_files(Files::Live(files), no_system_fonts);
        server.tool_router += Self::live_router();
        server
    }

    fn with_files(files: Files, no_system_fonts: bool) -> Self {
        Self {
            files,
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
                + Self::design_system_router()
                + Self::shaders_router()
                + Self::compare_router(),
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
    pub fn files(&self) -> &Files {
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

const INSTRUCTIONS: &str = "Offline access to local .fig files only. fileKey is optional for one file; otherwise use its stem or listed full path. nodeId accepts 1:2, 1-2, Figma URLs with node-id, figstudio://<fileKey>?node-id=1-2 links, and instance paths I1:2;3:4 (nodes inside an instance, with overrides applied). A figstudio://<fileKey>?page=0-1&rect=x,y,w,h region link (from the app's Copy region) works in get_screenshot, which draws that part of the page, and in get_nodes_in_region, which lists the nodes in it. Writes go only to Code Connect sidecars, get_design_context asset files and compare_files report artifacts; the .fig is never changed. Cloud tools are unavailable.";

const LIVE_INSTRUCTIONS: &str = " Live mode: the files are the figstudio app's open tabs. A missing fileKey means the active tab; a missing nodeId means its selection. list_open_tabs lists the tabs, get_selection describes the selected node, reveal_node selects a node in the app and shows it.";

/// The ports a bare `--http` and the app try, in order: 3845, then 3846 to 3855.
pub const HTTP_PORTS: std::ops::RangeInclusive<u16> = 3845..=3855;

/// `127.0.0.1` on each of [`HTTP_PORTS`].
pub fn default_http_addrs() -> Vec<SocketAddr> {
    HTTP_PORTS
        .map(|port| SocketAddr::from((Ipv4Addr::LOCALHOST, port)))
        .collect()
}

/// Bind the first address of `candidates` that is free.
///
/// # Errors
/// The last bind error when none is free; `InvalidInput` when `candidates` is empty.
pub async fn bind_first(
    candidates: impl IntoIterator<Item = SocketAddr>,
) -> std::io::Result<tokio::net::TcpListener> {
    let mut last = std::io::Error::new(std::io::ErrorKind::InvalidInput, "no address to bind");
    for addr in candidates {
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => return Ok(listener),
            Err(error) => last = error,
        }
    }
    Err(last)
}

#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for FigstudioServer {
    fn get_info(&self) -> ServerConfig {
        let mut instructions = INSTRUCTIONS.to_owned();
        if let Files::Live(_) = self.files {
            instructions.push_str(LIVE_INSTRUCTIONS);
        }
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
        )
        .with_server_info(Implementation::new(
            "figstudio-mcp",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(instructions)
    }

    /// What `#[tool_handler]` generates, inside a debug span with the tool name (never its
    /// arguments) and the elapsed time.
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        use tracing::Instrument as _;
        let span = tracing::debug_span!("tool_call", tool = %request.name);
        async {
            let started = std::time::Instant::now();
            let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
            let result = self.tool_router.call(tcc).await;
            tracing::debug!(
                elapsed_ms = started.elapsed().as_millis() as u64,
                ok = match &result {
                    Err(_) => false,
                    Ok(rmcp::model::CallToolResponse::Complete(r)) => r.is_error != Some(true),
                    Ok(_) => true,
                },
                "tool call finished"
            );
            result
        }
        .instrument(span)
        .await
    }
}

/// Serve /mcp with rmcp's default sessions and a loopback Host and Origin allowlist.
/// Origin is enforced: a missing Origin passes, a web origin or `null` gets 403.
pub async fn serve_http(
    listener: tokio::net::TcpListener,
    server: FigstudioServer,
    shutdown: tokio_util::sync::CancellationToken,
) -> std::io::Result<()> {
    serve_http_with(listener, server, shutdown, access::HttpAccess::loopback()).await
}

/// [`serve_http`] with explicit access rules: extra `Host` names and a bearer token.
pub async fn serve_http_with(
    listener: tokio::net::TcpListener,
    server: FigstudioServer,
    shutdown: tokio_util::sync::CancellationToken,
    access: access::HttpAccess,
) -> std::io::Result<()> {
    serve_http_router(listener, server, shutdown, axum::Router::new(), access).await
}

pub(crate) async fn serve_http_router(
    listener: tokio::net::TcpListener,
    server: FigstudioServer,
    shutdown: tokio_util::sync::CancellationToken,
    extra: axum::Router,
    access: access::HttpAccess,
) -> std::io::Result<()> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    let service: StreamableHttpService<FigstudioServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(server.clone()),
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_allowed_hosts(access.hosts().iter().cloned())
                .with_allowed_origins([
                    "http://localhost:*",
                    "http://127.0.0.1:*",
                    "http://[::1]:*",
                ])
                .enforce_origin_validation()
                .with_cancellation_token(shutdown.clone()),
        );
    let mut mcp = axum::Router::new().nest_service("/mcp", service);
    if let Some(token) = access.token_arc() {
        // Only /mcp: /bridge has its own Origin and pairing-token checks.
        mcp = mcp.layer(axum::middleware::from_fn_with_state(
            token,
            access::require_bearer,
        ));
    }
    axum::serve(listener, extra.merge(mcp))
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
}

/// `filter` with rmcp capped at `info` unless `spec` (the `RUST_LOG` value) names rmcp: rmcp's
/// `debug` and `trace` events carry full requests and responses (tool arguments, layer names,
/// text). One filter that needs both (`and`), so it only ever lowers rmcp and its level hint is
/// the stricter of the two; two stacked `with_filter`s would OR and raise the hint to `TRACE`.
/// Used by the standalone server and by the app that hosts it.
pub fn cap_rmcp<S>(
    filter: tracing_subscriber::EnvFilter,
    spec: Option<&str>,
) -> tracing_subscriber::filter::combinator::And<
    tracing_subscriber::EnvFilter,
    tracing_subscriber::filter::Targets,
    S,
>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    use tracing_subscriber::filter::{FilterExt, LevelFilter, Targets};
    let all = Targets::new().with_default(LevelFilter::TRACE);
    let cap = if spec.is_some_and(|spec| spec.contains("rmcp")) {
        all
    } else {
        all.with_target("rmcp", LevelFilter::INFO)
    };
    filter.and(cap)
}
