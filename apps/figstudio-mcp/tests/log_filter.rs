//! `rmcp_cap` keeps rmcp's request and response dumps out of `RUST_LOG=debug`, and never
//! turns rmcp on where `RUST_LOG` left it off.
use std::sync::{Arc, Mutex};
use tracing::Level;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt};

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Buf {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Whether an rmcp event at `level` is printed for this `RUST_LOG` (`None` = unset; the
/// server's default filter is `warn,figstudio_mcp=info`).
fn rmcp_printed(spec: Option<&str>, level: Level) -> bool {
    let buf = Buf::default();
    let out = buf.clone();
    let filter = EnvFilter::new(spec.unwrap_or("warn,figstudio_mcp=info"));
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(move || out.clone())
        .with_filter(filter)
        .with_filter(figstudio_mcp::rmcp_cap(spec));
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        if level == Level::DEBUG {
            tracing::debug!(target: "rmcp::service", "probe");
        } else {
            tracing::info!(target: "rmcp::service", "probe");
        }
    });
    let printed = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    printed.contains("probe")
}

#[test]
fn rmcp_debug_is_capped_unless_named() {
    assert!(!rmcp_printed(Some("debug"), Level::DEBUG));
    assert!(!rmcp_printed(Some("trace"), Level::DEBUG));
    assert!(!rmcp_printed(Some("figstudio=debug,debug"), Level::DEBUG));
    assert!(rmcp_printed(Some("debug,rmcp=debug"), Level::DEBUG));
}

#[test]
fn the_cap_never_turns_rmcp_info_on() {
    for spec in [
        None,
        Some(""),
        Some("off"),
        Some("error"),
        Some("warn"),
        Some("figstudio_fig=debug"),
    ] {
        assert!(!rmcp_printed(spec, Level::INFO), "{spec:?}");
    }
    assert!(rmcp_printed(Some("debug"), Level::INFO));
}
