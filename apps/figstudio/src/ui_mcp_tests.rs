//! Settings → Agents: entering an address, starting and stopping the hosted server.
use super::*;
use egui_kittest::kittest::NodeT as _;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;
use std::net::{SocketAddr, TcpStream};

fn wait_for<T>(
    h: &mut Harness<'_, App>,
    what: &str,
    mut f: impl FnMut(&Harness<'_, App>) -> Option<T>,
) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(v) = f(h) {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        h.step();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn the_agent_window_types_an_address_and_starts_and_stops_the_server() {
    let mut h = harness();
    assert!(!h.state().mcp.on, "off at launch");
    // No native menu here: File, View and Help sit behind the sidebar's "⋯" button.
    h.get_by_label("Menu").click();
    h.run_steps(3);
    h.get_by_label_contains("View").click();
    h.run_steps(3);
    h.get_by_label("Agent access…").click();
    h.run_steps(3);
    h.get_by_label("Allow remote agents").click();
    h.run_steps(3);
    // Found by its hint, not position: the layers panel has its own search field.
    let field = h.get_by(|n| {
        n.role() == egui::accesskit::Role::TextInput && n.placeholder() == Some("100.x.y.z:3845")
    });
    // An ephemeral loopback listener exercises the remote plan without the live agent port.
    field.click();
    h.run_steps(3);
    h.event(egui::Event::Text("127.0.0.1:0".into()));
    h.run_steps(3);
    assert_eq!(h.state().mcp.remote_addr, "127.0.0.1:0");
    let switch = |h: &Harness<'_, App>| {
        h.query_by_role_and_label(egui::accesskit::Role::CheckBox, "Agent access")
            .map(|n| n.rect())
    };
    assert!(switch(&h).is_some(), "editing stays in the window");
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Agent access")
        .click();
    h.run_steps(3);
    let addr: SocketAddr = wait_for(&mut h, "server address", |h| {
        h.query_all_by_label_contains("Connect to. 127.0.0.1")
            .next()?
            .value()?
            .strip_prefix("Connect to. ")?
            .strip_suffix("/mcp")?
            .parse()
            .ok()
    });
    assert_ne!(addr.port(), 3845);
    assert!(TcpStream::connect(addr).is_ok());
    // While it runs the remote switch cannot change, and says so (egui marks a widget in a
    // disabled scope disabled); its note names the token.
    let remote = h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Allow remote agents");
    let disabled = h.query_by(|n| {
        n.role() == egui::accesskit::Role::CheckBox
            && n.is_disabled()
            && n.bounding_box() == remote.accesskit_node().bounding_box()
    });
    assert!(
        disabled.is_some(),
        "reported disabled while the server runs"
    );
    assert!(
        h.query_by_label_contains("send the access token").is_some(),
        "the note says clients need the token"
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = runtime.block_on(async {
        let transport = StreamableHttpClientTransport::from_uri(format!("http://{addr}/mcp"));
        ().serve(transport).await.unwrap()
    });
    let listed = runtime
        .block_on(client.call_tool(CallToolRequestParams::new("list_open_tabs")))
        .unwrap();
    assert_eq!(
        listed.structured_content.unwrap()["tabs"][0]["fileKey"],
        "structure"
    );
    runtime.block_on(client.cancel()).unwrap();
    h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Agent access")
        .click();
    h.run_steps(3);
    assert!(!h.state().mcp.on);
    wait_for(&mut h, "port close", |_| {
        TcpStream::connect(addr).is_err().then_some(())
    });
}

#[test]
fn help_opens_the_about_window() {
    let mut h = harness();
    h.get_by_label("Menu").click();
    h.run_steps(3);
    h.get_by_label_contains("Help").click();
    h.run_steps(3);
    h.get_by_label("About figstudio").click();
    h.run_steps(3);
    assert!(
        h.query_by_label("Opens Figma .fig local copies offline.")
            .is_some()
    );
}

#[test]
fn reveal_node_opens_a_local_copy_and_answers_after_the_new_tab_is_published() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(fixture("sections.fig"), dir.path().join("My Design.fig")).unwrap();
    let mut h = harness();
    {
        let app = h.state_mut();
        app.link.dirs = vec![dir.path().to_owned()];
        // A loopback "remote" address picks a free port, away from a running app on :3845.
        app.mcp.remote = true;
        app.mcp.remote_addr = "127.0.0.1:0".to_owned();
        app.mcp.on = true;
    }
    let addr: SocketAddr = wait_for(&mut h, "server address", |h| {
        h.query_all_by_label_contains("MCP 127.0.0.1")
            .next()?
            .value()?
            .strip_prefix("MCP ")?
            .strip_suffix("/mcp")?
            .parse()
            .ok()
    });
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let client = runtime.block_on(async {
        let transport = StreamableHttpClientTransport::from_uri(format!("http://{addr}/mcp"));
        ().serve(transport).await.unwrap()
    });
    let link = "https://www.figma.com/design/KEY/My-Design?node-id=1-5";
    let reveal = runtime.spawn(async move {
        let args = serde_json::json!({ "nodeId": link });
        let request = CallToolRequestParams::new("reveal_node")
            .with_arguments(args.as_object().unwrap().clone());
        let result = client.call_tool(request).await.unwrap();
        (result, client)
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !reveal.is_finished() {
        assert!(Instant::now() < deadline, "reveal_node did not finish");
        h.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    let (result, client) = runtime.block_on(reveal).unwrap();
    let text = serde_json::to_value(&result).unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(result.is_error, Some(true), "{text}");
    let id = h.state().tabs[1].id;
    assert_eq!(
        text,
        format!("revealed 1:5 in tab {id} (opened the local copy)")
    );
    // No frame has run since the reply: the snapshot already had the new tab.
    let listed = runtime
        .block_on(client.call_tool(CallToolRequestParams::new("list_open_tabs")))
        .unwrap()
        .structured_content
        .unwrap();
    let tabs = listed["tabs"].as_array().unwrap();
    assert_eq!(tabs.len(), 2, "{listed}");
    assert_eq!(tabs[1]["fileKey"], "My Design");
    assert_eq!(tabs[1]["selection"], "1:5");
    assert_eq!(tabs[1]["active"], true);
    runtime.block_on(client.cancel()).unwrap();
    h.state_mut().mcp.on = false;
    h.run_steps(3);
}
