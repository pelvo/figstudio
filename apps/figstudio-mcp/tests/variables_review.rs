use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::tools::variables::variable_defs;
use serde_json::json;
use std::path::PathBuf;
#[path = "variables/support.rs"]
mod support;
use support::*;

fn map(doc: &Document) -> serde_json::Value {
    variable_defs(doc, "9:1").structured_content.unwrap()
}
fn update(doc: &mut Document, f: impl FnOnce(&mut Vec<Node>)) {
    let mut nodes = doc.nodes.clone();
    f(&mut nodes);
    doc.set_nodes(nodes).unwrap();
}
#[test]
fn number_units_and_float_noise() {
    let mut doc = document();
    update(&mut doc, |nodes| {
        let d = &mut nodes.iter_mut().find(|n| n.id == Guid(9, 31)).unwrap().data;
        d.set(
            "lineHeight",
            obj([
                ("value", Val::Float(120.0)),
                ("units", Val::Enum("PERCENT")),
            ]),
        );
        d.set(
            "letterSpacing",
            obj([
                ("value", Val::Float(f64::from(0.1_f32))),
                ("units", Val::Enum("PIXELS")),
            ]),
        );
        nodes.push(variable(
            40,
            "Fraction",
            vec![obj([(
                "value",
                obj([("floatValue", Val::Float(f64::from(0.1_f32)))]),
            )])],
        ));
        nodes[0]
            .data
            .set("fraction", obj([("variableData", alias(40))]));
    });
    let result = map(&doc);
    assert_eq!(result["Fraction"].to_string(), "0.1");
    assert_eq!(result["Spacing"].to_string(), "8");
    assert!(
        result["Body"]
            .as_str()
            .unwrap()
            .contains("lineHeight: 120%, letterSpacing: 0.1")
    );
    update(&mut doc, |nodes| {
        let d = &mut nodes.iter_mut().find(|n| n.id == Guid(9, 31)).unwrap().data;
        d.set(
            "lineHeight",
            obj([("value", Val::Float(0.0)), ("units", Val::Enum("RAW"))]),
        );
        d.set("textTracking", Val::Float(f64::from(0.2_f32)));
    });
    assert!(
        map(&doc)["Body"]
            .as_str()
            .unwrap()
            .contains("lineHeight: auto, letterSpacing: 0.2")
    );
    update(&mut doc, |nodes| {
        let d = &mut nodes.iter_mut().find(|n| n.id == Guid(9, 31)).unwrap().data;
        d.set("fontWeight", Val::Bool(false));
        d.set(
            "fontName",
            obj([
                ("family", Val::Str("Sans".into())),
                ("style", Val::Str("Semi-Bold".into())),
            ]),
        );
    });
    assert!(map(&doc)["Body"].as_str().unwrap().contains("weight: 600"));
}
#[test]
fn sorted_modes_supply_default_without_invented_field() {
    let mut doc = document();
    update(&mut doc, |nodes| {
        let d = &mut nodes.iter_mut().find(|n| n.id == Guid(9, 10)).unwrap().data;
        d.set("variableSetDefaultModeID", Val::Bool(false));
        d.set(
            "variableSetModes",
            Val::Arr(vec![
                obj([
                    ("id", id(20)),
                    ("name", Val::Str("Light".into())),
                    ("sortPosition", Val::Str("b".into())),
                ]),
                obj([
                    ("id", id(21)),
                    ("name", Val::Str("Dark".into())),
                    ("sortPosition", Val::Str("a".into())),
                ]),
            ]),
        );
    });
    assert_eq!(map(&doc)["Spacing"], json!(8));
    assert_eq!(map(&doc)["Spacing (Light)"], json!(4));
}
#[test]
fn collection_names_disambiguate_variables_and_styles_win_exact_collisions() {
    let mut doc = document();
    update(&mut doc, |nodes| {
        nodes.push(node(11, "VARIABLE_SET", "Other", &[]));
        let mut v = variable(
            40,
            "Spacing",
            vec![obj([("value", obj([("floatValue", Val::Float(12.0))]))])],
        );
        v.data.set("variableSetID", obj([("guid", id(11))]));
        nodes.push(v);
        nodes.push(variable(
            41,
            "Surface",
            vec![obj([("value", obj([("floatValue", Val::Float(99.0))]))])],
        ));
        nodes[0].data.set(
            "extra",
            Val::Arr(vec![
                obj([("variableData", alias(40))]),
                obj([("variableData", alias(41))]),
            ]),
        );
    });
    let result = map(&doc);
    assert_eq!(result["Spacing (Theme)"], json!(8));
    assert_eq!(result["Spacing (Theme) (Light)"], json!(4));
    assert_eq!(result["Spacing (Other)"], json!(12));
    assert_eq!(result["Surface"], "#FF000080");
    assert!(result.get("Spacing").is_none());
    assert!(variable_defs(&doc, "9:1").content.iter().any(|c| {
        c.as_text()
            .is_some_and(|t| t.text.contains("approximations:"))
    }));
}
#[test]
fn hidden_paints_are_omitted_and_opacity_multiplies_alpha() {
    let mut doc = document();
    update(&mut doc, |nodes| {
        let d = &mut nodes.iter_mut().find(|n| n.id == Guid(9, 30)).unwrap().data;
        d.set(
            "fillPaints",
            Val::Arr(vec![
                obj([("type", Val::Enum("IMAGE")), ("visible", Val::Bool(false))]),
                obj([
                    ("type", Val::Enum("SOLID")),
                    ("color", color()),
                    ("opacity", Val::Float(0.5)),
                ]),
                obj([
                    ("type", Val::Enum("SOLID")),
                    ("colorVar", alias(3)),
                    ("opacity", Val::Float(0.5)),
                ]),
            ]),
        );
    });
    assert_eq!(map(&doc)["Surface"], json!(["#FF000040", "#FF000040"]));
}
