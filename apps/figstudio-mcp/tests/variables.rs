use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::{FigstudioServer, tools::variables::variable_defs};
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::{Value, json};
use std::path::PathBuf;

#[path = "variables/support.rs"]
mod support;
use support::{document, fixture};

#[test]
fn used_tokens_modes_runs_legacy_styles_and_effects() {
    let result = variable_defs(&document(), "9:1");
    assert_ne!(result.is_error, Some(true));
    assert!(result.content.iter().any(|c| {
        c.as_text()
            .is_some_and(|t| t.text.contains("approximations:"))
    }));
    let map = result.structured_content.unwrap();
    assert_eq!(map["Spacing"], 8.0);
    assert_eq!(map["Spacing (Light)"], 4.0);
    assert_eq!(map["Alias"], 8.0);
    assert_eq!(map["Alias (Light)"], 4.0);
    assert_eq!(map["Accent"], "#FF000080");
    assert_eq!(map["Surface"], "#FF000080");
    assert_eq!(
        map["Body"],
        "Font(family: \"Sans\", style: Regular, size: 16.0, weight: 400, lineHeight: 24.0, letterSpacing: 0.0)"
    );
    assert_eq!(
        map["Shadow"],
        "Effect(type: DROP_SHADOW, color: #FF000080, offset: (1.0, 2.0), radius: 3.0, spread: 0.0)"
    );
    assert!(map.get("Unused").is_none());
    assert!(map.get("Cycle").is_none());
    assert_eq!(map["Label"], "sample");
    assert_eq!(map["Enabled"], true);
    assert_eq!(map["Grid"], json!([{"pattern":"COLUMNS", "count":12}]));
    assert_eq!(
        map["Gradient"],
        json!({"type":"GRADIENT_LINEAR", "stops":[{"position":0.0,"color":"#FF000080"}]})
    );
    let text = serde_json::to_string(&map).unwrap();
    assert!(text.find("Alias").unwrap() < text.find("Body").unwrap());
}
#[test]
fn invalid_ids_are_errors_and_empty_subtree_has_note() {
    let doc = document();
    for id in ["nope", "99:99", "I9:1;9:5"] {
        assert_eq!(variable_defs(&doc, id).is_error, Some(true));
    }
    let result = variable_defs(&doc, "9:31");
    assert_eq!(result.structured_content, Some(json!({})));
    assert!(
        result
            .content
            .iter()
            .any(|c| c.as_text().is_some_and(|t| t.text.contains("No resolved")))
    );
}
#[test]
fn all_public_fixtures_have_no_style_definitions() {
    for name in [
        "broken_images",
        "corners",
        "noise_and_grain",
        "sections",
        "stacks_wrap",
        "structure",
        "vector",
    ] {
        let doc = Document::open(fixture(name)).unwrap();
        assert!(
            !doc.nodes.iter().any(|n| n.data.get("styleType").is_some()),
            "{name}"
        );
        let result = variable_defs(&doc, &doc.pages[0].to_string());
        assert_ne!(result.is_error, Some(true));
        assert_eq!(result.structured_content, Some(json!({})));
    }
}

#[tokio::test]
async fn protocol_and_fixture_style_subtrees() {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let paths = vec![fixture("vector")];
        let doc = Document::open(&paths[0]).unwrap();
        let args = json!({"nodeId":doc.pages[0].to_string()});
        // Fixtures carry legacy style sentinels, but no resolvable style definitions.
        assert!(!doc.nodes.iter().any(|n| n.data.get("styleType").is_some()));
        let (client_io, server_io) = tokio::io::duplex(65536);
        let task = tokio::spawn(async move {
            FigstudioServer::new(paths, true)
                .serve(server_io)
                .await
                .unwrap()
                .waiting()
                .await
        });
        let client = ().serve(client_io).await.unwrap();
        let tools = client.list_all_tools().await.unwrap();
        let tool = tools
            .iter()
            .find(|t| t.name == "get_variable_defs")
            .unwrap();
        assert_eq!(tool.input_schema["required"], json!(["nodeId"]));
        let result = client
            .call_tool(
                CallToolRequestParams::new("get_variable_defs")
                    .with_arguments(args.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true));
        assert!(matches!(result.structured_content, Some(Value::Object(_))));
        let result = client
            .call_tool(
                CallToolRequestParams::new("get_variable_defs")
                    .with_arguments(json!({"nodeId":"999:999"}).as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        client.cancel().await.unwrap();
        task.await.unwrap().unwrap();
    })
    .await
    .expect("variables protocol timed out");
}
