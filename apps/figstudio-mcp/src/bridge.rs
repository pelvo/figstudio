//! Loopback HTTP + authenticated WebSocket, backed by unchanged live tools.
use crate::{
    FigstudioServer,
    live::{Command, LiveFiles, Snapshot},
};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use tokio_util::sync::CancellationToken;

/// Default maximum size of one uploaded file (2 GiB).
pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Origins allowed to pair with the bridge by default.
pub const DEFAULT_ORIGINS: [&str; 3] = [
    "https://figstudio.vtz.io",
    "http://127.0.0.1:8765",
    "http://localhost:8765",
];

/// Pairing configuration. Debug deliberately omits the secret token.
#[derive(Clone)]
pub struct BridgeConfig {
    /// Secret obtained from `pairing_token`.
    pub token: String,
    /// Exact browser origins; missing Origin is rejected too.
    pub origins: Vec<String>,
    /// Maximum bytes per upload.
    pub max_upload_bytes: u64,
}
impl std::fmt::Debug for BridgeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BridgeConfig")
            .field("max_upload_bytes", &self.max_upload_bytes)
            .finish_non_exhaustive()
    }
}

/// Generate an unpredictable 256-bit pairing secret.
///
/// # Errors
/// Returns the operating system randomness error, never a fallback token.
pub fn pairing_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(crate) struct Bridge {
    pub config: BridgeConfig,
    pub snapshot: Arc<RwLock<Arc<Snapshot>>>,
    pub commands: Mutex<std::sync::mpsc::Receiver<Command>>,
    pub wake: Arc<tokio::sync::Notify>,
    pub owner: tokio::sync::Mutex<()>,
    pub connection: Mutex<Option<CancellationToken>>,
    pub shutdown: CancellationToken,
}
impl Bridge {
    pub fn publish(&self, snapshot: Snapshot) {
        *self
            .snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(snapshot);
    }
}

/// Serve browser bridge and MCP on the same bound listener. `/mcp` follows `access`; `/bridge`
/// is guarded by its Origin allowlist and pairing token whatever the bind.
///
/// # Errors
/// Returns HTTP listener errors.
pub async fn serve_bridge(
    listener: tokio::net::TcpListener,
    config: BridgeConfig,
    no_system_fonts: bool,
    max_output_bytes: usize,
    shutdown: CancellationToken,
    access: crate::access::HttpAccess,
) -> std::io::Result<()> {
    let snapshot = Arc::new(RwLock::new(Arc::new(Snapshot::default())));
    let (tx, rx) = std::sync::mpsc::channel();
    let wake = Arc::new(tokio::sync::Notify::new());
    let notify = wake.clone();
    let files = Arc::new(LiveFiles::new(
        snapshot.clone(),
        tx,
        Arc::new(move || notify.notify_one()),
    ));
    let bridge = Arc::new(Bridge {
        config,
        snapshot,
        commands: Mutex::new(rx),
        wake,
        owner: Default::default(),
        connection: Default::default(),
        shutdown: shutdown.clone(),
    });
    let router = axum::Router::new()
        .route("/bridge", axum::routing::get(crate::bridge_ws::upgrade))
        .with_state(bridge);
    crate::serve_http_router(
        listener,
        FigstudioServer::live(files, no_system_fonts)
            .with_assets(crate::tools::design::Assets::Inline)
            .with_max_output_bytes(max_output_bytes),
        shutdown,
        router,
        access,
    )
    .await
}
