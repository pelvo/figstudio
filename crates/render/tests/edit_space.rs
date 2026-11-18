//! The editor's page transform (`figstudio_fig::edit::space`) agrees with the renderer's.

use std::path::PathBuf;

use figstudio_fig::Document;
use figstudio_fig::edit::space::world_transform;
use figstudio_render::absolute_transform;

#[test]
fn world_transform_matches_the_renderer_on_every_fixture_node() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut checked = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "fig") {
            continue;
        }
        let doc = Document::open(&path).unwrap();
        for node in &doc.nodes {
            let ours = world_transform(&doc, node.id);
            let c = absolute_transform(&doc, node.id).as_coeffs();
            let theirs = [ours.m00, ours.m10, ours.m01, ours.m11, ours.m02, ours.m12];
            for (a, b) in theirs.iter().zip(c) {
                // A file can hold a non-finite transform; both must read it the same way.
                assert!(
                    (a.is_nan() && b.is_nan()) || (a - b).abs() <= 1e-6 * b.abs().max(1.0),
                    "{}: node {} {theirs:?} != {c:?}",
                    path.display(),
                    node.id
                );
            }
            checked += 1;
        }
    }
    assert!(checked > 1000, "only {checked} nodes checked");
}
