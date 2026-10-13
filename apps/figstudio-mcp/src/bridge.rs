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
pub const DEFAULT_ORIGINS: [&str; 2] = ["http://127.0.0.1:8765", "http://localhost:8765"];

/// `Web:` banner lines, one per extra https origin the user allowed with `--allow-origin`.
#[must_use]
pub fn web_banner(extra_origins: &[String], token: &str) -> Vec<String> {
    extra_origins
        .iter()
        .filter(|origin| origin.starts_with("https://"))
        .map(|origin| format!("Web: {}/#mcp={token}", origin.trim_end_matches('/')))
        .collect()
}

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

/// Serve browser bridge and MCP on the same bound listener (and on `companions`, from
/// [`loopback_companions`]). `/mcp` follows `access`; `/bridge`
/// is guarded by its Origin allowlist and pairing token whatever the bind.
///
/// # Errors
/// Returns HTTP listener errors.
pub async fn serve_bridge(
    listener: tokio::net::TcpListener,
    config: BridgeConfig,
    no_system_fonts: bool,
    max_output_bytes: usize,
    companions: Vec<tokio::net::TcpListener>,
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
    let server = FigstudioServer::live(files, no_system_fonts)
        .with_assets(crate::tools::design::Assets::Inline)
        .with_max_output_bytes(max_output_bytes);
    let mut servers = vec![crate::serve_http_router(
        listener,
        server.clone(),
        shutdown.clone(),
        router.clone(),
        access,
    )];
    for local in companions {
        servers.push(crate::serve_http_router(
            local,
            server.clone(),
            shutdown.clone(),
            router.clone(),
            crate::access::HttpAccess::loopback(),
        ));
    }
    run_all(servers, &shutdown).await
}

/// Runs the listeners together; the first error stops the others (and cancels `shutdown`)
/// instead of leaving a half-working bridge until Ctrl-C.
async fn run_all(
    servers: Vec<impl std::future::Future<Output = std::io::Result<()>>>,
    shutdown: &CancellationToken,
) -> std::io::Result<()> {
    let result = futures_util::future::try_join_all(servers).await;
    if result.is_err() {
        shutdown.cancel();
    }
    result.map(|_| ())
}

/// The browser runs on this computer, so a remote bind (for example a Tailscale address) also
/// needs loopback listeners that it pairs with: `127.0.0.1` for either family, plus `[::1]`
/// for an IPv6 bind. Loopback `/mcp` needs no token. Bind these before printing the banner.
///
/// # Errors
/// Returns the bind error of a loopback address.
pub async fn loopback_companions(
    listener: &tokio::net::TcpListener,
) -> std::io::Result<Vec<tokio::net::TcpListener>> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    let addr = listener.local_addr()?;
    if addr.ip().is_loopback() || addr.ip().is_unspecified() {
        return Ok(Vec::new());
    }
    let mut wanted = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
    if addr.is_ipv6() {
        wanted.push(IpAddr::V6(Ipv6Addr::LOCALHOST));
    }
    let mut bound = Vec::new();
    for ip in wanted {
        bound.push(tokio::net::TcpListener::bind(SocketAddr::new(ip, addr.port())).await?);
    }
    Ok(bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_first_listener_error_ends_the_others_and_cancels() {
        let shutdown = CancellationToken::new();
        let servers: Vec<std::pin::Pin<Box<dyn std::future::Future<Output = _>>>> = vec![
            Box::pin(std::future::pending()),
            Box::pin(async { Err(std::io::Error::other("bind lost")) }),
        ];
        let result = run_all(servers, &shutdown).await;
        assert!(result.is_err());
        assert!(shutdown.is_cancelled());
    }
}
