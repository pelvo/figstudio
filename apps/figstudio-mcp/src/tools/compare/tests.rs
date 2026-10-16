use super::*;
use crate::tools::design::Assets;
use figstudio_fig::{Node, Obj, Val};

pub(super) fn node(id: u32, parent: Option<u32>, kind: &str, name: &str) -> Node {
    let mut data = Val::Obj(Obj::from([
        ("guid", Guid(0, id).to_val()),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(name.into())),
    ]));
    if let Some(parent) = parent {
        data.set(
            "parentIndex",
            Val::Obj(Obj::from([
                ("guid", Guid(0, parent).to_val()),
                ("position", Val::Str("a".into())),
            ])),
        );
    }
    Node::from_data(data).unwrap()
}

pub(super) fn document(children: Vec<Node>) -> Document {
    let mut doc = Document::from_bytes(include_bytes!(
        "../../../../../tests/fixtures/structure.fig"
    ))
    .unwrap();
    let mut nodes = vec![
        node(0, None, "DOCUMENT", "root"),
        node(1, Some(0), "CANVAS", "page"),
    ];
    nodes.extend(children);
    doc.set_nodes(nodes).unwrap();
    doc
}

/// One page whose only child is renamed between the two versions.
fn changed_pair() -> (Document, Document) {
    (
        document(vec![node(2, Some(1), "FRAME", "before")]),
        document(vec![node(2, Some(1), "FRAME", "after")]),
    )
}

#[test]
fn summary_counts_and_single_page_entry() {
    let (old, new) = changed_pair();
    let report = diff_documents(&old, &new);
    let value = summarize(&report, "/tmp/x/report.json", "/tmp/x/report.html");
    assert_eq!(value["summary"]["pagesChanged"], 1);
    assert_eq!(value["summary"]["nodesChanged"], 1);
    assert_eq!(value["pagesTotal"], 1);
    assert_eq!(value["pagesTruncated"], false);
    assert_eq!(value["pages"][0]["id"], "0:1");
    assert_eq!(value["pages"][0]["status"], "changed");
    assert_eq!(value["pages"][0]["changed"], 1);
    assert_eq!(value["reportJson"], "/tmp/x/report.json");
}

#[test]
fn summary_caps_inline_pages_at_the_bound() {
    // 105 changed pages: one renamed child on each.
    let build = |child_name: &str| {
        let mut doc = Document::from_bytes(include_bytes!(
            "../../../../../tests/fixtures/structure.fig"
        ))
        .unwrap();
        let mut nodes = vec![node(0, None, "DOCUMENT", "root")];
        for i in 0..105u32 {
            let page = 1 + 10 * i;
            nodes.push(node(page, Some(0), "CANVAS", "page"));
            nodes.push(node(page + 1, Some(page), "FRAME", child_name));
        }
        doc.set_nodes(nodes).unwrap();
        doc
    };
    let report = diff_documents(&build("before"), &build("after"));
    assert_eq!(report.pages.len(), 105);
    let value = summarize(&report, "r.json", "r.html");
    assert_eq!(value["pagesTotal"], 105);
    assert_eq!(value["pagesTruncated"], true);
    assert_eq!(value["pages"].as_array().unwrap().len(), MAX_INLINE_PAGES);
    // The cap drops page entries only, never the counts.
    assert_eq!(value["summary"]["pagesChanged"], 105);
    assert_eq!(value["summary"]["nodesChanged"], 105);
}

#[test]
fn page_json_returns_the_changed_page_only() {
    let (old, new) = changed_pair();
    let page = page_json(&old, &new, Guid(0, 1)).expect("changed page");
    assert_eq!(page["status"], "changed");
    assert_eq!(page["truncated"], false);
    assert_eq!(page["changed"].as_array().unwrap().len(), 1);
    assert_eq!(page["id"], "0:1");
    assert_eq!(page["changed"][0]["id"], "0:2");
    assert!(page_json(&old, &new, Guid(0, 999)).is_none());
}

#[test]
fn page_json_stringifies_moved_out_ids() {
    use serde_json::json;
    let mut value = json!({"id": [0, 1], "moved_out": [[0, 2]]});
    stringify_guids(&mut value);
    assert_eq!(value["moved_out"][0], "0:2");
}

#[test]
fn artifact_dir_is_unique_and_under_the_assets_dir() {
    let dir = tempfile::tempdir().unwrap();
    let assets = Assets::dir(dir.path().to_path_buf());
    let first = artifact_dir(&assets).unwrap();
    let second = artifact_dir(&assets).unwrap();
    assert_ne!(first, second);
    for created in [&first, &second] {
        let text = created.display().to_string();
        assert!(
            text.starts_with(&format!("{}/compare/compare-", dir.path().display())),
            "{text}"
        );
        // Keys (even path-like ones) can never reach the name: only `compare-…`.
        assert!(!text.contains("passwd"), "{text}");
    }
    // Inline mode falls back under the temp compare root, also unique.
    let inline = artifact_dir(&Assets::Inline).unwrap();
    assert!(
        inline.display().to_string().contains("figstudio-compare"),
        "{inline:?}"
    );
    assert!(
        inline
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("compare-")
    );
}

#[test]
fn long_page_name_is_truncated_inline_only() {
    let long = "x".repeat(10_000);
    let (mut old, mut new) = changed_pair();
    // Rename the page itself in both docs so the summary entry carries it.
    for doc in [&mut old, &mut new] {
        let mut nodes = doc.nodes.clone();
        for node in &mut nodes {
            if node.kind == "CANVAS" {
                node.name = long.clone();
                node.data.set("name", Val::Str(long.clone()));
            }
        }
        doc.set_nodes(nodes).unwrap();
    }
    let report = diff_documents(&old, &new);
    let value = summarize(&report, "/tmp/x/report.json", "/tmp/x/report.html");
    let page = &value["pages"][0];
    let name = page["name"].as_str().unwrap();
    assert!(name.len() <= 128, "name bytes: {}", name.len());
    assert!(name.ends_with('~'), "{name}");
    assert_eq!(page["nameTruncated"], true);
    assert!(
        serde_json::to_string(&value).unwrap().len() < 2000,
        "inline JSON must stay bounded"
    );
    // The disk report keeps the full name.
    let disk: Value = serde_json::from_str(&report.to_json().unwrap()).unwrap();
    assert_eq!(disk["pages"][0]["new_name"].as_str().unwrap().len(), 10_000);
}

#[test]
fn short_page_name_is_not_marked_truncated() {
    let (old, new) = changed_pair();
    let report = diff_documents(&old, &new);
    let value = summarize(&report, "r.json", "r.html");
    assert_eq!(value["pages"][0]["nameTruncated"], false);
}

#[test]
fn compare_and_write_leaves_json_and_html_artifacts() {
    let (old, new) = changed_pair();
    let dir = tempfile::tempdir().unwrap().keep();
    let value = compare_and_write(
        &old, &new, "old.fig", "new.fig", &dir, false, false, None, None,
    )
    .unwrap();
    assert_eq!(value["summary"]["pagesChanged"], 1);
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("report.json")).unwrap()).unwrap();
    // report.json keeps the diff crate's snake_case Summary serialization.
    assert_eq!(written["summary"]["pages_changed"], 1);
    let html = std::fs::read_to_string(dir.join("report.html")).unwrap();
    assert!(html.contains("<!DOCTYPE html>") || html.contains("<html"));
    std::fs::remove_dir_all(&dir).unwrap();
}
