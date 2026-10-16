//! `compare_files` / `compare_page` over two registered public fixtures.
// The `zip` dev-dependency is used only to build corrupt archives for the
// fixed-error-text test.
use figstudio_mcp::FigstudioServer;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/fixtures/{name}"))
}

fn texts(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn zip_bytes(marker: &str, payload: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut buf = vec![];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(marker.to_owned(), options).unwrap();
    zip.write_all(payload).unwrap();
    zip.start_file("canvas.fig", options).unwrap();
    zip.write_all(b"junk").unwrap();
    zip.finish().unwrap();
    buf
}

fn corrupt_zip(marker: &str) -> PathBuf {
    let dir = tempfile::tempdir().unwrap().keep();
    let path = dir.join("corrupt.fig");
    let mut bytes = zip_bytes(marker, b"not a canvas");
    let at = bytes
        .windows(12)
        .position(|w| w == b"not a canvas")
        .unwrap();
    bytes[at] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    path
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: Value,
) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn compare_files_and_page_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let assets = dir.path().join("assets");
    let (a, b) = tokio::io::duplex(65536);
    let server = FigstudioServer::new(vec![fixture("structure.fig"), fixture("corners.fig")], true)
        .with_assets(figstudio_mcp::tools::design::Assets::dir(assets.clone()));
    let server_task = tokio::spawn(server.serve(a));
    let client = ().serve(b).await.unwrap();
    let server_task = server_task.await.unwrap().unwrap();

    // Self-comparison: no changes, and both artifacts exist on disk.
    let args = json!({"oldFile": "structure", "newFile": "structure"});
    let same = call(&client, "compare_files", args).await;
    assert_ne!(same.is_error, Some(true), "{same:?}");
    let summary = same.structured_content.unwrap();
    assert_eq!(summary["summary"]["pagesChanged"], 0);
    assert_eq!(summary["pagesTotal"], 0);
    assert_eq!(summary["pagesTruncated"], false);
    let report_dir = std::path::PathBuf::from(summary["reportJson"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    assert!(
        report_dir.starts_with(assets.join("compare")),
        "{report_dir:?}"
    );
    assert!(
        report_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("compare-"),
        "{report_dir:?}"
    );
    let written: Value =
        serde_json::from_str(&std::fs::read_to_string(report_dir.join("report.json")).unwrap())
            .unwrap();
    // report.json keeps the diff crate's snake_case Summary serialization.
    assert_eq!(written["summary"]["pages_changed"], 0);
    assert!(report_dir.join("report.html").exists());

    // Different files: bounded inline summary, full detail via compare_page.
    let args = json!({"oldFile": "structure", "newFile": "corners"});
    let diff = call(&client, "compare_files", args).await;
    assert_ne!(diff.is_error, Some(true), "{diff:?}");
    let summary = diff.structured_content.unwrap();
    // Literal counts for this fixture pair (probed from the diff crate, see F6 note).
    assert_eq!(summary["summary"]["pagesChanged"], 2);
    assert_eq!(summary["summary"]["pagesAdded"], 0);
    assert_eq!(summary["summary"]["pagesRemoved"], 0);
    assert_eq!(summary["summary"]["nodesAdded"], 10);
    assert_eq!(summary["summary"]["nodesRemoved"], 23);
    assert_eq!(summary["summary"]["nodesChanged"], 3);
    assert_eq!(summary["pagesTotal"], 2);
    assert_eq!(summary["old"]["pageCount"], 2);
    assert_eq!(summary["new"]["pageCount"], 2);
    assert_eq!(summary["old"]["nodeCount"], 26);
    assert_eq!(summary["new"]["nodeCount"], 13);
    let page_id = summary["pages"][0]["id"].as_str().unwrap().to_owned();
    let page = call(
        &client,
        "compare_page",
        json!({"oldFile": "structure", "newFile": "corners", "pageId": page_id}),
    )
    .await;
    assert_ne!(page.is_error, Some(true), "{page:?}");
    let page = page.structured_content.unwrap();
    assert_eq!(page["id"], summary["pages"][0]["id"]);
    // Concrete detail: first page has 10 added nodes; first is a VECTOR root.
    assert_eq!(page["status"], "changed");
    let added = page["added"].as_array().unwrap();
    assert_eq!(added.len(), 10);
    assert_eq!(added[0]["id"], "2051:16");
    assert_eq!(added[0]["kind"], "VECTOR");
    assert_eq!(added[0]["descendants"], 0);
    assert_eq!(page["old_name"], "Page 1");
    assert_eq!(page["new_name"], "Page 1");

    client.cancel().await.unwrap();
    server_task.cancel().await.unwrap();
}

#[tokio::test]
async fn corrupt_inputs_get_fixed_error_text() {
    let marker = "review-marker-member.fig";
    let corrupt = corrupt_zip(marker);
    let valid = fixture("structure.fig");
    let (a, b) = tokio::io::duplex(65536);
    let server = FigstudioServer::new(vec![valid, corrupt], true);
    let server_task = tokio::spawn(server.serve(a));
    let client = ().serve(b).await.unwrap();
    let server_task = server_task.await.unwrap().unwrap();

    for (args, expected) in [
        (
            json!({"oldFile": "corrupt", "newFile": "structure"}),
            "cannot read the older file",
        ),
        (
            json!({"oldFile": "structure", "newFile": "corrupt"}),
            "cannot read the newer file",
        ),
    ] {
        let result = call(&client, "compare_files", args).await;
        assert_eq!(result.is_error, Some(true), "{result:?}");
        let text = texts(&result);
        assert_eq!(text, expected);
        assert!(!text.contains(marker), "leaked marker: {text}");
        assert!(!text.contains("Invalid"), "leaked detail: {text}");
    }

    for (args, expected) in [
        (
            json!({"oldFile": "corrupt", "newFile": "structure", "pageId": "0:1"}),
            "cannot read the older file",
        ),
        (
            json!({"oldFile": "structure", "newFile": "corrupt", "pageId": "0:1"}),
            "cannot read the newer file",
        ),
    ] {
        let result = call(&client, "compare_page", args).await;
        assert_eq!(result.is_error, Some(true), "{result:?}");
        let text = texts(&result);
        assert_eq!(text, expected);
        assert!(!text.contains(marker), "leaked marker: {text}");
    }

    // Ambiguous-key errors are fixed strings and must not echo the key.
    let result = call(
        &client,
        "compare_files",
        json!({"oldFile": marker, "newFile": "structure"}),
    )
    .await;
    assert_eq!(result.is_error, Some(true), "{result:?}");
    let text = texts(&result);
    assert_eq!(
        text,
        "fileKey is missing, unknown, or ambiguous; use a listed full path"
    );
    assert!(!text.contains(marker), "leaked key: {text}");

    client.cancel().await.unwrap();
    server_task.cancel().await.unwrap();
}

#[tokio::test]
async fn artifact_dir_ignores_planted_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let assets = dir.path().join("assets");
    let legacy = assets.join("compare/structure-vs-structure");
    let victim = dir.path().join("victim");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::create_dir_all(&victim).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&victim, &legacy).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&victim, &legacy).unwrap();

    let (a, b) = tokio::io::duplex(65536);
    let server = FigstudioServer::new(vec![fixture("structure.fig")], true)
        .with_assets(figstudio_mcp::tools::design::Assets::dir(assets.clone()));
    let server_task = tokio::spawn(server.serve(a));
    let client = ().serve(b).await.unwrap();
    let server_task = server_task.await.unwrap().unwrap();
    let args = json!({"oldFile": "structure", "newFile": "structure"});
    let result = call(&client, "compare_files", args).await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let summary = result.structured_content.unwrap();
    let report_json = std::path::PathBuf::from(summary["reportJson"].as_str().unwrap());
    assert!(report_json.is_file(), "{report_json:?}");
    assert!(
        report_json.starts_with(assets.join("compare")),
        "{report_json:?}"
    );
    assert!(
        report_json
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("compare-"),
        "{report_json:?}"
    );
    assert!(
        summary["reportHtml"]
            .as_str()
            .unwrap()
            .ends_with("report.html")
    );
    assert!(
        !victim.join("report.json").exists(),
        "write followed planted symlink"
    );
    assert!(!victim.join("report.html").exists());

    // A second identical request gets a different isolated dir.
    let again = call(
        &client,
        "compare_files",
        json!({"oldFile": "structure", "newFile": "structure"}),
    )
    .await;
    let again = again.structured_content.unwrap();
    assert_ne!(summary["reportJson"], again["reportJson"]);

    client.cancel().await.unwrap();
    server_task.cancel().await.unwrap();
}

#[tokio::test]
async fn renamed_frame_pair_reports_literal_change() {
    // Controlled pair: the structure fixture with its first page frame renamed.
    use figstudio_fig::Val;
    let dir = tempfile::tempdir().unwrap();
    let mut doc = figstudio_fig::Document::open(fixture("structure.fig")).unwrap();
    let page = doc.pages[0];
    let mut nodes = doc.nodes.clone();
    let frame = nodes
        .iter_mut()
        .find(|n| n.kind == "FRAME" && n.parent == Some(page))
        .expect("fixture frame");
    frame.name = "mcp-compare-renamed-frame".into();
    frame
        .data
        .set("name", Val::Str("mcp-compare-renamed-frame".into()));
    doc.set_nodes(nodes).unwrap();
    let newer = dir.path().join("newer.fig");
    doc.save(&newer).unwrap();
    let assets = dir.path().join("assets");
    let (a, b) = tokio::io::duplex(65536);
    let server = FigstudioServer::new(vec![fixture("structure.fig"), newer], true)
        .with_assets(figstudio_mcp::tools::design::Assets::dir(assets));
    let server_task = tokio::spawn(server.serve(a));
    let client = ().serve(b).await.unwrap();
    let server_task = server_task.await.unwrap().unwrap();
    let summary = call(
        &client,
        "compare_files",
        json!({"oldFile": "structure", "newFile": "newer"}),
    )
    .await;
    assert_ne!(summary.is_error, Some(true), "{summary:?}");
    let summary = summary.structured_content.unwrap();
    assert_eq!(summary["summary"]["pagesChanged"], 1);
    assert_eq!(summary["summary"]["nodesChanged"], 1);
    assert_eq!(summary["summary"]["propertiesChanged"], 1);
    assert_eq!(summary["summary"]["nodesAdded"], 0);
    assert_eq!(summary["summary"]["nodesRemoved"], 0);
    assert_eq!(summary["pagesTotal"], 1);
    let page_id = summary["pages"][0]["id"].as_str().unwrap().to_owned();
    let page = call(
        &client,
        "compare_page",
        json!({"oldFile": "structure", "newFile": "newer", "pageId": page_id}),
    )
    .await;
    assert_ne!(page.is_error, Some(true), "{page:?}");
    let page = page.structured_content.unwrap();
    let changed = page["changed"].as_array().unwrap();
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0]["id"], "1:2");
    assert_eq!(changed[0]["kind"], "FRAME");
    assert_eq!(changed[0]["changes"][0]["field"], "name");
    assert_eq!(changed[0]["changes"][0]["new"], "mcp-compare-renamed-frame");
    assert!(changed[0]["changes"][0]["old"].is_string());
    assert_eq!(changed[0]["changes"][0]["category"], "Other");
    client.cancel().await.unwrap();
    server_task.cancel().await.unwrap();
}

#[tokio::test]
async fn duplicate_stems_get_distinct_artifact_dirs() {
    use figstudio_fig::{Document, Val};
    fn marker_doc(dir: &std::path::Path, name: &str, label: &str) -> PathBuf {
        let mut doc = Document::open(fixture("structure.fig")).unwrap();
        let page = doc.pages[0];
        let mut nodes = doc.nodes.clone();
        let frame = nodes
            .iter_mut()
            .find(|n| n.kind == "FRAME" && n.parent == Some(page))
            .expect("fixture frame");
        frame.name = label.into();
        frame.data.set("name", Val::Str(label.into()));
        doc.set_nodes(nodes).unwrap();
        let sub = dir.join(name);
        std::fs::create_dir_all(&sub).unwrap();
        let path = sub.join("dup.fig");
        doc.save(&path).unwrap();
        path
    }
    let dir = tempfile::tempdir().unwrap();
    let older = marker_doc(dir.path(), "a", "dup-before-marker");
    let newer = marker_doc(dir.path(), "b", "dup-after-marker");
    let assets = dir.path().join("assets");
    let (a, b) = tokio::io::duplex(65536);
    let server = FigstudioServer::new(vec![older.clone(), newer.clone()], true)
        .with_assets(figstudio_mcp::tools::design::Assets::dir(assets.clone()));
    let server_task = tokio::spawn(server.serve(a));
    let client = ().serve(b).await.unwrap();
    let server_task = server_task.await.unwrap().unwrap();
    let old_arg = older.to_string_lossy().into_owned();
    let new_arg = newer.to_string_lossy().into_owned();

    let forward = call(
        &client,
        "compare_files",
        json!({"oldFile": old_arg, "newFile": new_arg}),
    )
    .await;
    assert_ne!(forward.is_error, Some(true), "{forward:?}");
    let forward = forward.structured_content.unwrap();
    let reverse = call(
        &client,
        "compare_files",
        json!({"oldFile": new_arg, "newFile": old_arg}),
    )
    .await;
    assert_ne!(reverse.is_error, Some(true), "{reverse:?}");
    let reverse = reverse.structured_content.unwrap();
    assert_ne!(
        forward["reportJson"], reverse["reportJson"],
        "artifact dirs collide"
    );
    // Rendered artifacts also stay in each request's own dir (paths, not stems).
    let rendered = call(
        &client,
        "compare_files",
        json!({"oldFile": old_arg, "newFile": new_arg, "includeRender": true}),
    )
    .await;
    assert_ne!(rendered.is_error, Some(true), "{rendered:?}");
    let rendered = rendered.structured_content.unwrap();
    let dir_r = std::path::PathBuf::from(rendered["reportJson"].as_str().unwrap())
        .parent()
        .unwrap()
        .to_path_buf();
    assert!(dir_r.join("report.json").is_file());
    assert_ne!(forward["reportJson"], rendered["reportJson"]);

    // Ambiguous stem alone is rejected, not silently resolved.
    let ambiguous = call(
        &client,
        "compare_files",
        json!({"oldFile": "dup", "newFile": new_arg}),
    )
    .await;
    assert_eq!(ambiguous.is_error, Some(true), "{ambiguous:?}");

    client.cancel().await.unwrap();
    server_task.cancel().await.unwrap();
}
