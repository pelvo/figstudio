//! View menu → Agent access: the toggle starts and stops the hosted server.
use std::net::{SocketAddr, TcpStream};

use egui_kittest::kittest::NodeT;
use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::transport::StreamableHttpClientTransport;

use super::*;

const ITEM: &str = "Agent access (MCP server)";
const STATUS: &str = "MCP 127.0.0.1";

/// Whether the menu item is checked (a click closes the menu, so this opens it again).
fn checked(h: &mut Harness<'_, App>) -> bool {
    open_view(h);
    h.get_by_label(ITEM).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
}

/// Opens the View menu (it takes a few frames to appear) unless it is open already.
fn open_view(h: &mut Harness<'_, App>) {
    if h.query_by_label(ITEM).is_none() {
        h.get_by_label("View").click();
        wait_for(h, "the View menu", |h| h.query_by_label(ITEM).map(|_| ()));
    }
}

/// Opens View and clicks the checkbox.
fn toggle(h: &mut Harness<'_, App>) {
    open_view(h);
    h.get_by_label(ITEM).click();
    h.run_steps(3);
}

/// The address the status bar shows, once the server has bound its port.
fn shown_addr(h: &Harness<'_, App>) -> Option<SocketAddr> {
    let node = h.query_by_label_contains(STATUS)?;
    // A plain egui label is exposed as the node's value.
    let label = node.value()?;
    label
        .strip_prefix("MCP ")?
        .strip_suffix("/mcp")?
        .parse()
        .ok()
}

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
fn the_view_menu_starts_and_stops_the_mcp_server() {
    let mut h = harness();
    assert!(h.query_by_label_contains(STATUS).is_none(), "off at launch");
    open_view(&mut h);
    assert!(!checked(&mut h), "unchecked at launch");

    toggle(&mut h);
    assert!(checked(&mut h));
    let addr = wait_for(&mut h, "the MCP address in the status bar", shown_addr);
    assert!(addr.ip().is_loopback());
    assert!(TcpStream::connect(addr).is_ok(), "the port answers");

    // The hosted server sees the tab the UI opened.
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

    toggle(&mut h);
    assert!(!checked(&mut h));
    wait_for(&mut h, "the status label to go", |h| {
        h.query_by_label_contains(STATUS).is_none().then_some(())
    });
    wait_for(&mut h, "the port to close", |_| {
        TcpStream::connect(addr).is_err().then_some(())
    });
}

#[test]
fn clicking_the_remote_address_field_keeps_the_view_menu_open() {
    let mut h = harness();
    open_view(&mut h);
    h.get_by_label("Allow remote agents").click();
    h.run_steps(3);
    open_view(&mut h);
    let field = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .last()
        .expect("the address field, the last text input")
        .rect();
    h.hover_at(field.center());
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: field.center(),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
    }
    h.run_steps(3);
    assert!(
        h.query_by_label(ITEM).is_some(),
        "the menu closed under the address field"
    );
}
