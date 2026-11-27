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
        let png = pixels(&result, "image/png", image::ImageFormat::Png);
        for invalid in ["jpg", "jxl", "", "WEBP"] {
            if invalid == "jpg" && name == "download_assets" {
                let jpg = call(&env, name, json!({"format":"jpg"}));
                assert_eq!(jpg["isError"], false, "{jpg}");
                pixels(&jpg, "image/jpeg", image::ImageFormat::Jpeg);
                continue;
            }
            let invalid = call(&env, name, json!({"format":invalid}));
            assert_eq!(invalid["isError"], true, "{name}: {invalid}");
        }
        let webp = call(&env, name, json!({"format":"webp"}));
        assert_eq!(webp["isError"], false, "{webp}");
        assert_eq!(pixels(&webp, "image/webp", image::ImageFormat::WebP), png);
        if name == "get_design_context" {
            assert!(text(&result).contains("<img"));
        }
    }
    // Tools without images ignore `format`, valid or not.
    for name in ["get_metadata", "get_variable_defs", "get_libraries"] {
        let result = call(&env, name, json!({"format":"jpg"}));
        assert_eq!(result["isError"], false, "{name}: {result}");
    }
    for (name, args, marker) in [
        ("get_variable_defs", json!({}), "No resolved tokens"),
        ("get_figjam", json!({}), "<"),
        ("get_libraries", json!({}), "components"),
        ("get_prototype", json!({}), "screens"),
        ("search_design_system", json!({"query":""}), "results"),
        ("list_file_shaders", json!({}), "shaders"),
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

#[test]
fn copy_for_figma_reads_lazy_png_through_reader_and_propagates_failures() {
    use figstudio_fig::{
        Obj, Val, archive,
        clipboard::decode_clipboard_html,
        edit::{EditOp, History, execute_edit_ops},
        sha1::sha1_hex,
    };
    use std::collections::HashMap;
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};

    let make_png = |rgba: [u8; 4]| -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(2, 2, image::Rgba(rgba));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    };
    let png_a = make_png([220, 40, 60, 255]);
    let png_b = make_png([30, 140, 210, 255]);
    let hash_a = sha1_hex(&png_a);
    let hash_b = sha1_hex(&png_b);
    let undeclared_hash = "aa".repeat(20);

    // Build a synthetic .fig archive with two PNG images and three nodes:
    // - rect_a -> hash_a (declared in images/)
    // - rect_b -> hash_b (declared in images/)
    // - rect_undeclared -> undeclared_hash (in neither images nor lazy_images)
    let mut base = Document::open(fixture()).unwrap();
    let page = base.pages[0];
    let mut history = History::new(8);
    let create_rect = |doc: &mut Document, hist: &mut History, x: f64| {
        execute_edit_ops(
            doc,
            hist,
            &[EditOp::CreateShape {
                id: None,
                kind: "RECTANGLE".into(),
                parent: page,
                name: None,
                x,
                y: 0.0,
                width: 24.0,
                height: 24.0,
                corner_radius: 0.0,
                fill_hex: None,
                count: None,
                star_inner_scale: None,
            }],
            None,
            None,
            false,
        )
        .unwrap()
        .0
        .created_ids[0]
    };
    let rect_a = create_rect(&mut base, &mut history, 0.0);
    let rect_b = create_rect(&mut base, &mut history, 40.0);
    let rect_undeclared = create_rect(&mut base, &mut history, 80.0);

    base.images.insert(hash_a.clone(), png_a.clone());
    base.images.insert(hash_b.clone(), png_b.clone());
    for (target, hash_hex) in [(rect_a, &hash_a), (rect_b, &hash_b)] {
        execute_edit_ops(
            &mut base,
            &mut history,
            &[EditOp::SetImageFill {
                target,
                sha1_hex: hash_hex.clone(),
                scale_mode: "FILL".into(),
                opacity: 1.0,
            }],
            None,
            None,
            false,
        )
        .unwrap();
    }
    let mut undeclared_txn = figstudio_fig::edit::Txn::new("undeclared hash");
    let mut img_obj = Obj::new();
    img_obj.insert("hash", Val::Bytes(vec![0xaa; 20]));
    let mut paint = Obj::new();
    paint.insert("type", Val::Enum(figstudio_fig::intern("IMAGE")));
    paint.insert("opacity", Val::Float(1.0));
    paint.insert("visible", Val::Bool(true));
    paint.insert("blendMode", Val::Enum(figstudio_fig::intern("NORMAL")));
    paint.insert("image", Val::Obj(img_obj));
    paint.insert("imageScaleMode", Val::Enum(figstudio_fig::intern("FILL")));
    undeclared_txn.set_field(
        rect_undeclared,
        "fillPaints",
        Val::Arr(vec![Val::Obj(paint)]),
    );
    base.apply_txn(&undeclared_txn).unwrap();

    // Package canvas + PNG entries into an in-memory ZIP and open via `Document::from_lazy`.
    let canvas = base.encode_canvas().unwrap();
    let mut zip_buf = Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut zip_buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip.start_file("canvas.fig", opts).unwrap();
        zip.write_all(&canvas).unwrap();
        zip.start_file(format!("images/{hash_a}"), opts).unwrap();
        zip.write_all(&png_a).unwrap();
        zip.start_file(format!("images/{hash_b}"), opts).unwrap();
        zip.write_all(&png_b).unwrap();
        zip.finish().unwrap();
    }
    let zip_bytes = Arc::new(zip_buf.into_inner());
    let entries = archive::read_entries(
        &mut Cursor::new(zip_bytes.as_slice()),
        zip_bytes.len() as u64,
    )
    .unwrap();
    let mut eager = HashMap::new();
    for e in entries
        .iter()
        .filter(|e| archive::EAGER_ENTRIES.contains(&e.name.as_str()))
    {
        eager.insert(
            e.name.clone(),
            archive::read_entry(&mut Cursor::new(zip_bytes.as_slice()), e).unwrap(),
        );
    }
    let lazy = Arc::new(Document::from_lazy(entries, eager).unwrap());
    assert!(lazy.images.is_empty());
    assert!(lazy.lazy_images.contains_key(&hash_a));
    assert!(lazy.lazy_images.contains_key(&hash_b));
    assert!(!lazy.lazy_images.contains_key(&undeclared_hash));

    // 1. Existing byte-backed reader succeeds -> exact PNG bytes survive HTML round trip.
    let reader_bytes = zip_bytes.clone();
    let fail_b = Arc::new(AtomicBool::new(false));
    let fail_b_flag = fail_b.clone();
    let hash_b_match = format!("images/{hash_b}");
    let rendering = Rendering::with_reader(
        Arc::new(move |entry| {
            if fail_b_flag.load(Ordering::Relaxed) && entry.name == hash_b_match {
                return None;
            }
            archive::read_entry(&mut Cursor::new(reader_bytes.as_slice()), entry).ok()
        }),
        Images::default(),
    );
    let mut test_env = env();
    test_env.tabs[0].doc = lazy.clone();
    test_env.tabs[0].rendering = rendering;

    let res = call(
        &test_env,
        "copy_for_figma",
        json!({ "nodeIds": [rect_a.to_string(), rect_b.to_string()] }),
    );
    assert_eq!(res["isError"], false, "{res}");
    let html = res["structuredContent"]["html"].as_str().unwrap();
    let decoded = decode_clipboard_html(html).unwrap();
    let map: HashMap<String, Vec<u8>> = decoded.images.into_iter().collect();
    assert_eq!(map.get(&hash_a), Some(&png_a));
    assert_eq!(map.get(&hash_b), Some(&png_b));
    let decoded_a = image::load_from_memory(&map[&hash_a]).unwrap().to_rgba8();
    assert_eq!(decoded_a.dimensions(), (2, 2));
    assert_eq!(decoded_a.get_pixel(0, 0).0, [220, 40, 60, 255]);

    // 2. Partial/failed lazy read -> error propagated, never silently dropped.
    fail_b.store(true, Ordering::Relaxed);
    let partial_fail = call(
        &test_env,
        "copy_for_figma",
        json!({ "nodeIds": [rect_a.to_string(), rect_b.to_string()] }),
    );
    assert_eq!(
        partial_fail["isError"], true,
        "failing lazy read for rect_b must fail export: {partial_fail}"
    );

    // 3. Missing reader completely (`Rendering::default()`) on known lazy asset -> error propagated.
    test_env.tabs[0].rendering = Rendering::default();
    let no_reader = call(
        &test_env,
        "copy_for_figma",
        json!({ "nodeIds": [rect_a.to_string()] }),
    );
    assert_eq!(
        no_reader["isError"], true,
        "missing reader for known lazy asset must fail export: {no_reader}"
    );

    // 4. Undeclared/missing image hash policy -> succeeds without fabricating bytes.
    let undeclared_res = call(
        &test_env,
        "copy_for_figma",
        json!({ "nodeIds": [rect_undeclared.to_string()] }),
    );
    assert_eq!(undeclared_res["isError"], false, "{undeclared_res}");
    let undeclared_html = undeclared_res["structuredContent"]["html"]
        .as_str()
        .unwrap();
    let undeclared_decoded = decode_clipboard_html(undeclared_html).unwrap();
    assert!(undeclared_decoded.images.is_empty());
}
