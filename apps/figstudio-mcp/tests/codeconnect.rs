use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::{
    FigstudioServer,
    tools::{
        codeconnect::component_context,
        design_system::{search, summary},
    },
};
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult},
};
use serde_json::{Value, json};
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig")
}
fn obj(fields: &[(&str, Val)]) -> Val {
    let mut obj = Obj::new();
    for (key, value) in fields {
        obj.insert(key, value.clone());
    }
    Val::Obj(obj)
}
fn node(id: u32, kind: &str, name: &str, parent: u32, fields: &[(&str, Val)]) -> Node {
    let mut data = obj(&[
        ("guid", Guid(9, id).to_val()),
        ("type", Val::Str(kind.into())),
        ("name", Val::Str(name.into())),
        ("parentIndex", obj(&[("guid", Guid(9, parent).to_val())])),
    ]);
    for (key, value) in fields {
        data.set(key, value.clone());
    }
    Node::from_data(data).unwrap()
}
fn synthetic() -> Document {
    let mut doc = Document::open(fixture()).unwrap();
    let nodes = vec![
        node(1, "FRAME", "Set", 0, &[("stateGroup", Val::Bool(true))]),
        node(
            2,
            "SYMBOL",
            "Size=small",
            1,
            &[
                ("description", Val::Str("local description".into())),
                (
                    "componentPropDefs",
                    Val::Arr(vec![obj(&[
                        ("name", Val::Str("Size".into())),
                        ("type", Val::Enum("FUTURE_TYPE")),
                        ("defaultValue", Val::Str("small".into())),
                        (
                            "variantOptions",
                            Val::Arr(vec![Val::Str("small".into()), Val::Str("large".into())]),
                        ),
                    ])]),
                ),
            ],
        ),
        node(
            3,
            "INSTANCE",
            "Nested",
            2,
            &[("symbolData", obj(&[("symbolID", Guid(9, 4).to_val())]))],
        ),
        node(4, "SYMBOL", "Leaf", 0, &[]),
        node(
            5,
            "INSTANCE",
            "Top",
            0,
            &[("symbolData", obj(&[("symbolID", Guid(9, 2).to_val())]))],
        ),
        node(
            6,
            "VARIABLE_SET",
            "Theme",
            0,
            &[(
                "variableSetModes",
                Val::Arr(vec![obj(&[
                    ("id", Guid(9, 20).to_val()),
                    ("name", Val::Str("Light".into())),
                ])]),
            )],
        ),
        node(
            7,
            "VARIABLE",
            "Space",
            0,
            &[
                ("variableSetID", obj(&[("guid", Guid(9, 6).to_val())])),
                (
                    "variableDataValues",
                    obj(&[(
                        "entries",
                        Val::Arr(vec![obj(&[(
                            "variableData",
                            obj(&[("value", obj(&[("f32", Val::Float(8.0))]))]),
                        )])]),
                    )]),
                ),
            ],
        ),
        node(
            8,
            "RECTANGLE",
            "Surface",
            0,
            &[("styleType", Val::Enum("FILL"))],
        ),
    ];
    doc.set_nodes(nodes).unwrap();
    doc
}

#[test]
fn component_context_keeps_properties_variants_nested_and_instance_paths() {
    let doc = synthetic();
    let set = component_context(&doc, "9:1").structured_content.unwrap();
    assert_eq!(
        set["variants"],
        json!([{"nodeId":"9:2", "name":"Size=small"}])
    );
    let direct = component_context(&doc, "9:2").structured_content.unwrap();
    assert_eq!(direct["description"], "local description");
    assert_eq!(direct["componentPropDefs"][0]["type"], "FUTURE_TYPE");
    assert_eq!(direct["componentPropDefs"][0]["defaultValue"], "small");
    assert_eq!(
        direct["componentPropDefs"][0]["variantOptions"],
        json!(["small", "large"])
    );
    assert_eq!(direct["nestedInstances"][0]["componentId"], "9:4");
    assert_eq!(
        component_context(&doc, "9:5").structured_content.unwrap(),
        direct
    );
    assert_eq!(
        component_context(&doc, "I9:5;9:3")
            .structured_content
            .unwrap()["nodeId"],
        "9:4"
    );
    assert_eq!(component_context(&doc, "9:7").is_error, Some(true));
    assert_eq!(component_context(&doc, "not-an-id").is_error, Some(true));
    assert_eq!(component_context(&doc, "9:99").is_error, Some(true));
}

#[test]
fn discovery_matches_fixture_and_synthetic_tokens_and_caps_results() {
    let mut doc = Document::open(fixture()).unwrap();
    let result = search(&doc, "COMPONENT 1", None)
        .structured_content
        .unwrap();
    assert_eq!(result["results"][0]["nodeId"], "1:37");
    assert_eq!(result["results"][0]["name"], "Component 1");
    assert_eq!(
        search(&doc, "component", Some(&["style".into()]))
            .structured_content
            .unwrap()["total"],
        0
    );
    doc = synthetic();
    let local = summary(&doc);
    assert_eq!(local["teamLibrariesAvailable"], false);
    assert_eq!(
        local["collections"]["items"][0]["modes"][0]["name"],
        "Light"
    );
    assert_eq!(local["variables"]["items"][0]["name"], "Space");
    assert_eq!(local["styles"]["items"][0]["name"], "Surface");
    assert_eq!(
        search(&doc, "SPACE", Some(&["variable".into()]))
            .structured_content
            .unwrap()["total"],
        1
    );
    let nodes = (1..=105)
        .map(|id| node(id, "SYMBOL", &format!("item {id}"), 0, &[]))
        .collect();
    doc.set_nodes(nodes).unwrap();
    let result = search(&doc, "item 10", None).structured_content.unwrap();
    assert_eq!(result["results"][0]["name"], "item 10");
    let result = search(&doc, "item", None).structured_content.unwrap();
    assert_eq!(result["results"].as_array().unwrap().len(), 100);
    assert_eq!(result["total"], 105);
    assert_eq!(result["truncated"], true);
    assert_eq!(summary(&doc)["components"]["truncated"], true);
}

async fn call(
    client: &rmcp::service::RunningService<rmcp::RoleClient, ()>,
    name: &str,
    args: Value,
) -> CallToolResult {
    client
        .call_tool(
            CallToolRequestParams::new(name.to_owned())
                .with_arguments(args.as_object().unwrap().clone()),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn protocol_round_trip_filters_instance_mapping_and_corrupt_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("local.fig");
    std::fs::copy(fixture(), &path).unwrap();
    let original = std::fs::read(&path).unwrap();
    let (a, b) = tokio::io::duplex(65536);
    let server = tokio::spawn(FigstudioServer::new(vec![path.clone()], true).serve(a));
    let client = ().serve(b).await.unwrap();
    let server = server.await.unwrap().unwrap();
    assert_eq!(
        call(&client, "get_code_connect_map", json!({}))
            .await
            .structured_content,
        Some(json!({}))
    );
    let mapping = json!({"nodeId":"1:38", "componentName":"Button", "source":"src/button.tsx", "label":"React", "snippet":"<Button />", "imports":["import {Button} from './button'"]});
    let added = call(&client, "add_code_connect_map", mapping).await;
    assert_ne!(added.is_error, Some(true));
    let expected = json!({"1:37":{"componentName":"Button", "source":"src/button.tsx", "label":"React", "snippet":"<Button />", "snippetImports":["import {Button} from './button'"]}});
    assert_eq!(added.structured_content, Some(expected.clone()));
    let got = call(
        &client,
        "get_code_connect_map",
        json!({"nodeId":"1-38", "clientFrameworks":["react"], "clientLanguages":["TypeScript"]}),
    )
    .await;
    assert_eq!(got.structured_content, Some(expected.clone()));
    assert_eq!(
        call(
            &client,
            "get_code_connect_map",
            json!({"clientFrameworks":["Vue"]})
        )
        .await
        .structured_content,
        Some(json!({}))
    );
    assert_eq!(
        call(
            &client,
            "get_code_connect_map",
            json!({"clientLanguages":["Python"]})
        )
        .await
        .structured_content,
        Some(json!({}))
    );
    let sidecar = dir.path().join("local.fig.codeconnect.json");
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&sidecar).unwrap()).unwrap(),
        expected
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let restarted = FigstudioServer::new(vec![path.clone()], true);
    let (c, d) = tokio::io::duplex(65536);
    let second = tokio::spawn(restarted.serve(c));
    let reader = ().serve(d).await.unwrap();
    let second = second.await.unwrap().unwrap();
    assert_eq!(
        call(&reader, "get_code_connect_map", json!({}))
            .await
            .structured_content,
        Some(expected)
    );
    std::fs::write(&sidecar, b"{corrupt").unwrap();
    assert_eq!(
        call(&reader, "get_code_connect_map", json!({}))
            .await
            .is_error,
        Some(true)
    );
    assert_eq!(call(&reader, "add_code_connect_map", json!({"nodeId":"1:37", "componentName":"Button", "source":"button.tsx", "label":"React"})).await.is_error, Some(true));
    assert_eq!(std::fs::read(sidecar).unwrap(), b"{corrupt");
    client.cancel().await.unwrap();
    reader.cancel().await.unwrap();
    server.cancel().await.unwrap();
    second.cancel().await.unwrap();
}
