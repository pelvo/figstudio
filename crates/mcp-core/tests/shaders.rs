//! `list_file_shaders` and `get_shader` on the public shaders playground fixture
//! (`tests/fixtures/shaders.fig`, a Figma Community file, images removed).
use figstudio_fig::Document;
use figstudio_mcp_core::shaders::{get, list};
use serde_json::{Value, json};
use std::path::PathBuf;

fn doc() -> Document {
    Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/shaders.fig"),
    )
    .unwrap()
}

fn shader(doc: &Document, id: &str) -> Value {
    let out = get(doc, id);
    assert!(!out.is_error, "{:?}", out.content);
    out.structured.unwrap()
}

#[test]
fn the_list_holds_fills_and_effects_but_not_generative_plugins() {
    let out = list(&doc()).structured.unwrap();
    let shaders = out["shaders"].as_array().unwrap();
    // 15 code components: 4 fills, 8 effects and 3 generative plugins.
    assert_eq!(out["total"], 12);
    assert_eq!(out["truncated"], false);
    let count = |kind: &str| shaders.iter().filter(|s| s["type"] == kind).count();
    assert_eq!((count("fill"), count("effect")), (4, 8));
    // Every shader in the file is used by at least one layer.
    assert!(shaders.iter().all(|s| s["usedBy"].as_u64().unwrap() >= 1));
    // What the file cannot hold is reported as missing, not invented.
    assert!(
        shaders
            .iter()
            .all(|s| s["name"].is_null() && s["files"] == json!([]))
    );
    assert!(out["note"].as_str().unwrap().contains("Figma's cloud"));
}

#[test]
fn a_shader_is_found_by_its_id_key_or_node_id() {
    let doc = doc();
    let by_id = shader(&doc, "1650660541440366682");
    assert_eq!(by_id["nodeId"], "1:5");
    assert_eq!(by_id["version"], "430");
    assert_eq!(by_id["published"], true);
    assert_eq!(
        shader(&doc, "c2b302fbdfce8a1ec92a5a80b10a5916ab506ed7"),
        by_id
    );
    assert_eq!(shader(&doc, "1:5"), by_id);
}

#[test]
fn parameters_come_in_panel_order_with_ranges_and_options() {
    let s = shader(&doc(), "1:5");
    let params = s["parameters"].as_array().unwrap();
    let names: Vec<_> = params.iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "shape",
            "ringCount",
            "falloff",
            "inverseFalloff",
            "color",
            "offset",
            "transform"
        ]
    );
    let falloff = &params[2];
    assert_eq!(falloff["label"], "Falloff");
    assert_eq!(falloff["control"], "SLIDER");
    // Stored as f32: rounded so 0.1 reads as 0.1.
    assert_eq!(
        (&falloff["min"], &falloff["max"], &falloff["step"]),
        (&json!(0.0), &json!(5.0), &json!(0.1))
    );
    assert_eq!(
        params[0]["options"][1],
        json!({"label": "Circle", "value": 1.0})
    );
    assert_eq!(params[3]["default"], false);
}

#[test]
fn usages_name_the_layer_field_and_values_by_parameter() {
    let s = shader(&doc(), "1:5");
    assert_eq!(s["usedBy"], 1);
    let usage = &s["usages"][0];
    assert_eq!(usage["nodeId"], "1:14622");
    assert_eq!(usage["field"], "fillPaints[1]");
    assert_eq!(usage["values"]["offset"], 0.77);
    assert_eq!(usage["values"]["transform"]["radius"], 144.0);
    // An effect used by two layers lists both.
    let effect = shader(&doc(), "33d4bf82a8f949caa6b12336d86f0b723ee8209e");
    assert_eq!(effect["type"], "effect");
    assert_eq!(effect["usages"].as_array().unwrap().len(), 2);
    assert!(
        effect["usages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u["field"].as_str().unwrap().starts_with("effects["))
    );
}

#[test]
fn an_unknown_shader_is_a_tool_error() {
    let out = get(&doc(), "nope");
    assert!(out.is_error);
    // A generative plugin is a code component too, but not a shader.
    assert!(get(&doc(), "1:14").is_error);
}
