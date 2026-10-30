//! Reading the Code Connect sidecar takes no lock: it never waits for a writer and leaves no
//! `.lock` file next to the `.fig`. The store needs files (feature `fs`).
#![cfg(feature = "fs")]
use figstudio_mcp_core::codeconnect_store::{Store, mapped};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn copy_fixture(dir: &Path) -> PathBuf {
    let fig = dir.join("structure.fig");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
        &fig,
    )
    .unwrap();
    fig
}

#[test]
fn reading_mappings_leaves_no_lock_file() {
    let dir = tempfile::tempdir().unwrap();
    let fig = copy_fixture(dir.path());
    assert!(mapped(&fig).unwrap().is_empty());
    std::fs::write(
        dir.path().join("structure.fig.codeconnect.json"),
        r#"{"1:2": {"componentName": "Card", "source": "src/Card.tsx", "label": "React",
            "snippet": null}}"#,
    )
    .unwrap();
    assert_eq!(mapped(&fig).unwrap()["1:2"].component_name, "Card");
    assert!(
        !dir.path()
            .join("structure.fig.codeconnect.json.lock")
            .exists(),
        "a read must not create the lock file"
    );
}

#[test]
fn reading_mappings_does_not_wait_for_a_writer() {
    let dir = tempfile::tempdir().unwrap();
    let fig = copy_fixture(dir.path());
    let writer = Store::open(&fig).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = fig.clone();
    std::thread::spawn(move || {
        let _ = tx.send(mapped(&reader).map(|m| m.len()));
    });
    let read = rx.recv_timeout(Duration::from_secs(5));
    drop(writer);
    assert_eq!(
        read,
        Ok(Ok(0)),
        "the read finished while a writer held the lock"
    );
}
