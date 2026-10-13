//! `cap_rmcp` keeps rmcp's request and response dumps out of `RUST_LOG=debug`, never turns
//! rmcp on where `RUST_LOG` left it off, and never raises the subscriber's level hint.
use std::sync::{Arc, Mutex};
use tracing::Level;
use tracing_subscriber::{
    EnvFilter, Layer, Registry,
    layer::{Filter, SubscriberExt},
};

/// The server's filter when `RUST_LOG` is unset.
const DEFAULT: &str = "warn,figstudio_mcp=info";

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

/// Whether an rmcp event at `level` is printed for this `RUST_LOG` (`None` = unset).
fn rmcp_printed(spec: Option<&str>, level: Level) -> bool {
    let buf = Buf::default();
    let out = buf.clone();
    let filter = EnvFilter::new(spec.unwrap_or(DEFAULT));
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(move || out.clone())
        .with_filter(figstudio_mcp::cap_rmcp(filter, spec));
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

/// The level hint decides which callsites stay enabled (and build their fields) before any
/// filter runs, so the cap must not raise it above the `EnvFilter`'s.
#[test]
fn the_cap_keeps_the_env_filter_level_hint() {
    for spec in [
        None,
        Some("off"),
        Some("error"),
        Some("warn"),
        Some("debug"),
    ] {
        let text = spec.unwrap_or(DEFAULT);
        let alone = Filter::<Registry>::max_level_hint(&EnvFilter::new(text));
        let capped = figstudio_mcp::cap_rmcp::<Registry>(EnvFilter::new(text), spec);
        assert_eq!(
            Filter::<Registry>::max_level_hint(&capped),
            alone,
            "{spec:?}"
        );
    }
}
