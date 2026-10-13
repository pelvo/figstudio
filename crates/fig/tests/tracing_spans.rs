//! `Document::from_bytes` reports counts at debug level and nothing from the file's content.

use figstudio_fig::Document;
use std::sync::{Arc, Mutex};
use tracing_subscriber::fmt::MakeWriter;

#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Buffer {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Buffer {
    type Writer = Buffer;
    fn make_writer(&'a self) -> Buffer {
        self.clone()
    }
}

fn capture(level: tracing::Level) -> (String, Document) {
    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(buffer.clone())
        .finish();
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/structure.fig"
    ))
    .unwrap();
    let doc =
        tracing::subscriber::with_default(subscriber, || Document::from_bytes(&bytes).unwrap());
    let out = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    (out, doc)
}

#[test]
fn from_bytes_logs_counts_and_no_layer_names() {
    let (out, doc) = capture(tracing::Level::DEBUG);
    assert!(out.contains("nodes="), "no node count in: {out}");
    assert!(out.contains("bytes="), "no byte size in: {out}");
    for node in &doc.nodes {
        let name = node.name.as_str();
        if name.len() > 3 {
            assert!(!out.contains(name), "layer name {name:?} leaked: {out}");
        }
    }
}

#[test]
fn from_bytes_is_silent_at_info() {
    let (out, _) = capture(tracing::Level::INFO);
    assert!(out.is_empty(), "info output: {out}");
}
