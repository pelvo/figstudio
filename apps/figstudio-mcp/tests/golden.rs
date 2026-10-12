//! Golden record of every MCP tool over the public fixtures (L0 harness).
//!
//! Ignored by default. `scripts/mcp-golden.sh <rev>` runs it in two worktrees and diffs
//! the output. Set `FIGSTUDIO_GOLDEN_DIR` to choose where the JSON records go.
use figstudio_mcp::{FigstudioServer, tools::design::Assets};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, GetPromptRequestParams},
};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};

const FIXTURES: [&str; 7] = [
    "broken_images",
    "corners",
    "noise_and_grain",
    "sections",
    "stacks_wrap",
    "structure",
    "vector",
];

/// Every `(tag, id)` pair of the XML reply, in document order.
fn tagged_ids(xml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for piece in xml.split('<').skip(1) {
        let Some(tag_end) = piece.find([' ', '>', '/']) else {
            continue;
        };
        let tag = &piece[..tag_end];
        let head = piece.split('>').next().unwrap_or("");
        if let Some(value) = head
            .split(" id=\"")
            .nth(1)
            .and_then(|v| v.split('"').next())
        {
            out.push((tag.to_string(), value.to_string()));
        }
    }
    out
}

fn args(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn write(dir: &Path, name: &str, value: &Value) {
    let text = serde_json::to_string_pretty(value).unwrap() + "\n";
    std::fs::write(dir.join(name), text).unwrap();
}

fn text_of(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

#[tokio::test]
#[ignore = "golden record; run by scripts/mcp-golden.sh"]
async fn golden_record() {
    let dir =
        PathBuf::from(std::env::var_os("FIGSTUDIO_GOLDEN_DIR").expect("set FIGSTUDIO_GOLDEN_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    // Relative registration: the paths in the output match across worktrees.
    let paths: Vec<PathBuf> = FIXTURES
        .iter()
        .map(|stem| PathBuf::from(format!("../../tests/fixtures/{stem}.fig")))
        .collect();
    let (a, b) = tokio::io::duplex(1 << 24);
    let server = FigstudioServer::new(paths, true).with_assets(Assets::Inline);
    let server_task = tokio::spawn(server.serve(a));
    let client = ().serve(b).await.unwrap();
    let _server = server_task.await.unwrap().unwrap();

    // Sorted by name: the router's own order is not part of the contract being recorded.
    let mut tools = client.list_all_tools().await.unwrap();
    tools.sort_by(|x, y| x.name.cmp(&y.name));
    write(
        &dir,
        "00-tools-list.json",
        &serde_json::to_value(&tools).unwrap(),
    );
    let prompts = client.list_all_prompts().await.unwrap();
    write(
        &dir,
        "00-prompts-list.json",
        &serde_json::to_value(&prompts).unwrap(),
    );
    // The registry holds 7 files, so the prompt needs an explicit fileKey.
    let rules = client
        .get_prompt(
            GetPromptRequestParams::new("create_design_system_rules")
                .with_arguments(args(json!({"fileKey": "structure"}))),
        )
        .await
        .unwrap();
    write(
        &dir,
        "00-prompts-get.json",
        &serde_json::to_value(&rules).unwrap(),
    );

    let call = |name: &'static str, arguments: Value| {
        let client = &client;
        async move {
            let result = client
                .call_tool(CallToolRequestParams::new(name).with_arguments(args(arguments)))
                .await
                .unwrap();
            serde_json::to_value(&result).unwrap()
        }
    };
    write(&dir, "00-whoami.json", &call("whoami", json!({})).await);
    write(
        &dir,
        "00-list_files.json",
        &call("list_files", json!({})).await,
    );

    for stem in FIXTURES {
        let fdir = dir.join(stem);
        std::fs::create_dir_all(&fdir).unwrap();
        let key = json!(stem);
        let mut n = 0;
        let mut step = |tool: &str, value: &Value| {
            n += 1;
            write(&fdir, &format!("{n:02}-{tool}.json"), value);
        };

        let root = call("get_metadata", json!({"fileKey": key})).await;
        step("get_metadata-root", &root);
        let pages: Vec<String> = tagged_ids(&text_of(&root))
            .into_iter()
            .filter(|(tag, _)| tag == "canvas")
            .map(|(_, id)| id)
            .collect();

        let mut frames: Vec<String> = Vec::new();
        let mut component: Option<String> = None;
        for page in &pages {
            let meta = call(
                "get_metadata",
                json!({"fileKey": key, "nodeId": page, "depth": 2}),
            )
            .await;
            step(&format!("get_metadata-{page}"), &meta);
            for (tag, id) in tagged_ids(&text_of(&meta)) {
                match tag.as_str() {
                    "frame" if frames.len() < 3 && !frames.contains(&id) => frames.push(id),
                    "symbol" if component.is_none() => component = Some(id),
                    _ => {}
                }
            }
        }
        for page in &pages {
            let shot = call(
                "get_screenshot",
                json!({"fileKey": key, "nodeId": page, "scale": 0.25}),
            )
            .await;
            step(&format!("get_screenshot-{page}"), &shot);
        }
        for frame in &frames {
            let shot = call("get_screenshot", json!({"fileKey": key, "nodeId": frame})).await;
            step(&format!("get_screenshot-{frame}"), &shot);
            let ctx = call(
                "get_design_context",
                json!({"fileKey": key, "nodeId": frame}),
            )
            .await;
            step(&format!("get_design_context-{frame}"), &ctx);
            let vars = call(
                "get_variable_defs",
                json!({"fileKey": key, "nodeId": frame}),
            )
            .await;
            step(&format!("get_variable_defs-{frame}"), &vars);
        }

        // First five frames, then pages, as PNG asset requests.
        let mut asset_ids: Vec<String> =
            frames.iter().chain(pages.iter()).take(5).cloned().collect();
        asset_ids.dedup();
        let assets = call(
            "download_assets",
            json!({"fileKey": key, "nodeIds": asset_ids}),
        )
        .await;
        step("download_assets", &assets);
        step(
            "search_design_system",
            &call(
                "search_design_system",
                json!({"fileKey": key, "query": "a"}),
            )
            .await,
        );
        step(
            "get_libraries",
            &call("get_libraries", json!({"fileKey": key})).await,
        );
        step(
            "get_figjam",
            &call("get_figjam", json!({"fileKey": key})).await,
        );
        if let Some(component) = &component {
            let ctx = call(
                "get_context_for_code_connect",
                json!({"fileKey": key, "nodeId": component}),
            )
            .await;
            step("get_context_for_code_connect", &ctx);
        }
        step(
            "get_code_connect_map",
            &call("get_code_connect_map", json!({"fileKey": key})).await,
        );

        // One invalid id per node-taking tool.
        step(
            "invalid-get_metadata",
            &call("get_metadata", json!({"fileKey": key, "nodeId": "x:y"})).await,
        );
        step(
            "invalid-get_screenshot",
            &call("get_screenshot", json!({"fileKey": key, "nodeId": "x:y"})).await,
        );
        step(
            "invalid-get_design_context",
            &call(
                "get_design_context",
                json!({"fileKey": key, "nodeId": "x:y"}),
            )
            .await,
        );
        step(
            "invalid-get_variable_defs",
            &call(
                "get_variable_defs",
                json!({"fileKey": key, "nodeId": "x:y"}),
            )
            .await,
        );
        step(
            "invalid-download_assets",
            &call(
                "download_assets",
                json!({"fileKey": key, "nodeIds": ["x:y"]}),
            )
            .await,
        );
        step(
            "invalid-get_context_for_code_connect",
            &call(
                "get_context_for_code_connect",
                json!({"fileKey": key, "nodeId": "x:y"}),
            )
            .await,
        );
        step(
            "invalid-get_code_connect_map",
            &call(
                "get_code_connect_map",
                json!({"fileKey": key, "nodeId": "x:y"}),
            )
            .await,
        );
    }
    client.cancel().await.unwrap();
}
