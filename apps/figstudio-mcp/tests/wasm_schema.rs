//! Browser schemas must not drift from the native SDK's independent contract.
use figstudio_mcp::{
    FigstudioServer,
    live::{LiveFiles, Snapshot},
};
use figstudio_mcp_core::wasm_rpc::tool_list;
use rmcp::{ServiceExt, model::GetPromptRequestParams};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    sync::{Arc, RwLock, mpsc},
};

fn required(schema: &Value) -> BTreeSet<&str> {
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|v| v.as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn browser_tools_match_native_names_properties_and_required_arguments() {
    let file = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(figstudio_fig::Document::open(&file).unwrap());
    let snapshot = Arc::new(RwLock::new(Arc::new(Snapshot::default())));
    let (tx, _rx) = mpsc::channel();
    let live = LiveFiles::new(snapshot, tx, Arc::new(|| {}));
    let (a, b) = tokio::io::duplex(65536);
    let server = tokio::spawn(FigstudioServer::live(Arc::new(live), true).serve(a));
    let client = ().serve(b).await.unwrap();
    let server = server.await.unwrap().unwrap();
    let native = client.list_all_tools().await.unwrap();
    let browser = tool_list();
    // Sidecar/cloud operations have no source binding store in the browser. whoami is a
    // native installation diagnostic, not document inspection.
    // compare_files/compare_page are also native only: figstudio-diff renders through the
    // native renderer, which the wasm32 browser build must not link. Exact native
    // compare schemas are asserted in tests/stdio.rs and tests/http.rs.
    let omitted = [
        "get_code_connect_map",
        "add_code_connect_map",
        "whoami",
        "compare_files",
        "compare_page",
    ];
    for tool in &browser {
        let name = tool["name"].as_str().unwrap();
        let other = native
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{name} missing natively"));
        let schema = Value::Object((*other.input_schema).clone());
        assert_eq!(
            required(&tool["inputSchema"]),
            required(&schema),
            "{name} required arguments"
        );
        let properties = |v: &Value| {
            v["properties"]
                .as_object()
                .map(|p| p.keys().cloned().collect::<BTreeSet<_>>())
                .unwrap_or_default()
        };
        assert_eq!(
            properties(&tool["inputSchema"]),
            properties(&schema),
            "{name} property names"
        );
    }
    for tool in &native {
        assert!(
            omitted.contains(&tool.name.as_ref())
                || browser.iter().any(|b| b["name"] == tool.name.as_ref()),
            "unexplained omission: {}",
            tool.name
        );
    }
    // A registered public file also proves both adapters preserve exactly the same prompt.
    let (a, b) = tokio::io::duplex(65536);
    let prompt_server = tokio::spawn(FigstudioServer::new(vec![file], true).serve(a));
    let prompt_client = ().serve(b).await.unwrap();
    let prompt_server = prompt_server.await.unwrap().unwrap();
    let prompt = prompt_client
        .get_prompt(GetPromptRequestParams::new("create_design_system_rules"))
        .await
        .unwrap();
    let prompt = serde_json::to_value(prompt).unwrap();
    assert_eq!(
        prompt["messages"][0]["content"]["text"],
        figstudio_mcp_core::prompts::rules_text(&doc, None, None)
    );
    prompt_client.cancel().await.unwrap();
    prompt_server.cancel().await.unwrap();
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}
