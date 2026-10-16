//! Browser transport contracts using only the licensed public fixtures.
#[path = "wasm_rpc/support.rs"]
mod support;
use base64::{Engine, engine::general_purpose::STANDARD};
use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::{rendering::Rendering, wasm_rpc::handle_request};
use figstudio_render::images::Images;
use serde_json::{Value, json};
use std::{io::Cursor, sync::Arc};
use support::*;

#[test]
fn protocol_negotiates_and_returns_json_rpc_errors_without_notification_replies() {
    let env = env();
    let init = rpc(
        &env,
        "initialize",
        json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1"}}),
    );
    assert_eq!(init["id"], "request");
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "figstudio-web-mcp");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(init["result"]["capabilities"]["prompts"].is_object());
    assert_eq!(rpc(&env, "ping", json!({}))["result"], json!({}));
    for id in [json!(0), json!(""), Value::Null] {
        let reply: Value = serde_json::from_str(
            &handle_request(
                &json!({"jsonrpc":"2.0","id":id,"method":"ping"}).to_string(),
                &env,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(reply["id"], id);
        assert_eq!(reply["result"], json!({}));
    }
    for method in [
        "notifications/initialized",
        "notifications/cancelled",
        "ping",
    ] {
        assert!(
            handle_request(&json!({"jsonrpc":"2.0","method":method}).to_string(), &env).is_none()
        );
    }
    let parse: Value = serde_json::from_str(&handle_request("{", &env).unwrap()).unwrap();
    assert_eq!(parse["error"]["code"], -32700);
    assert!(parse["id"].is_null());
    for (method, params, code) in [
        ("missing", json!({}), -32601),
        // A notifications/* method with an id is a request the server does not implement.
        ("notifications/initialized", json!({}), -32601),
        ("notifications/bogus", json!({}), -32601),
        ("tools/call", json!({}), -32602),
        ("tools/call", json!({"name":"unknown"}), -32602),
        ("ping", json!([]), -32602),
        (
            "tools/call",
            json!({"name":"get_metadata","arguments":{"depth":"wrong"}}),
            -32602,
        ),
        (
            "tools/call",
            json!({"name":"reveal_node","arguments":{}}),
            -32602,
        ),
        (
            "tools/call",
            json!({"name":"search_design_system","arguments":{}}),
            -32602,
        ),
        ("prompts/get", json!({"name":"unknown"}), -32602),
    ] {
        assert_eq!(rpc(&env, method, params)["error"]["code"], code);
    }
    for invalid in [
        json!({}),
        json!({"jsonrpc":"1.0","id":1,"method":"ping"}),
        json!({"jsonrpc":"2.0","id":true,"method":"ping"}),
    ] {
        let reply: Value =
            serde_json::from_str(&handle_request(&invalid.to_string(), &env).unwrap()).unwrap();
        assert_eq!(reply["error"]["code"], -32600);
    }
}

#[test]
fn tools_dispatch_core_outputs_and_inline_png_assets() {
    let env = env();
    let listed = rpc(&env, "tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    let meta = call(&env, "get_metadata", json!({}));
    assert!(text(&meta).contains("<pages"));
    let node = call(&env, "get_metadata", json!({"nodeId":"1:34","depth":0}));
    assert!(text(&node).contains("1:34"));
    for name in ["get_screenshot", "get_design_context", "download_assets"] {
        assert!(tools.iter().any(|t| t["name"] == name));
        let result = call(&env, name, json!({}));
        assert_eq!(result["isError"], false, "{result}");
        png(&result);
        if name == "get_design_context" {
            assert!(text(&result).contains("<img"));
        }
    }
    for (name, args, marker) in [
        ("get_variable_defs", json!({}), "No resolved tokens"),
        ("get_figjam", json!({}), "<"),
        ("get_libraries", json!({}), "components"),
        ("search_design_system", json!({"query":""}), "results"),
        (
            "get_context_for_code_connect",
            json!({"nodeId":"1:37"}),
            "component",
        ),
        (
            "get_nodes_in_region",
            json!({"page":env.tabs[0].page.to_string(),"rect":[0,0,10000,10000]}),
            "nodes",
        ),
    ] {
        let result = call(&env, name, args);
        assert_eq!(result["isError"], false, "{name}: {result}");
        assert!(
            text(&result)
                .to_lowercase()
                .contains(&marker.to_lowercase()),
            "{name}: {result}"
        );
    }
    for args in [
        json!({"nodeId":"bad"}),
        json!({"scale":9}),
        json!({"fileKey":"unknown"}),
    ] {
        assert_eq!(call(&env, "get_screenshot", args)["isError"], true);
    }
}

#[test]
fn live_tab_routing_selection_and_reveal_use_the_same_snapshot() {
    let mut env = env();
    let mut second = env.tabs[0].clone();
    second.id = 8;
    second.file_key = "second".into();
    second.selection = None;
    env.tabs.push(second);
    let tabs = call(&env, "list_open_tabs", json!({}));
    assert_eq!(tabs["structuredContent"]["tabs"][0]["active"], true);
    assert_eq!(tabs["structuredContent"]["tabs"][1]["active"], false);
    assert_eq!(
        tabs["structuredContent"]["tabs"][0]["viewport"],
        json!([0., 0., 800., 600.])
    );
    let selected = call(&env, "get_selection", json!({}));
    assert_eq!(selected["structuredContent"]["node"]["nodeId"], "1:34");
    assert!(selected["structuredContent"]["node"]["bounds"].is_object());
    assert!(
        call(&env, "get_selection", json!({"fileKey":"second"}))["structuredContent"]["node"]
            .is_null()
    );
    assert_eq!(
        call(&env, "get_screenshot", json!({"fileKey":"second"}))["isError"],
        true
    );
    let revealed = call(
        &env,
        "reveal_node",
        json!({"nodeId":"figstudio://second?node-id=1-34"}),
    );
    assert_eq!(revealed["isError"], false);
    assert_eq!(*env.revealed.lock().unwrap(), vec![(8, Guid(1, 34))]);
    assert_eq!(
        call(&env, "reveal_node", json!({"nodeId":"9:999"}))["isError"],
        true
    );
    assert_eq!(env.revealed.lock().unwrap().len(), 1);
    env.tabs.clear();
    assert_eq!(call(&env, "get_selection", json!({}))["isError"], true);
    assert_eq!(
        call(&env, "list_open_tabs", json!({}))["structuredContent"]["tabs"],
        json!([])
    );
}

#[test]
fn prompts_supply_shared_guidance_and_inventory_with_data_preferences() {
    let env = env();
    let list = rpc(&env, "prompts/list", json!({}));
    assert_eq!(
        list["result"]["prompts"][0]["name"],
        "create_design_system_rules"
    );
    let reply = rpc(
        &env,
        "prompts/get",
        json!({"name":"create_design_system_rules","arguments":{"clientFrameworks":"React","clientLanguages":"typescript"}}),
    );
    let message = &reply["result"]["messages"][0];
    assert_eq!(message["role"], "user");
    let text = message["content"]["text"].as_str().unwrap();
    for required in [
        "rules/design-system.md",
        "Reuse existing components",
        "get_screenshot",
        "Local design-system inventory",
        "React",
        "typescript",
    ] {
        assert!(text.contains(required));
    }
    assert_eq!(
        rpc(
            &env,
            "prompts/get",
            json!({"name":"create_design_system_rules","arguments":{"fileKey":2}})
        )["error"]["code"],
        -32602
    );
}

#[test]
fn byte_backed_reader_supplies_lazy_originals_and_never_falls_through() {
    let bytes = Arc::new(std::fs::read(fixture()).unwrap());
    let lazy = Document::open_lazy(fixture()).unwrap();
    let entry = lazy.lazy_images.values().next().unwrap();
    let expected =
        figstudio_fig::archive::read_entry(&mut Cursor::new(bytes.as_slice()), entry).unwrap();
    let source = bytes.clone();
    let mut rendering = Rendering::with_reader(
        Arc::new(move |e| {
            figstudio_fig::archive::read_entry(&mut Cursor::new(source.as_slice()), e).ok()
        }),
        Images::default(),
    );
    assert_eq!(rendering.read(entry), Some(expected));
    assert!(format!("{rendering:?}").contains("EntryReader"));
    let mut env = env();
    env.tabs[0].doc = Arc::new(lazy);
    env.tabs[0].rendering = rendering.clone();
    let out = call(&env, "download_assets", json!({"original":true}));
    assert_eq!(out["isError"], false);
    let original = out["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["type"] == "image")
        .unwrap();
    let bytes = STANDARD.decode(original["data"].as_str().unwrap()).unwrap();
    assert_eq!(original["mimeType"], "image/jpeg");
    assert_eq!(
        image::guess_format(&bytes).unwrap(),
        image::ImageFormat::Jpeg
    );
    rendering.reader = Some(Arc::new(|_| None));
    rendering.path = Some(fixture());
    assert!(rendering.read(entry_for(&env)).is_none());
}
fn entry_for(env: &Env) -> &figstudio_fig::archive::Entry {
    env.tabs[0].doc.lazy_images.values().next().unwrap()
}

#[cfg(not(feature = "fs"))]
#[test]
fn default_design_assets_are_inline_without_filesystem_support() {
    let env = env();
    let output =
        figstudio_mcp_core::design::design_context(&env.tabs[0].doc, "1:34", &Default::default());
    assert!(!output.is_error);
    assert!(output.content.iter().any(
        |b| matches!(b,figstudio_mcp_core::output::Block::Image{mime,..} if mime=="image/png")
    ));
}
