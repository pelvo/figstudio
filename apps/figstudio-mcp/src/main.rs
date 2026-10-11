use clap::Parser;
use figstudio_mcp::{FigstudioServer, serve_http};
use rmcp::ServiceExt;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Parser)]
#[command(version, about = "Offline MCP server for local Figma files")]
struct Args {
    /// Serve /mcp (default: 127.0.0.1:3845). Custom addresses use --http=ADDR.
    #[arg(long, num_args = 0..=1, require_equals = true, default_missing_value = "127.0.0.1:3845")]
    http: Option<SocketAddr>,
    /// Disable installed fonts for subsequent rendering tools.
    #[arg(long)]
    no_system_fonts: bool,
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
    let server = FigstudioServer::new(args.files, args.no_system_fonts);
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
        assert!(Args::try_parse_from(["figstudio-mcp", "--http=invalid", "example.fig"]).is_err());
        assert!(
            Args::try_parse_from(["figstudio-mcp", "example.fig"])
                .unwrap()
                .http
                .is_none()
        );
    }
}
