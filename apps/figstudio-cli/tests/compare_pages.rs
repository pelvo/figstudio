//! Added/removed page counts stay visible even when no content nodes change.
use figstudio_fig::{Document, Guid, Node, Obj, Val};

fn document(page: u32) -> Document {
    let mut doc =
        Document::from_bytes(include_bytes!("../../../tests/fixtures/structure.fig")).unwrap();
    let root = Val::Obj(Obj::from([
        ("guid", Guid(0, 0).to_val()),
        ("type", Val::Enum("DOCUMENT")),
    ]));
    let page = Val::Obj(Obj::from([
        ("guid", Guid(0, page).to_val()),
        ("type", Val::Enum("CANVAS")),
        (
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", Guid(0, 0).to_val()),
                ("position", Val::Str("a".into())),
            ])),
        ),
    ]));
    doc.set_nodes(vec![
        Node::from_data(root).unwrap(),
        Node::from_data(page).unwrap(),
    ])
    .unwrap();
    doc
}

#[test]
fn stdout_counts_added_and_removed_empty_pages() {
    let dir = std::env::temp_dir().join(format!("figstudio-page-counts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let old = dir.join("before.fig");
    let new = dir.join("after.fig");
    document(1).save(&old).unwrap();
    document(2).save(&new).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
        .args(["compare"])
        .arg(&old)
        .arg(&new)
        .arg("-o")
        .arg(dir.join("report"))
        .arg("--no-render")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        String::from_utf8(result.stdout)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        "pages added 1, removed 1, changed 0, nodes added 0, removed 0, changed 0, moved 0"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
