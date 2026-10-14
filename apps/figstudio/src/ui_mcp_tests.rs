//! Agent access window: entering an address, starting and stopping the hosted server.
use super::*;
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
    h.get_by_label("View").click();
    h.run_steps(3);
    h.get_by_label("Agent access…").click();
    h.run_steps(3);
    h.get_by_label("Allow remote agents").click();
    h.run_steps(3);
    let field = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .last()
        .unwrap();
    // An ephemeral loopback listener exercises the remote plan without the live agent port.
    field.click();
    h.run_steps(3);
    h.event(egui::Event::Text("127.0.0.1:0".into()));
    h.run_steps(3);
    assert_eq!(h.state().mcp.remote_addr, "127.0.0.1:0");
    assert!(
        h.query_by_label("Agent access (MCP server)").is_some(),
        "editing stays in the window"
    );
    h.get_by_label("Agent access (MCP server)").click();
    h.run_steps(3);
    let addr: SocketAddr = wait_for(&mut h, "server address", |h| {
        h.query_all_by_label_contains("MCP 127.0.0.1")
            .next()?
            .value()?
            .strip_prefix("MCP ")?
            .strip_suffix("/mcp")?
            .parse()
            .ok()
    });
    assert_ne!(addr.port(), 3845);
    assert!(TcpStream::connect(addr).is_ok());
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
    h.get_by_label("Agent access (MCP server)").click();
    h.run_steps(3);
    assert!(!h.state().mcp.on);
    wait_for(&mut h, "port close", |_| {
        TcpStream::connect(addr).is_err().then_some(())
    });
}

#[test]
fn help_opens_the_about_window() {
    let mut h = harness();
    h.get_by_label("Help").click();
    h.run_steps(3);
    h.get_by_label("About figstudio").click();
    h.run_steps(3);
    assert!(
        h.query_by_label("Opens Figma .fig local copies offline.")
            .is_some()
    );
}
