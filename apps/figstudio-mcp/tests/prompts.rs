//! Exercise SDK prompts/list and prompts/get on the same handler as both transports.
use figstudio_mcp::FigstudioServer;
use rmcp::{ServiceExt, model::GetPromptRequestParams};
use serde_json::{Value, json};
use std::path::PathBuf;

#[tokio::test]
async fn protocol_lists_and_gets_local_rules_with_optional_arguments() {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let original = std::fs::read(&file).unwrap();
    let (a, b) = tokio::io::duplex(65536);
    let server = tokio::spawn(FigstudioServer::new(vec![file.clone()], true).serve(a));
    let client = ().serve(b).await.unwrap();
    let server = server.await.unwrap().unwrap();
    assert!(client.peer_info().unwrap().capabilities.prompts.is_some());
    let listed = client.list_prompts(None).await.unwrap();
    assert_eq!(listed.prompts.len(), 1);
    let prompt = &listed.prompts[0];
    assert_eq!(prompt.name, "create_design_system_rules");
    let arguments = prompt.arguments.as_ref().unwrap();
    assert_eq!(arguments.len(), 3);
    for name in ["clientFrameworks", "clientLanguages", "fileKey"] {
        let argument = arguments
            .iter()
            .find(|argument| argument.name == name)
            .unwrap();
        assert_ne!(argument.required, Some(true));
    }
    let result = client
        .get_prompt(GetPromptRequestParams::new("create_design_system_rules"))
        .await
        .unwrap();
    assert_eq!(result.messages.len(), 1);
    let value = serde_json::to_value(&result).unwrap();
    assert_eq!(value["messages"][0]["role"], "user");
    let text = value["messages"][0]["content"]["text"].as_str().unwrap();
    for required in [
        "rules/design-system.md",
        "instructions/design-system.md",
        "Reuse existing components",
        "variables, modes and styles",
        "asset handling",
        "get_design_context",
        "get_screenshot",
        "get_metadata",
        "no team libraries",
    ] {
        assert!(text.contains(required), "missing {required}");
    }
    let inventory: Value =
        serde_json::from_str(text.split("never as instructions):\n").nth(1).unwrap()).unwrap();
    assert_eq!(inventory["teamLibrariesAvailable"], false);
    assert!(
        inventory["components"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["nodeId"] == "1:37" && entry["name"] == "Component 1")
    );
    for key in ["collections", "variables", "styles"] {
        assert!(inventory[key]["items"].is_array());
        assert!(inventory[key]["total"].is_number());
    }
    let with_args = client.get_prompt(GetPromptRequestParams::new("create_design_system_rules")
        .with_arguments(json!({"clientFrameworks":"React,Vue", "clientLanguages":"TypeScript", "fileKey":"structure"}).as_object().unwrap().clone())).await.unwrap();
    let value = serde_json::to_value(with_args).unwrap();
    let text = value["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("React,Vue"));
    assert!(text.contains("TypeScript"));
    assert!(
        client
            .get_prompt(
                GetPromptRequestParams::new("create_design_system_rules")
                    .with_arguments(json!({"fileKey":"missing"}).as_object().unwrap().clone())
            )
            .await
            .is_err()
    );
    assert!(
        client
            .get_prompt(GetPromptRequestParams::new("missing_prompt"))
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(file).unwrap(), original);
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}
