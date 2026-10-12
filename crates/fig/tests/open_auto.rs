//! `Document::open_auto`: eager below the threshold, lazy above it, same content either way.
use std::path::PathBuf;

use figstudio_fig::{Document, LAZY_ABOVE};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.fig"))
}

#[test]
fn the_default_threshold_is_256_mib() {
    assert_eq!(LAZY_ABOVE, 256 * 1024 * 1024);
}

#[test]
fn small_files_open_eagerly_and_forced_lazy_files_list_their_images() {
    let path = fixture("broken_images");
    let eager = Document::open_auto(&path).unwrap();
    assert!(eager.lazy_images.is_empty());
    let forced = Document::open_auto_above(&path, 0).unwrap();
    assert!(
        !forced.lazy_images.is_empty(),
        "a fixture with images opens lazily"
    );
    assert!(
        forced.images.is_empty(),
        "lazy open reads no image bytes up front"
    );
    // Both open the same canvas.
    assert_eq!(eager.nodes.len(), forced.nodes.len());
}
