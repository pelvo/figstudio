//! `cap_rmcp_logs` keeps rmcp's request and response dumps out of `RUST_LOG=debug`.
use tracing::Level;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt};

fn rmcp_debug_enabled(spec: &str) -> bool {
    let filter = figstudio_mcp::cap_rmcp_logs(EnvFilter::new(spec), Some(spec));
    let subscriber = tracing_subscriber::registry().with(filter);
    tracing::subscriber::with_default(
        subscriber,
        || tracing::enabled!(target: "rmcp::service", Level::DEBUG),
    )
}

#[test]
fn rmcp_debug_is_capped_unless_named() {
    assert!(!rmcp_debug_enabled("debug"));
    assert!(!rmcp_debug_enabled("trace"));
    assert!(!rmcp_debug_enabled("figstudio=debug,debug"));
    assert!(rmcp_debug_enabled("debug,rmcp=debug"));
}
