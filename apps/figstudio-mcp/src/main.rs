use clap::Parser;
use figstudio_mcp::{FigstudioServer, serve_http};
use rmcp::ServiceExt;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Parser)]
#[command(version, about = "Offline MCP server for local Figma files")]
struct Args {
    /// Serve /mcp (default: 127.0.0.1:3845). Put bare --http after FILE.fig, or before --.
    #[arg(long, num_args = 0..=1, default_missing_value = "127.0.0.1:3845")]
    http: Option<SocketAddr>,
    /// Disable installed fonts for subsequent rendering tools.
    #[arg(long)]
    no_system_fonts: bool,
    #[arg(required = true, value_name = "FILE.fig")]
    files: Vec<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let server = FigstudioServer::new(args.files, args.no_system_fonts);
    if let Some(addr) = args.http {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let shutdown = tokio_util::sync::CancellationToken::new();
        let cancel = shutdown.clone();
        let signal = tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
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
            result = tokio::signal::ctrl_c() => { result?; cancel.cancel(); waiting.await?; }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_transport_and_font_options() {
        let args = Args::try_parse_from([
            "figstudio-mcp",
            "example.fig",
            "--http",
            "--no-system-fonts",
        ])
        .unwrap();
        assert_eq!(
            args.http.unwrap(),
            "127.0.0.1:3845".parse::<SocketAddr>().unwrap()
        );
        assert!(args.no_system_fonts);
        let args =
            Args::try_parse_from(["figstudio-mcp", "--http", "127.0.0.1:4567", "example.fig"])
                .unwrap();
        assert_eq!(args.http.unwrap().port(), 4567);
        assert_eq!(args.files, vec![PathBuf::from("example.fig")]);
        assert!(Args::try_parse_from(["figstudio-mcp"]).is_err());
        assert!(
            Args::try_parse_from(["figstudio-mcp", "--http", "invalid", "example.fig"]).is_err()
        );
        assert!(
            Args::try_parse_from(["figstudio-mcp", "example.fig"])
                .unwrap()
                .http
                .is_none()
        );
    }
}
