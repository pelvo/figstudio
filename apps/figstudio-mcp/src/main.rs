use clap::Parser;
use figstudio_mcp::{FigstudioServer, serve_http, tools::design::Assets};
use rmcp::ServiceExt;
use std::{net::SocketAddr, ops::RangeInclusive, path::PathBuf};

/// Accepted `--max-output-bytes`: 4 kB (one element and its closing tags always fit) to
/// 100 kB (the largest size any measured client accepted was 74 kB).
const MAX_OUTPUT_BYTES_RANGE: RangeInclusive<u64> = 4_000..=100_000;

#[derive(Debug, Parser)]
#[command(version, about = "Offline MCP server for local Figma files")]
struct Args {
    /// Serve /mcp (default: 127.0.0.1:3845). Custom addresses use --http=ADDR.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "127.0.0.1:3845")]
    http: Option<SocketAddr>,
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
    #[arg(required = true, value_name = "FILE.fig")]
    files: Vec<PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run());
    // Tokio's blocking stdin read cannot be cancelled when a client keeps its pipe open.
    runtime.shutdown_background();
    result
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let assets = match (args.assets, args.assets_dir) {
        (Some(_), _) => Assets::Inline,
        (None, Some(dir)) => Assets::dir(dir),
        (None, None) => Assets::default(),
    };
    let server = FigstudioServer::new(args.files, args.no_system_fonts)
        .with_assets(assets)
        .with_max_output_bytes(args.max_output_bytes);
    if let Some(addr) = args.http {
        if !addr.ip().is_loopback() {
            eprintln!("warning: non-loopback bind; rmcp rejects non-loopback Host headers");
        }
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let shutdown = tokio_util::sync::CancellationToken::new();
        let cancel = shutdown.clone();
        let signal = tokio::spawn(async move {
            if let Err(error) = shutdown_signal().await {
                eprintln!("shutdown signal error: {error}");
            }
            cancel.cancel();
        });
        let result = serve_http(listener, server, shutdown).await;
        signal.abort();
        result?;
    } else {
        let service = server.serve(rmcp::transport::stdio()).await?;
        let cancel = service.cancellation_token();
        let mut waiting = Box::pin(service.waiting());
        tokio::select! {
            result = &mut waiting => { result?; }
            result = shutdown_signal() => { result?; cancel.cancel(); waiting.await?; }
        }
    }
    Ok(())
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
        assert_eq!(
            args.http.unwrap(),
            "127.0.0.1:3845".parse::<SocketAddr>().unwrap()
        );
        assert!(args.no_system_fonts);
        let args = Args::try_parse_from(["figstudio-mcp", "--http=127.0.0.1:4567", "example.fig"])
            .unwrap();
        assert_eq!(args.http.unwrap().port(), 4567);
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
