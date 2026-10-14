use anyhow::Context;
use clap::Parser;
use figstudio_mcp::{FigstudioServer, access::HttpAccess, serve_http_with, tools::design::Assets};
use rmcp::ServiceExt;
use std::{net::SocketAddr, ops::RangeInclusive, path::PathBuf};
use tracing_subscriber::{EnvFilter, prelude::*};

/// Accepted `--max-output-bytes`: 4 kB (one element and its closing tags always fit) to
/// 100 kB (the largest size any measured client accepted was 74 kB).
const MAX_OUTPUT_BYTES_RANGE: RangeInclusive<u64> = 4_000..=100_000;

#[derive(Debug, Parser)]
#[command(version, about = "Offline MCP server for local Figma files")]
struct Args {
    /// Serve /mcp on 127.0.0.1:3845, or the next free port up to 3855. --http=ADDR binds
    /// exactly ADDR and never falls back.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "")]
    http: Option<Http>,
    /// Disable installed fonts for subsequent rendering tools.
    #[arg(long)]
    no_system_fonts: bool,
    /// Directory for get_design_context's PNG assets (default ./figma-assets).
    #[arg(long, value_name = "DIR", conflicts_with = "assets")]
    assets_dir: Option<PathBuf>,
    /// `inline`: return assets as image content instead of writing files.
    #[arg(long, value_parser = ["inline"])]
    assets: Option<String>,
    /// Byte cap of get_metadata, get_design_context and get_figjam text (default 24000).
    /// Claude Code drops replies above about 25k tokens; keep this under that.
    #[arg(
        long,
        value_name = "N",
        default_value_t = figstudio_mcp::tools::metadata::DEFAULT_OUTPUT_BYTES,
        value_parser = clap::builder::RangedU64ValueParser::<usize>::new()
            .range(MAX_OUTPUT_BYTES_RANGE)
    )]
    max_output_bytes: usize,
    /// Extra `Host` name accepted on a non-loopback `--http=ADDR` (repeatable), for example a
    /// Tailscale name. The bound IP is accepted automatically.
    #[arg(long, value_name = "NAME", requires = "http")]
    allow_host: Vec<String>,
    /// Allow `--http=0.0.0.0:PORT` or `[::]:PORT`; the default refuses to listen everywhere.
    #[arg(long, requires = "http")]
    allow_any_interface: bool,
    /// Pair a browser with a local live MCP server (no FILE required).
    #[arg(long, conflicts_with_all = ["files", "assets", "assets_dir"])]
    bridge: bool,
    /// Additional exact browser Origin allowed to pair.
    #[arg(long, requires = "bridge")]
    allow_origin: Vec<String>,
    /// Maximum bytes per browser upload (default 2 GiB).
    #[arg(long, requires = "bridge", default_value_t = figstudio_mcp::bridge::DEFAULT_MAX_UPLOAD_BYTES)]
    max_upload_bytes: u64,
    #[arg(required_unless_present = "bridge", value_name = "FILE.fig")]
    files: Vec<PathBuf>,
}

/// Where `--http` listens.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Http {
    /// Bare `--http`: the first free one of [`figstudio_mcp::default_http_addrs`].
    Default,
    /// `--http=ADDR`.
    Exact(SocketAddr),
}

impl std::str::FromStr for Http {
    type Err = std::net::AddrParseError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            Ok(Self::Default)
        } else {
            value.parse().map(Self::Exact)
        }
    }
}

/// Logs go to stderr only: stdout is the MCP stdio transport. `RUST_LOG` overrides the default.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("warn,figstudio_mcp=info"));
    let fmt = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_level(false)
        .with_target(false)
        .without_time();
    tracing_subscriber::registry()
        .with(fmt.with_filter(figstudio_mcp::cap_rmcp(
            filter,
            std::env::var("RUST_LOG").ok().as_deref(),
        )))
        .init();
}

fn main() -> anyhow::Result<()> {
    init_tracing();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;
    let result = runtime.block_on(run());
    // Tokio's blocking stdin read cannot be cancelled when a client keeps its pipe open.
    runtime.shutdown_background();
    result
}

async fn run() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.bridge {
        return run_bridge(&args).await;
    }
    let assets = match (args.assets, args.assets_dir) {
        (Some(_), _) => Assets::Inline,
        (None, Some(dir)) => Assets::dir(dir),
        (None, None) => Assets::default(),
    };
    let server = FigstudioServer::new(args.files, args.no_system_fonts)
        .with_assets(assets)
        .with_max_output_bytes(args.max_output_bytes);
    if let Some(http) = args.http {
        let listener = match http {
            Http::Exact(addr) => tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("failed to bind {addr}"))?,
            Http::Default => figstudio_mcp::bind_first(figstudio_mcp::default_http_addrs())
                .await
                .context("no free port for --http")?,
        };
        // Program output, not a log line: clients read the port from it, so RUST_LOG must not hide it.
        eprintln!("listening on http://{}/mcp", listener.local_addr()?);
        let access = access_for(&listener, &args.allow_host, args.allow_any_interface)?;
        let shutdown = tokio_util::sync::CancellationToken::new();
        let cancel = shutdown.clone();
        let signal = tokio::spawn(async move {
            if let Err(error) = shutdown_signal().await {
                tracing::error!("shutdown signal error: {error}");
            }
            cancel.cancel();
        });
        let result = serve_http_with(listener, server, shutdown, access).await;
        signal.abort();
        result.context("HTTP server failed")?;
    } else {
        let service = server
            .serve(rmcp::transport::stdio())
            .await
            .context("failed to start the stdio MCP session")?;
        let cancel = service.cancellation_token();
        let mut waiting = Box::pin(service.waiting());
        tokio::select! {
            result = &mut waiting => { result?; }
            result = shutdown_signal() => { result?; cancel.cancel(); waiting.await?; }
        }
    }
    Ok(())
}

async fn run_bridge(args: &Args) -> anyhow::Result<()> {
    use figstudio_mcp::bridge::{
        BridgeConfig, DEFAULT_ORIGINS, loopback_companions, pairing_token, serve_bridge,
    };
    let listener = match args.http {
        Some(Http::Exact(addr)) => tokio::net::TcpListener::bind(addr).await?,
        _ => figstudio_mcp::bind_first(figstudio_mcp::default_http_addrs()).await?,
    };
    let access = access_for(&listener, &args.allow_host, args.allow_any_interface)?;
    let token = pairing_token()?;
    let addr = listener.local_addr()?;
    // Everything is bound before the banner, so a client that reads it can connect at once.
    let companions = loopback_companions(&listener).await?;
    eprintln!("MCP: http://{addr}/mcp");
    for line in figstudio_mcp::bridge::web_banner(&args.allow_origin, &token) {
        eprintln!("{line}");
    }
    eprintln!("Local: http://127.0.0.1:8765/#mcp={token}");
    eprintln!(
        "Bridge port: {} (set the web app URL if not 3845)",
        addr.port()
    );
    for local in &companions {
        eprintln!("Browser bridge: ws://{}/bridge", local.local_addr()?);
    }
    let mut origins = DEFAULT_ORIGINS
        .iter()
        .map(|origin| (*origin).into())
        .collect::<Vec<_>>();
    origins.extend(args.allow_origin.clone());
    let shutdown = tokio_util::sync::CancellationToken::new();
    let cancel = shutdown.clone();
    let signal = tokio::spawn(async move {
        let _ = shutdown_signal().await;
        cancel.cancel();
    });
    let result = serve_bridge(
        listener,
        BridgeConfig {
            token,
            origins,
            max_upload_bytes: args.max_upload_bytes,
        },
        args.no_system_fonts,
        args.max_output_bytes,
        companions,
        shutdown,
        access,
    )
    .await;
    signal.abort();
    result?;
    Ok(())
}

/// Access rules for the bound listener; a non-loopback one prints its bearer token and how
/// to hand it to a client.
fn access_for(
    listener: &tokio::net::TcpListener,
    allow_host: &[String],
    allow_any_interface: bool,
) -> anyhow::Result<HttpAccess> {
    let addr = listener.local_addr()?;
    let access = HttpAccess::for_bind(addr, allow_host, allow_any_interface)?;
    if let Some(token) = access.token() {
        eprintln!("Remote access: Authorization: Bearer {token}");
        eprintln!("Plain HTTP, no TLS: use Tailscale or an SSH tunnel, not an open network.");
    }
    if let Some(hint) = figstudio_mcp::access::wildcard_hint(addr, allow_host) {
        eprintln!("{hint}");
    }
    Ok(access)
}

async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_transport_and_font_options() {
        let args = Args::try_parse_from([
            "figstudio-mcp",
            "--http",
            "example.fig",
            "--no-system-fonts",
        ])
        .unwrap();
        assert_eq!(args.http.unwrap(), Http::Default);
        assert!(args.no_system_fonts);
        let args = Args::try_parse_from(["figstudio-mcp", "--http=127.0.0.1:4567", "example.fig"])
            .unwrap();
        assert_eq!(
            args.http.unwrap(),
            Http::Exact("127.0.0.1:4567".parse().unwrap())
        );
        assert_eq!(args.files, vec![PathBuf::from("example.fig")]);
        assert!(Args::try_parse_from(["figstudio-mcp"]).is_err());
        let args = Args::try_parse_from(["figstudio-mcp", "--assets-dir", "out", "a.fig"]).unwrap();
        assert_eq!(args.assets_dir, Some(PathBuf::from("out")));
        let args = Args::try_parse_from(["figstudio-mcp", "--assets", "inline", "a.fig"]).unwrap();
        assert_eq!(args.assets.as_deref(), Some("inline"));
        assert!(Args::try_parse_from(["figstudio-mcp", "--assets", "other", "a.fig"]).is_err());
        assert!(
            Args::try_parse_from([
                "figstudio-mcp",
                "--assets",
                "inline",
                "--assets-dir",
                "d",
                "a.fig"
            ])
            .is_err()
        );
        assert!(Args::try_parse_from(["figstudio-mcp", "--http=invalid", "example.fig"]).is_err());
        assert!(
            Args::try_parse_from(["figstudio-mcp", "example.fig"])
                .unwrap()
                .http
                .is_none()
        );
    }

    #[test]
    fn max_output_bytes_defaults_to_24k_and_is_range_checked() {
        let args = Args::try_parse_from(["figstudio-mcp", "a.fig"]).unwrap();
        assert_eq!(args.max_output_bytes, 24_000);
        let args = Args::try_parse_from(["figstudio-mcp", "--max-output-bytes", "60000", "a.fig"])
            .unwrap();
        assert_eq!(args.max_output_bytes, 60_000);
        for bad in ["3999", "100001", "x"] {
            assert!(
                Args::try_parse_from(["figstudio-mcp", "--max-output-bytes", bad, "a.fig"])
                    .is_err(),
                "{bad}"
            );
        }
    }
}
