//! Live mode over the protocol, against a fake app (snapshot from fixtures, command loop).
#[path = "live/support.rs"]
mod support;

use figstudio_fig::Guid;
use figstudio_mcp::{FigstudioServer, bind_first, live::Snapshot};
use rmcp::ServiceExt;
use serde_json::{Value, json};
use std::sync::Arc;
use support::{Answer, call, doc, fixture, start, tab, texts};

fn two_tabs() -> Snapshot {
    Snapshot {
        tabs: vec![
            tab(
                7,
                fixture("structure.fig"),
                doc("structure.fig"),
                Some(Guid(1, 25)),
            ),
            tab(9, fixture("sections.fig"), doc("sections.fig"), None),
        ],
        active: Some(7),
    }
}

fn data(result: &rmcp::model::CallToolResult) -> Value {
    assert_ne!(result.is_error, Some(true), "{}", texts(result));
    result.structured_content.clone().unwrap()
}

#[tokio::test]
async fn lists_tabs_and_selection() {
    let app = start(two_tabs(), Answer::Reply).await;
    let info = app.client.peer_info().unwrap();
    assert!(info.instructions.as_ref().unwrap().contains("Live mode"));
    let tabs = data(&app.call("list_open_tabs", json!({})).await);
    assert_eq!(tabs["tabs"][0]["tabId"], 7);
    assert_eq!(tabs["tabs"][0]["fileKey"], "structure");
    assert_eq!(tabs["tabs"][0]["title"], "structure.fig");
    assert_eq!(
        tabs["tabs"][0]["page"],
        json!({"id": "0:1", "name": "Page 1"})
    );
    assert_eq!(tabs["tabs"][0]["selection"], "1:25");
    assert_eq!(tabs["tabs"][0]["active"], true);
    assert_eq!(tabs["tabs"][1]["fileKey"], "sections");
    assert_eq!(tabs["tabs"][1]["selection"], Value::Null);
    assert_eq!(tabs["tabs"][1]["active"], false);
    let whoami = data(&app.call("whoami", json!({})).await);
    assert_eq!(whoami["mode"], "live");
    assert_eq!(whoami["files"], json!(["structure", "sections"]));

    let selection = data(&app.call("get_selection", json!({})).await);
    assert_eq!(selection["fileKey"], "structure");
    assert_eq!(selection["tabId"], 7);
    assert_eq!(selection["node"]["nodeId"], "1:25");
    assert_eq!(selection["node"]["name"], "logo-icon 1");
    assert_eq!(selection["node"]["kind"], "FRAME");
    assert!(selection["node"]["bounds"]["width"].as_f64().unwrap() > 0.0);
    let other = data(
        &app.call("get_selection", json!({"fileKey": "sections"}))
            .await,
    );
    assert_eq!(other["tabId"], 9);
    assert_eq!(other["node"], Value::Null);
}

#[tokio::test]
async fn duplicate_stems_are_keyed_by_path() {
    let path = |dir: &str| std::path::PathBuf::from(dir).join("structure.fig");
    let snapshot = Snapshot {
        tabs: vec![
            tab(1, path("a"), doc("structure.fig"), None),
            tab(2, path("b"), doc("structure.fig"), Some(Guid(1, 4))),
        ],
        active: Some(1),
    };
    let app = start(snapshot, Answer::Reply).await;
    let tabs = data(&app.call("list_open_tabs", json!({})).await);
    let key = path("b").to_string_lossy().into_owned();
    assert_eq!(tabs["tabs"][1]["fileKey"], key.as_str());
    let selection = data(&app.call("get_selection", json!({"fileKey": key})).await);
    assert_eq!(selection["tabId"], 2);
    let ambiguous = app
        .call("get_selection", json!({"fileKey": "structure"}))
        .await;
    assert_eq!(ambiguous.is_error, Some(true));
}

#[tokio::test]
async fn node_id_defaults_to_the_selection() {
    let app = start(two_tabs(), Answer::Reply).await;
    let design = app.call("get_design_context", json!({})).await;
    assert_ne!(design.is_error, Some(true), "{}", texts(&design));
    assert!(texts(&design).contains(r#"data-node-id="1:25""#));
    let shot = app.call("get_screenshot", json!({})).await;
    assert_ne!(shot.is_error, Some(true), "{}", texts(&shot));
    let explicit = app.call("get_screenshot", json!({"nodeId": "1:25"})).await;
    assert_eq!(texts(&shot), texts(&explicit));
    let vars = app.call("get_variable_defs", json!({})).await;
    assert_ne!(vars.is_error, Some(true), "{}", texts(&vars));
    let assets = app.call("download_assets", json!({"nodeIds": []})).await;
    assert!(texts(&assets).contains("1:25"), "{}", texts(&assets));
    app.replace(Snapshot {
        tabs: vec![tab(
            7,
            fixture("structure.fig"),
            doc("structure.fig"),
            Some(Guid(1, 37)),
        )],
        active: Some(7),
    });
    let context = app.call("get_context_for_code_connect", json!({})).await;
    assert_ne!(context.is_error, Some(true), "{}", texts(&context));
    assert!(
        texts(&context).contains("Component 1"),
        "{}",
        texts(&context)
    );

    // Nothing selected in the chosen tab.
    for tool in [
        "get_design_context",
        "get_screenshot",
        "get_variable_defs",
        "get_context_for_code_connect",
        "download_assets",
    ] {
        let none = app.call(tool, json!({"fileKey": "structure"})).await;
        assert_ne!(none.is_error, Some(true), "{tool}: {}", texts(&none));
    }
    app.replace(two_tabs());
    for tool in [
        "get_design_context",
        "get_screenshot",
        "get_variable_defs",
        "get_context_for_code_connect",
        "download_assets",
    ] {
        let none = app.call(tool, json!({"fileKey": "sections"})).await;
        assert_eq!(none.is_error, Some(true), "{tool}");
        assert!(
            texts(&none).contains("nothing is selected"),
            "{tool}: {}",
            texts(&none)
        );
    }
    // get_metadata keeps "no nodeId = page list".
    let pages = app.call("get_metadata", json!({})).await;
    assert!(texts(&pages).contains("<canvas"), "{}", texts(&pages));
}

#[tokio::test]
async fn standalone_requires_a_node_id() {
    let (client_io, server_io) = tokio::io::duplex(1 << 20);
    let server = FigstudioServer::new(vec![fixture("structure.fig")], true);
    tokio::spawn(async move { server.serve(server_io).await.unwrap().waiting().await });
    let client = ().serve(client_io).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    for live in ["list_open_tabs", "get_selection", "reveal_node"] {
        assert!(!tools.iter().any(|tool| tool.name == live), "{live}");
    }
    for tool in [
        "get_design_context",
        "get_screenshot",
        "get_variable_defs",
        "get_context_for_code_connect",
    ] {
        let result = call(&client, tool, json!({})).await;
        assert_eq!(result.is_error, Some(true), "{tool}");
        assert_eq!(texts(&result), "nodeId is required", "{tool}");
    }
    let assets = call(&client, "download_assets", json!({})).await;
    assert!(texts(&assets).contains("1 to 20"), "{}", texts(&assets));
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn reveal_reaches_the_app() {
    let app = start(two_tabs(), Answer::Reply).await;
    let done = app.call("reveal_node", json!({"nodeId": "1-4"})).await;
    assert_ne!(done.is_error, Some(true), "{}", texts(&done));
    let outer = app
        .call("reveal_node", json!({"nodeId": "I1:38;1:35"}))
        .await;
    assert!(
        texts(&outer).contains("outer instance"),
        "{}",
        texts(&outer)
    );
    let other = app
        .call(
            "reveal_node",
            json!({"nodeId": "1:5", "fileKey": "sections"}),
        )
        .await;
    assert_ne!(other.is_error, Some(true), "{}", texts(&other));
    let missing = app.call("reveal_node", json!({"nodeId": "999:1"})).await;
    assert_eq!(missing.is_error, Some(true));
    let bad = app.call("reveal_node", json!({"nodeId": "x"})).await;
    assert_eq!(texts(&bad), "invalid node ID");
    assert_eq!(
        *app.revealed.lock().unwrap(),
        vec![(7, Guid(1, 4)), (7, Guid(1, 38)), (9, Guid(1, 5))]
    );
    assert_eq!(app.wakes.load(std::sync::atomic::Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_figma_link_to_a_file_no_tab_shows_asks_the_app_to_open_it() {
    let app = start(two_tabs(), Answer::Reply).await;
    let figma = |name: &str, node: &str| json!({"nodeId": format!("https://www.figma.com/design/KEY/{name}?node-id={node}&t=x")});
    // An open tab matches by name: revealed there, nothing opened.
    let shown = app.call("reveal_node", figma("sections", "1-5")).await;
    assert_ne!(shown.is_error, Some(true), "{}", texts(&shown));
    // No tab matches: the app opens the local copy and its answer is the reply.
    let opened = app.call("reveal_node", figma("Openable", "1-4")).await;
    assert_ne!(opened.is_error, Some(true), "{}", texts(&opened));
    assert!(texts(&opened).starts_with("opened https://www.figma.com/design/KEY/Openable?"));
    // No local copy, but the active tab has the node: revealed there, as before.
    let fallback = app.call("reveal_node", figma("Missing", "1-4")).await;
    assert_ne!(fallback.is_error, Some(true), "{}", texts(&fallback));
    // No local copy and no such node: the open error explains it.
    let missing = app.call("reveal_node", figma("Missing", "999-1")).await;
    assert_eq!(missing.is_error, Some(true));
    assert_eq!(texts(&missing), "no local copy");
    // An explicit fileKey or a link without a file part never opens anything.
    let keyed = json!({"nodeId": "https://www.figma.com/design/KEY/Other?node-id=1-5", "fileKey": "sections"});
    let keyed = app.call("reveal_node", keyed).await;
    assert_ne!(keyed.is_error, Some(true), "{}", texts(&keyed));
    let bare = json!({"nodeId": "https://www.figma.com/?node-id=1-4"});
    assert_ne!(app.call("reveal_node", bare).await.is_error, Some(true));
    assert_eq!(app.opened.lock().unwrap().len(), 3);
    assert_eq!(
        *app.revealed.lock().unwrap(),
        vec![
            (9, Guid(1, 5)),
            (7, Guid(1, 4)),
            (9, Guid(1, 5)),
            (7, Guid(1, 4))
        ]
    );
}

#[tokio::test]
async fn reveal_times_out_when_the_app_is_silent() {
    let app = start(two_tabs(), Answer::Never).await;
    let started = std::time::Instant::now();
    let result = app.call("reveal_node", json!({"nodeId": "1:4"})).await;
    assert_eq!(result.is_error, Some(true));
    assert_eq!(texts(&result), "the app did not respond");
    assert!(started.elapsed() >= figstudio_mcp::live::REVEAL_TIMEOUT);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn snapshot_replaced_while_a_call_runs() {
    let held = doc("structure.fig");
    let mut caught = false;
    for _ in 0..20 {
        let snapshot = Snapshot {
            tabs: vec![tab(1, fixture("structure.fig"), held.clone(), None)],
            active: Some(1),
        };
        let app = Arc::new(start(snapshot, Answer::Reply).await);
        let caller = app.clone();
        let call = tokio::spawn(async move {
            caller
                .call("get_design_context", json!({"nodeId": "0:1"}))
                .await
        });
        // The call holds its own clone once it has opened the document (test + snapshot = 2).
        while Arc::strong_count(&held) < 3 && !call.is_finished() {
            tokio::task::yield_now().await;
        }
        app.replace(Snapshot::default());
        caught |= Arc::strong_count(&held) >= 3;
        let result = call.await.unwrap();
        assert_ne!(result.is_error, Some(true), "{}", texts(&result));
        let empty = data(&app.call("list_open_tabs", json!({})).await);
        assert_eq!(empty["tabs"], json!([]));
        let gone = app.call("get_design_context", json!({})).await;
        assert_eq!(texts(&gone), "no file is open in the app");
        if caught {
            break;
        }
    }
    // Catching the window depends on scheduling (one core may never catch it), so a miss is
    // inconclusive, not a failure; every round above still checks that no call failed.
    if !caught {
        eprintln!("inconclusive: never replaced the snapshot during a call");
    }
}

#[tokio::test]
async fn bind_first_skips_a_busy_port() {
    let busy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let taken = busy.local_addr().unwrap();
    let next = "127.0.0.1:0".parse().unwrap();
    let listener = bind_first([taken, next]).await.unwrap();
    assert_ne!(listener.local_addr().unwrap(), taken);
    assert!(bind_first([taken]).await.is_err());
    let none = bind_first([]).await.unwrap_err();
    assert_eq!(none.kind(), std::io::ErrorKind::InvalidInput);
    let ports: Vec<u16> = figstudio_mcp::default_http_addrs()
        .iter()
        .map(|a| a.port())
        .collect();
    assert_eq!(ports, (3845..=3855).collect::<Vec<_>>());
}

#[test]
fn live_render_caches_follow_the_snapshot() {
    use figstudio_mcp::{files::Files, live::LiveFiles};
    let structure = doc("structure.fig");
    let snapshot = Arc::new(std::sync::RwLock::new(Arc::new(Snapshot {
        tabs: vec![tab(1, fixture("structure.fig"), structure.clone(), None)],
        active: Some(1),
    })));
    let (commands, _inbox) = std::sync::mpsc::channel();
    let files = Files::Live(Arc::new(LiveFiles::new(
        snapshot.clone(),
        commands,
        Arc::new(|| {}),
    )));
    let first = files.rendering(None, &structure, false).unwrap();
    assert_eq!(
        first.path.as_deref(),
        Some(fixture("structure.fig").as_path())
    );
    assert!(!first.system_fonts);
    let again = files.rendering(None, &structure, true).unwrap();
    assert!(again.system_fonts);
    // Clones share one cache: an image stored through one is seen through the other.
    let mut png = Vec::new();
    image::RgbaImage::new(4, 4)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    assert!(first.images.insert_encoded("t", &png, 0));
    assert!(again.images.bytes() > 0, "same document, new cache");
    // A document that left the snapshot renders without lazy reads and gets a fresh cache later.
    *snapshot.write().unwrap() = Arc::new(Snapshot::default());
    assert_eq!(files.rendering(None, &structure, false).unwrap().path, None);
    *snapshot.write().unwrap() = Arc::new(Snapshot {
        tabs: vec![tab(1, fixture("structure.fig"), structure.clone(), None)],
        active: Some(1),
    });
    let back = files.rendering(None, &structure, false).unwrap();
    assert_eq!(back.images.bytes(), 0, "a closed document's cache was kept");
}
