//! The hosted server: snapshot key, reveal answers, port fallback, and a real HTTP client.

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use figstudio_fig::Document;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};

use super::*;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn tab(id: u64, name: &str) -> Tab {
    let doc = Arc::new(Document::open(fixture(name)).unwrap());
    Tab::new(id, fixture(name), doc, None, Duration::ZERO)
}

fn host(addrs: Vec<SocketAddr>) -> McpHost {
    McpHost {
        on: true,
        addrs,
        running: None,
    }
}

const ANY_PORT: &str = "127.0.0.1:0";

/// Run frames until the server reports its address (or an error), at most 10 s.
fn wait_started(host: &mut McpHost, ctx: &egui::Context, tabs: &mut [Tab]) -> SocketAddr {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut active = 0;
    loop {
        host.sync(ctx, tabs, active);
        assert_eq!(host.frame(tabs, &mut active), None);
        if let Some(addr) = host.addr() {
            return addr;
        }
        assert!(Instant::now() < deadline, "the server did not start");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_key_follows_what_the_tools_see() {
    let mut tabs = vec![tab(1, "structure.fig"), tab(2, "sections.fig")];
    let before = key(&tabs, 0);
    tabs[0].hover = Some(Guid(1, 25));
    assert_eq!(key(&tabs, 0), before, "hover is not published");
    tabs[0].select(Some(Guid(1, 25)));
    let selected = key(&tabs, 0);
    assert_ne!(selected, before, "selection");
    // Any other page id: the key compares ids only.
    tabs[1].set_page(Guid(0, 99));
    let paged = key(&tabs, 0);
    assert_ne!(paged, selected, "page");
    tabs[0].canvas_rect = Some(egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(800.0, 600.0),
    ));
    let viewed = key(&tabs, 0);
    assert_ne!(viewed, paged, "viewport");
    tabs[0].view.pan.x += 0.2;
    assert_eq!(key(&tabs, 0), viewed, "sub-unit pan is not published");
    assert_ne!(key(&tabs, 1), viewed, "active tab");
    tabs.remove(1);
    assert_ne!(key(&tabs, 0), viewed, "tab close");
}

#[test]
fn reveal_selects_and_fits_or_answers_tab_closed() {
    let mut host = host(vec![]);
    let (commands, inbox) = mpsc::channel();
    let (_events, news) = mpsc::channel();
    let mut running = Running {
        addr: None,
        cancel: CancellationToken::new(),
        snapshot: Default::default(),
        commands: inbox,
        events: news,
        last_key: None,
    };
    let mut tabs = vec![tab(1, "structure.fig"), tab(2, "structure.fig")];
    let mut active = 0;
    let (reply, mut ok) = tokio::sync::oneshot::channel();
    let node = Guid(1, 42);
    commands
        .send(Command::Reveal {
            tab: 2,
            node,
            reply,
        })
        .unwrap();
    let (reply, mut closed) = tokio::sync::oneshot::channel();
    commands
        .send(Command::Reveal {
            tab: 9,
            node,
            reply,
        })
        .unwrap();
    running.drain(&mut tabs, &mut active);
    assert_eq!(ok.try_recv().unwrap(), Ok(()));
    assert_eq!(closed.try_recv().unwrap(), Err("tab closed".to_owned()));
    assert_eq!(active, 1);
    assert_eq!(tabs[1].selection, Some(node));
    assert_eq!(tabs[1].fit_node, Some(node));
    assert_eq!(tabs[0].selection, None);
    host.running = Some(running);
    host.stop();
}

#[test]
fn a_busy_port_falls_back_to_the_next() {
    let busy = TcpListener::bind(ANY_PORT).unwrap();
    let busy_addr = busy.local_addr().unwrap();
    let mut host = host(vec![busy_addr, ANY_PORT.parse().unwrap()]);
    let ctx = egui::Context::default();
    let addr = wait_started(&mut host, &ctx, &mut []);
    assert_ne!(addr, busy_addr);
    assert!(addr.ip().is_loopback());
    host.stop();
}

#[test]
fn no_free_port_turns_the_toggle_off_with_an_error() {
    let busy = TcpListener::bind(ANY_PORT).unwrap();
    let mut host = host(vec![busy.local_addr().unwrap()]);
    let ctx = egui::Context::default();
    let deadline = Instant::now() + Duration::from_secs(10);
    let error = loop {
        host.sync(&ctx, &[], 0);
        if let Some(error) = host.frame(&mut [], &mut 0) {
            break error;
        }
        assert!(Instant::now() < deadline, "no error reported");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(error.contains("no free port"), "{error}");
    assert!(!host.on);
    assert!(host.running.is_none());
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    tool: &'static str,
    args: Value,
) -> Value {
    let request =
        CallToolRequestParams::new(tool).with_arguments(args.as_object().unwrap().clone());
    let result = client.call_tool(request).await.unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    result.structured_content.unwrap_or(Value::Null)
}

#[test]
fn an_http_client_lists_tabs_and_reveals_a_node() {
    let ctx = egui::Context::default();
    let mut tabs = vec![tab(3, "structure.fig"), tab(4, "sections.fig")];
    tabs[0].select(Some(Guid(1, 25)));
    let mut host = host(vec![ANY_PORT.parse().unwrap()]);
    let addr = wait_started(&mut host, &ctx, &mut tabs);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = runtime.block_on(async {
        let transport = StreamableHttpClientTransport::from_uri(format!("http://{addr}/mcp"));
        ().serve(transport).await.unwrap()
    });
    let listed = runtime.block_on(call(&client, "list_open_tabs", json!({})));
    assert_eq!(listed["tabs"][0]["tabId"], 3);
    assert_eq!(listed["tabs"][0]["selection"], "1:25");
    assert_eq!(listed["tabs"][1]["fileKey"], "sections");

    // The app answers reveal_node from its frame loop; run it while the call waits.
    let reveal = runtime.spawn(async move {
        let request = CallToolRequestParams::new("reveal_node").with_arguments(
            json!({"nodeId": "1:42", "fileKey": "structure"})
                .as_object()
                .unwrap()
                .clone(),
        );
        let result = client.call_tool(request).await.unwrap();
        (result, client)
    });
    let mut active = 1;
    let deadline = Instant::now() + Duration::from_secs(10);
    while !reveal.is_finished() {
        assert_eq!(host.frame(&mut tabs, &mut active), None);
        host.sync(&ctx, &tabs, active);
        assert!(Instant::now() < deadline, "reveal_node did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    let (result, client) = runtime.block_on(reveal).unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(active, 0);
    assert_eq!(tabs[0].selection, Some(Guid(1, 42)));
    let selected = runtime.block_on(call(&client, "get_selection", json!({})));
    assert_eq!(selected["node"]["nodeId"], "1:42");
    runtime.block_on(client.cancel()).unwrap();

    // Toggle off: the socket closes.
    host.on = false;
    host.sync(&ctx, &tabs, active);
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(addr).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the port is still open after stop"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
