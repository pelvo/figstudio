//! The Code Connect sidecar is locked across processes. A child process that adds a mapping
//! must wait while another process holds the sibling `<sidecar>.lock`, and keep every mapping
//! when several children write at once.
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::FigstudioServer;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PROCESSES: u32 = 4;
const PER_PROCESS: u32 = 10;

fn obj(fields: &[(&str, Val)]) -> Val {
    let mut out = Obj::new();
    for (key, value) in fields {
        out.insert(key, value.clone());
    }
    Val::Obj(out)
}

/// A copy of structure.fig plus `count` local components with ids 9:100 onward.
fn library(file: &Path, count: u32) {
    let mut doc = Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    let mut nodes = doc.nodes.clone();
    for index in 0..count {
        nodes.push(
            Node::from_data(obj(&[
                ("guid", Guid(9, 100 + index).to_val()),
                (
                    "parentIndex",
                    obj(&[
                        ("guid", Guid(0, 1).to_val()),
                        ("position", Val::Str("!".into())),
                    ]),
                ),
                ("type", Val::Enum("SYMBOL")),
                ("name", Val::Str(format!("Component {index}"))),
            ]))
            .unwrap(),
        );
    }
    doc.set_nodes(nodes).unwrap();
    doc.save(file).expect("save");
}

fn mapping(id: u32) -> Value {
    json!({"nodeId": format!("9:{id}"), "componentName": format!("C{id}"),
        "source": format!("src/c{id}.tsx"), "label": "React"})
}

fn lock_path(file: &Path) -> PathBuf {
    let mut name = file.as_os_str().to_os_string();
    name.push(".codeconnect.json.lock");
    PathBuf::from(name)
}

/// Child role: add `PER_PROCESS` mappings through this process's own server.
async fn add_many(file: PathBuf, first: u32) {
    let (a, b) = tokio::io::duplex(65536);
    let server = tokio::spawn(FigstudioServer::new(vec![file], true).serve(a));
    let client = ().serve(b).await.unwrap();
    let _server = server.await.unwrap().unwrap();
    for n in 0..PER_PROCESS {
        let result = client
            .call_tool(
                CallToolRequestParams::new("add_code_connect_map")
                    .with_arguments(mapping(first + n).as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{result:?}");
    }
}

fn spawn_child(file: &Path, role: &str, first: u32) -> std::process::Child {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "child_role", "--nocapture"])
        .env("FIGSTUDIO_LOCK_ROLE", role)
        .env("FIGSTUDIO_LOCK_FILE", file)
        .env("FIGSTUDIO_LOCK_FIRST", first.to_string())
        .spawn()
        .unwrap()
}

/// Runs only when spawned by a test below; otherwise it is a no-op.
#[tokio::test]
async fn child_role() {
    let Ok(role) = std::env::var("FIGSTUDIO_LOCK_ROLE") else {
        return;
    };
    let file = PathBuf::from(std::env::var("FIGSTUDIO_LOCK_FILE").unwrap());
    let first: u32 = std::env::var("FIGSTUDIO_LOCK_FIRST")
        .unwrap()
        .parse()
        .unwrap();
    assert!(role == "add", "unknown role {role}");
    add_many(file, first).await;
}

#[tokio::test]
async fn a_held_sidecar_lock_makes_another_process_wait() {
    use std::fs::OpenOptions;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("local.fig");
    library(&file, PER_PROCESS);
    // Another process holds the lock: take the same sibling file exclusively here.
    let held = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path(&file))
        .unwrap();
    held.lock().unwrap();
    let mut child = spawn_child(&file, "add", 100);
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        child.try_wait().unwrap().is_none(),
        "the child finished while the lock was held"
    );
    held.unlock().unwrap();
    let started = Instant::now();
    assert!(child.wait().unwrap().success());
    assert!(started.elapsed() < Duration::from_secs(30));
    let saved: Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("local.fig.codeconnect.json")).unwrap(),
    )
    .unwrap();
    assert!(saved.get("9:100").is_some(), "{saved}");
}

#[tokio::test]
async fn concurrent_processes_keep_every_mapping() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("local.fig");
    library(&file, PROCESSES * PER_PROCESS);
    let mut children: Vec<_> = (0..PROCESSES)
        .map(|index| spawn_child(&file, "add", 100 + index * PER_PROCESS))
        .collect();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    let saved: Value = serde_json::from_slice(
        &std::fs::read(dir.path().join("local.fig.codeconnect.json")).unwrap(),
    )
    .unwrap();
    let kept = saved.as_object().unwrap().len();
    assert_eq!(
        kept,
        (PROCESSES * PER_PROCESS) as usize,
        "lost updates: kept {kept}"
    );
}
