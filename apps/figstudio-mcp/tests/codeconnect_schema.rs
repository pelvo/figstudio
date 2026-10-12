//! Real schema properties, bounded payloads and public fixture component sets.
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::{FigstudioServer, tools::codeconnect::component_context};
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::json;
use std::path::PathBuf;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sections.fig")
}
fn obj(fields: &[(&str, Val)]) -> Val {
    let mut result = Obj::new();
    for (key, value) in fields {
        result.insert(key, value.clone());
    }
    Val::Obj(result)
}

#[tokio::test]
async fn sections_component_set_and_variant_are_discovered_through_tools() {
    let (a, b) = tokio::io::duplex(65536);
    let server = tokio::spawn(FigstudioServer::new(vec![fixture()], true).serve(a));
    let client = ().serve(b).await.unwrap();
    let server = server.await.unwrap().unwrap();
    for id in ["1:4", "1:3"] {
        let result = client
            .call_tool(
                CallToolRequestParams::new("get_context_for_code_connect")
                    .with_arguments(json!({"nodeId":id}).as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "{id}");
        let value = result.structured_content.unwrap();
        assert_eq!(value["componentPropDefs"][0]["type"], "VARIANT");
        assert!(value["componentPropDefs"][0]["id"].is_string());
        assert!(
            value["variants"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v["nodeId"] == "1:3")
        );
        assert!(
            value["variantOptions"]
                .as_object()
                .unwrap()
                .values()
                .any(|v| !v.as_array().unwrap().is_empty())
        );
    }
    let result = client
        .call_tool(
            CallToolRequestParams::new("search_design_system").with_arguments(
                json!({"query":"Component", "kinds":["component_set"]})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(
        result.structured_content.unwrap()["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["nodeId"] == "1:4")
    );
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}

#[test]
fn real_property_shapes_normalize_guids_and_drop_binary_payloads() {
    let mut doc = Document::open(fixture()).unwrap();
    let mut set = doc.node(Guid(1, 4)).unwrap().clone();
    set.data.set(
        "componentPropDefs",
        Val::Arr(vec![obj(&[
            ("id", Guid(8, 1).to_val()),
            ("name", Val::Str("Size".into())),
            ("type", Val::Enum("FUTURE_TYPE")),
            ("sortPosition", Val::Str("!".into())),
            ("parentPropDefId", Guid(8, 2).to_val()),
            (
                "initialValue",
                obj(&[
                    ("guidValue", Guid(8, 3).to_val()),
                    (
                        "textValue",
                        obj(&[
                            ("characters", Val::Str("small".into())),
                            (
                                "styleOverrideTable",
                                Val::Arr(vec![obj(&[
                                    (
                                        "fontName",
                                        obj(&[("fontDigest", Val::Bytes(vec![1, 2, 3]))]),
                                    ),
                                    (
                                        "componentPropRefs",
                                        Val::Arr(vec![obj(&[("defID", Guid(8, 6).to_val())])]),
                                    ),
                                ])]),
                            ),
                        ]),
                    ),
                ]),
            ),
            (
                "preferredValues",
                obj(&[(
                    "instanceSwapValues",
                    Val::Arr(vec![obj(&[
                        ("type", Val::Enum("COMPONENT")),
                        ("key", Val::Str("local-key".into())),
                    ])]),
                )]),
            ),
            (
                "varValue",
                obj(&[
                    ("dataType", Val::Enum("SYMBOL_ID")),
                    (
                        "value",
                        obj(&[("symbolIdValue", obj(&[("guid", Guid(8, 5).to_val())]))]),
                    ),
                ]),
            ),
        ])]),
    );
    set.data.set(
        "stateGroupPropertyValueOrders",
        Val::Arr(vec![obj(&[
            ("property", Val::Str("Size".into())),
            (
                "values",
                Val::Arr(vec![Val::Str("small".into()), Val::Str("large".into())]),
            ),
        ])]),
    );
    let mut variant = doc.node(Guid(1, 3)).unwrap().clone();
    variant.data.set(
        "variantPropSpecs",
        Val::Arr(vec![obj(&[
            ("propDefId", Guid(8, 1).to_val()),
            ("value", Val::Str("small".into())),
        ])]),
    );
    doc.set_nodes(vec![set.clone(), variant]).unwrap();
    let result = component_context(&doc, "1:3").structured_content.unwrap();
    let prop = &result["componentPropDefs"][0];
    assert_eq!(prop["id"], "8:1");
    assert_eq!(prop["parentPropDefId"], "8:2");
    assert_eq!(prop["initialValue"]["guidValue"], "8:3");
    assert_eq!(prop["initialValue"]["textValue"]["characters"], "small");
    let text_style = &prop["initialValue"]["textValue"]["styleOverrideTable"][0];
    assert!(text_style["fontName"].get("fontDigest").is_none());
    assert_eq!(text_style["componentPropRefs"][0]["defID"], "8:6");
    assert_eq!(
        prop["preferredValues"]["instanceSwapValues"][0]["key"],
        "local-key"
    );
    assert_eq!(prop["varValue"]["value"]["symbolIdValue"]["guid"], "8:5");
    assert_eq!(prop["sortPosition"], "!");
    assert_eq!(prop["type"], "FUTURE_TYPE");
    assert_eq!(result["variantOptions"], json!({"Size":["small", "large"]}));
    assert_eq!(
        result["variants"][0]["variantProperties"],
        json!({"Size":"small"})
    );
    set.data.set(
        "componentPropDefs",
        Val::Arr(
            (0..105)
                .map(|i| obj(&[("id", Guid(8, i).to_val())]))
                .collect(),
        ),
    );
    doc.set_nodes(vec![set]).unwrap();
    let result = component_context(&doc, "1:4").structured_content.unwrap();
    assert_eq!(result["componentPropDefs"].as_array().unwrap().len(), 100);
    assert_eq!(result["truncated"], true);
}

#[tokio::test]
async fn variant_mapping_falls_back_to_set_but_keeps_requested_key() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("local.fig");
    let mut doc = Document::open(fixture()).unwrap();
    let instance = Node::from_data(obj(&[
        ("guid", Guid(8, 9).to_val()),
        ("type", Val::Enum("INSTANCE")),
        ("symbolData", obj(&[("symbolID", Guid(1, 3).to_val())])),
    ]))
    .unwrap();
    let mut nodes = doc.nodes.clone();
    nodes.push(instance);
    doc.set_nodes(nodes).unwrap();
    doc.save(&file).unwrap();
    std::fs::write(
        dir.path().join("local.fig.codeconnect.json"),
        br#"{"1:4":{"componentName":"Set","source":"set.tsx","label":"React"}}"#,
    )
    .unwrap();
    let (a, b) = tokio::io::duplex(65536);
    let server = tokio::spawn(FigstudioServer::new(vec![file], true).serve(a));
    let client = ().serve(b).await.unwrap();
    let server = server.await.unwrap().unwrap();
    let result = client
        .call_tool(
            CallToolRequestParams::new("get_code_connect_map")
                .with_arguments(json!({"nodeId":"1:3"}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        result.structured_content,
        Some(
            json!({"1:3":{"componentName":"Set","source":"set.tsx","label":"React","snippet":null,"snippetImports":[]}})
        )
    );
    let sidecar = dir.path().join("local.fig.codeconnect.json");
    for (contents, expected) in [
        (
            json!({"1:4":{"componentName":"Set", "source":"set.tsx", "label":"React"}}),
            "Set",
        ),
        (
            json!({"1:4":{"componentName":"Set", "source":"set.tsx", "label":"React"}, "1:3":{"componentName":"Variant", "source":"variant.tsx", "label":"React"}}),
            "Variant",
        ),
        (
            json!({"1:3":{"componentName":"Variant", "source":"variant.tsx", "label":"React"}, "8:9":{"componentName":"Instance", "source":"instance.tsx", "label":"React"}}),
            "Instance",
        ),
    ] {
        std::fs::write(&sidecar, serde_json::to_vec(&contents).unwrap()).unwrap();
        let result = client
            .call_tool(
                CallToolRequestParams::new("get_code_connect_map")
                    .with_arguments(json!({"nodeId":"8:9"}).as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(
            result.structured_content.unwrap()["8:9"]["componentName"],
            expected
        );
    }
    client.cancel().await.unwrap();
    server.cancel().await.unwrap();
}
