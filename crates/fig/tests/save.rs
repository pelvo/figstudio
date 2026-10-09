//! `save` writes a temp file beside the target, then renames it. A failed save must not leave
//! that temp file behind, and must not touch the destination.

use std::path::PathBuf;

use figstudio_fig::Document;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}

/// A fresh empty directory under the system temp dir, unique per test.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("figstudio-save-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_failed_rename_leaves_no_temp_file_behind() {
    let dir = scratch("rename");
    // A non-empty directory where the file should go: rename(file -> directory) fails.
    let target = dir.join("out.fig");
    std::fs::create_dir_all(target.join("occupied")).unwrap();

    let doc = Document::open(fixture()).unwrap();
    assert!(
        doc.save(&target).is_err(),
        "save onto a directory must fail"
    );

    let mut leftovers: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    leftovers.sort();
    assert_eq!(
        leftovers,
        ["out.fig"],
        "temp file left behind: {leftovers:?}"
    );
    assert!(target.is_dir(), "destination must be untouched");
    let _ = std::fs::remove_dir_all(&dir);
}
