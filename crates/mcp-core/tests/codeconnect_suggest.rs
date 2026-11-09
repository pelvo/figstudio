//! `get_code_connect_suggestions` core: which components a selection uses that have no local
//! mapping yet.
use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::codeconnect_suggest::{ALL_CONNECTED, NONE_FOUND, suggestions};
use serde_json::{Value, json};
use std::path::PathBuf;

fn open(name: &str) -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/fixtures/{name}.fig")),
    )
    .unwrap()
}

fn suggest(doc: &Document, id: &str, mapped: &[&str], exclude: bool) -> Value {
    let out = suggestions(
        doc,
        id,
        |g: Guid| mapped.contains(&g.to_string().as_str()),
        exclude,
    );
    assert!(!out.is_error, "{:?}", out.content);
    out.structured.unwrap()
}

#[test]
fn instances_on_a_page_suggest_their_component_once() {
    let doc = open("structure");
    let out = suggest(&doc, "0:1", &[], false);
    assert_eq!(out["total"], 1);
    let c = &out["components"][0];
    assert_eq!(c["mainComponentNodeId"], "1:37");
    assert_eq!(c["type"], "COMPONENT");
    assert_eq!(c["instanceCount"], 2);
    assert!(c["properties"].is_array());
    assert!(
        out["prompt"]
            .as_str()
            .unwrap()
            .contains("send_code_connect_mappings")
    );
    // The lightweight form leaves out properties and guidance.
    let light = suggest(&doc, "0:1", &[], true);
    assert!(light["components"][0].get("properties").is_none());
    assert!(light.get("prompt").is_none());
}

#[test]
fn mapped_components_and_empty_selections_say_so() {
    let doc = open("structure");
    assert_eq!(
        suggest(&doc, "0:1", &["1:37"], false)["message"],
        ALL_CONNECTED
    );
    assert_eq!(suggest(&doc, "1:34", &[], false)["message"], NONE_FOUND);
    // An instance alone suggests its component.
    assert_eq!(
        suggest(&doc, "1:38", &[], true)["components"][0]["mainComponentNodeId"],
        "1:37"
    );
}

#[test]
fn variants_are_suggested_as_their_set_and_the_list_is_capped() {
    let doc = open("shaders");
    let out = suggest(&doc, "0:1", &[], true);
    let components = out["components"].as_array().unwrap();
    assert_eq!(out["total"], 96);
    assert_eq!(components.len(), 96);
    assert_eq!(out["truncated"], false);
    // No variant appears on its own: its set stands for it.
    for c in components {
        let id: Guid = c["mainComponentNodeId"].as_str().unwrap().parse().unwrap();
        let parent = doc
            .node(id)
            .unwrap()
            .parent
            .and_then(|p| doc.node(p))
            .unwrap();
        let in_set =
            parent.kind == "COMPONENT_SET" || parent.data.flag("isStateGroup") == Some(true);
        assert!(!(c["type"] == "COMPONENT" && in_set), "{c}");
    }
    let set = components
        .iter()
        .find(|c| c["mainComponentNodeId"] == "1:1387")
        .expect("the most used set");
    assert_eq!(set["type"], "COMPONENT_SET");
    // Mapping the set connects every variant.
    let after = suggest(&doc, "0:1", &["1:1387"], true);
    assert!(
        !after["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["mainComponentNodeId"] == "1:1387")
    );
    assert_eq!(after["total"], json!(out["total"].as_u64().unwrap() - 1));
}

#[test]
fn a_long_list_is_capped_at_100() {
    // The internal canvas holds every component of the file.
    let out = suggest(&open("shaders"), "0:2", &[], true);
    assert_eq!(out["components"].as_array().unwrap().len(), 100);
    assert_eq!(out["truncated"], true);
    assert!(out["total"].as_u64().unwrap() > 100);
}

#[test]
fn bad_targets_are_tool_errors() {
    let doc = open("structure");
    assert!(suggestions(&doc, "9:9999", |_| false, false).is_error);
    assert!(suggestions(&doc, "0:0", |_| false, false).is_error);
}
