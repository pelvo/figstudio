use super::{
    artifacts::{artifact_dir, sweep_old_artifacts},
    summary,
    tests::{document, node},
};
use crate::tools::design::Assets;
use figstudio_fig::{Guid, Node, Obj, Val};
use std::time::{Duration, SystemTime};

#[test]
fn page_json_caps_large_entries_without_losing_totals() {
    let old = document(vec![node(2, Some(1), "FRAME", &"a".repeat(70_000))]);
    let new = document(vec![
        node(2, Some(1), "FRAME", "after"),
        node(3, Some(1), "FRAME", "added"),
    ]);
    let page = summary::page_json(&old, &new, Guid(0, 1)).unwrap();
    assert!(serde_json::to_vec(&page).unwrap().len() <= 65_536);
    assert_eq!(page["truncated"], true);
    for (field, total) in [("added", 1), ("removed", 0), ("changed", 1)] {
        assert_eq!(page[format!("{field}Total")], total);
        assert_eq!(
            page[field].as_array().unwrap().len() as u64
                + page[format!("{field}Omitted")].as_u64().unwrap(),
            total
        );
    }
    assert_eq!(page["id"], "0:1");
    // Metadata alone must not defeat the cap.
    let mut old = document(vec![]);
    let mut new = document(vec![]);
    for (doc, name) in [
        (&mut old, "a".repeat(70_000)),
        (&mut new, "b".repeat(70_000)),
    ] {
        doc.set_nodes(vec![
            node(0, None, "DOCUMENT", "root"),
            node(1, Some(0), "CANVAS", &name),
        ])
        .unwrap();
    }
    let page = summary::page_json(&old, &new, Guid(0, 1)).unwrap();
    assert!(serde_json::to_vec(&page).unwrap().len() <= 65_536);
    assert_eq!(page["truncated"], true);
}

#[test]
fn page_json_does_not_rewrite_guid_shaped_property_values() {
    let mut before = node(2, Some(1), "FRAME", "frame");
    before.data.set(
        "custom",
        Val::Obj(Obj::from([(
            "id",
            Val::Arr(vec![Val::Int(9), Val::Int(9)]),
        )])),
    );
    let before = Node::from_data(before.data).unwrap();
    let mut after = node(2, Some(1), "FRAME", "frame");
    after.data.set("custom", Val::Bool(false));
    let after = Node::from_data(after.data).unwrap();
    let page =
        summary::page_json(&document(vec![before]), &document(vec![after]), Guid(0, 1)).unwrap();
    let change = page["changed"][0]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == "custom")
        .unwrap();
    assert_eq!(change["old"]["id"], serde_json::json!([9, 9]));
    assert_eq!(page["changed"][0]["id"], "0:2");
}

#[test]
fn page_json_formats_only_schema_move_and_reference_guids() {
    let old = document(vec![
        node(2, Some(1), "FRAME", "frame"),
        node(3, Some(1), "FRAME", "removed"),
        node(4, Some(1), "FRAME", "parent"),
    ]);
    let new = document(vec![
        node(2, Some(4), "FRAME", "frame"),
        node(4, Some(1), "FRAME", "parent"),
        node(5, Some(1), "FRAME", "added"),
    ]);
    let page = summary::page_json(&old, &new, Guid(0, 1)).unwrap();
    assert_eq!(page["changed"][0]["moved"]["old_parent"], "0:1");
    assert_eq!(page["changed"][0]["moved"]["new_parent"], "0:4");
    assert_eq!(page["added"][0]["id"], "0:5");
    assert_eq!(page["removed"][0]["id"], "0:3");
    assert_eq!(page["truncated"], false);
    assert_eq!(page["changedTotal"], 1);
    assert_eq!(page["changedOmitted"], 0);
}

#[test]
fn artifact_sweep_removes_only_expired_compare_directories() {
    let root = tempfile::tempdir().unwrap();
    for name in ["compare-old", "keep-old"] {
        std::fs::create_dir(root.path().join(name)).unwrap();
    }
    std::fs::write(root.path().join("compare-file"), b"keep").unwrap();
    let now = SystemTime::now();
    let age = Duration::from_secs(24 * 60 * 60);
    sweep_old_artifacts(root.path(), now, age);
    assert!(root.path().join("compare-old").exists());
    sweep_old_artifacts(root.path(), now + age + Duration::from_secs(60), age);
    assert!(!root.path().join("compare-old").exists());
    assert!(root.path().join("keep-old").exists());
    assert!(root.path().join("compare-file").exists());
}

#[cfg(unix)]
#[test]
fn artifact_root_symlink_is_rejected_without_touching_target() {
    let dir = tempfile::tempdir().unwrap();
    let victim = tempfile::tempdir().unwrap();
    std::fs::write(victim.path().join("sentinel"), b"untouched").unwrap();
    std::os::unix::fs::symlink(victim.path(), dir.path().join("compare")).unwrap();
    let error = artifact_dir(&Assets::dir(dir.path().to_owned())).unwrap_err();
    assert_eq!(error.to_string(), "artifact directory unavailable");
    assert_eq!(std::fs::read_dir(victim.path()).unwrap().count(), 1);
    assert_eq!(
        std::fs::read(victim.path().join("sentinel")).unwrap(),
        b"untouched"
    );
}
