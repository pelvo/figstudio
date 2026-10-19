//! Synthetic component property and variable regressions.
mod common;
use common::*;
use figstudio_fig::Val;

fn property_ref(def: u32, field: &str) -> Val {
    Val::Arr(vec![obj(&[
        ("defID", id(def).to_val()),
        ("componentPropNodeField", Val::Str(field.into())),
    ])])
}
fn assignment(def: u32, key: &str, value: Val) -> Val {
    obj(&[("defID", id(def).to_val()), ("value", obj(&[(key, value)]))])
}

#[test]
fn component_boolean_and_swap_assignments_are_scoped_to_each_instance() {
    let mut child = instance(3, 2, 4, vec![]);
    child.data.set(
        "componentPropRefs",
        property_ref(90, "OVERRIDDEN_SYMBOL_ID"),
    );
    let mut hidden = node(7, 2, "RECTANGLE");
    hidden.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    hidden
        .data
        .set("componentPropRefs", property_ref(91, "VISIBLE"));
    let mut red = node(5, 4, "RECTANGLE");
    red.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let mut green = node(8, 6, "RECTANGLE");
    green.data.set("fillPaints", paint(0.0, 1.0, 0.0));
    let mut first = instance(10, 1, 2, vec![]);
    first.data.set(
        "componentPropAssignments",
        Val::Arr(vec![
            assignment(90, "guidValue", id(6).to_val()),
            assignment(91, "boolValue", Val::Bool(false)),
        ]),
    );
    let mut second = instance(11, 1, 2, vec![]);
    second
        .data
        .set("transform", obj(&[("m02", Val::Float(40.0))]));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        child,
        hidden,
        node(4, 0, "SYMBOL"),
        red,
        node(6, 0, "SYMBOL"),
        green,
        first,
        second,
    ]);
    let px = pixels(&doc);
    assert_eq!(pixel(&px, 10, 10), &[0, 255, 0, 255]);
    assert_eq!(pixel(&px, 50, 10), &[255, 0, 0, 255]);
}

#[test]
fn parameter_consumption_map_binding_wins_over_a_stale_component_prop_ref() {
    let mut symbol = node(2, 0, "SYMBOL");
    symbol.data.set(
        "componentPropDefs",
        Val::Arr(vec![
            obj(&[
                ("id", id(91).to_val()),
                ("initialValue", obj(&[("boolValue", Val::Bool(true))])),
            ]),
            obj(&[
                ("id", id(92).to_val()),
                ("initialValue", obj(&[("boolValue", Val::Bool(true))])),
            ]),
        ]),
    );
    // A copied layer: the legacy ref still names 91, the consumption map rebinds it to 92.
    let mut rect = node(3, 2, "RECTANGLE");
    rect.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    rect.data
        .set("componentPropRefs", property_ref(91, "VISIBLE"));
    rect.data.set(
        "parameterConsumptionMap",
        obj(&[(
            "entries",
            Val::Arr(vec![obj(&[
                ("variableField", Val::Str("VISIBLE".into())),
                (
                    "variableData",
                    obj(&[
                        ("dataType", Val::Str("PROP_REF".into())),
                        (
                            "value",
                            obj(&[("propRefValue", obj(&[("defId", id(92).to_val())]))]),
                        ),
                    ]),
                ),
            ])]),
        )]),
    );
    let mut top = instance(4, 1, 2, vec![]);
    top.data.set(
        "componentPropAssignments",
        Val::Arr(vec![assignment(91, "boolValue", Val::Bool(false))]),
    );
    let doc = document(vec![node(1, 0, "CANVAS"), symbol, rect, top]);
    assert_eq!(pixel(&pixels(&doc), 10, 10), &[255, 0, 0, 255]);
}

#[test]
fn property_text_uses_fresh_derived_glyphs_and_defaults_follow_parent_definitions() {
    let mut symbol = node(2, 0, "SYMBOL");
    symbol.data.set(
        "componentPropDefs",
        Val::Arr(vec![obj(&[
            ("id", id(91).to_val()),
            ("parentPropDefId", id(92).to_val()),
        ])]),
    );
    let mut definitions = node(9, 0, "FRAME");
    definitions.data.set(
        "componentPropDefs",
        Val::Arr(vec![obj(&[
            ("id", id(92).to_val()),
            ("initialValue", obj(&[("boolValue", Val::Bool(true))])),
        ])]),
    );
    let mut text = node(3, 2, "TEXT");
    text.data.set("visible", Val::Bool(false));
    text.data.set(
        "componentPropRefs",
        Val::Arr(vec![
            obj(&[
                ("defID", id(90).to_val()),
                ("componentPropNodeField", Val::Str("TEXT_DATA".into())),
            ]),
            obj(&[
                ("defID", id(91).to_val()),
                ("componentPropNodeField", Val::Str("VISIBLE".into())),
            ]),
        ]),
    );
    text.data
        .set("textData", obj(&[("characters", Val::Str("old".into()))]));
    text.data.set("fillPaints", paint(0.0, 0.0, 1.0));
    let mut top = instance(4, 1, 2, vec![]);
    top.data.set(
        "componentPropAssignments",
        Val::Arr(vec![assignment(
            90,
            "textValue",
            obj(&[("characters", Val::Str("new".into()))]),
        )]),
    );
    let mut doc = document(vec![
        node(1, 0, "CANVAS"),
        symbol,
        text,
        definitions,
        top.clone(),
    ]);
    // An em-square glyph with a baseline at 20: visible only if the boolean default is applied.
    let blob = doc.blobs.len() as i64;
    let mut commands = Vec::new();
    for (cmd, x, y) in [(1, 0f32, 0f32), (2, 1.0, 0.0), (2, 1.0, 1.0), (2, 0.0, 1.0)] {
        commands.push(cmd);
        commands.extend(x.to_le_bytes());
        commands.extend(y.to_le_bytes());
    }
    commands.push(0);
    doc.blobs.push(commands);
    top.data.set(
        "derivedSymbolData",
        Val::Arr(vec![patch(
            &[3],
            &[(
                "derivedTextData",
                obj(&[(
                    "glyphs",
                    Val::Arr(vec![obj(&[
                        ("commandsBlob", Val::Int(blob)),
                        ("fontSize", Val::Float(20.0)),
                        ("position", size(0.0, 20.0)),
                    ])]),
                )]),
            )],
        )]),
    );
    let mut nodes = doc.nodes.clone();
    *nodes.iter_mut().find(|n| n.id == id(4)).unwrap() = top;
    doc.set_nodes(nodes).unwrap();
    assert_eq!(pixel(&pixels(&doc), 10, 10), &[0, 0, 255, 255]);
    // Removing fresh derived glyphs must not draw the component's old stored outline.
    let mut nodes = doc.nodes.clone();
    let fresh = nodes
        .iter()
        .find(|n| n.id == id(4))
        .unwrap()
        .data
        .get("derivedSymbolData")
        .unwrap()
        .items()[0]
        .get("derivedTextData")
        .unwrap()
        .clone();
    nodes
        .iter_mut()
        .find(|n| n.id == id(3))
        .unwrap()
        .data
        .set("derivedTextData", fresh);
    nodes
        .iter_mut()
        .find(|n| n.id == id(4))
        .unwrap()
        .data
        .set("derivedSymbolData", Val::Arr(vec![]));
    doc.set_nodes(nodes).unwrap();
    assert_eq!(pixel(&pixels(&doc), 10, 10), &[0, 0, 0, 0]);
}

#[test]
fn paint_aliases_select_the_exact_asset_version_and_follow_guid_chains() {
    let mut red = node(3, 2, "RECTANGLE");
    let mut paints = paint(1.0, 0.0, 0.0);
    if let Val::Arr(p) = &mut paints {
        p[0].set(
            "colorVar",
            obj(&[(
                "value",
                obj(&[(
                    "alias",
                    obj(&[(
                        "assetRef",
                        obj(&[
                            ("key", Val::Str("public-token".into())),
                            ("version", Val::Str("v2".into())),
                        ]),
                    )]),
                )]),
            )]),
        );
    }
    red.data.set("fillPaints", paints);
    let mut alias = node(7, 0, "VARIABLE");
    alias.data.set("key", Val::Str("public-token".into()));
    alias.data.set("version", Val::Str("v2".into()));
    alias.data.set(
        "variableDataValues",
        obj(&[(
            "entries",
            Val::Arr(vec![obj(&[(
                "variableData",
                obj(&[("value", obj(&[("alias", obj(&[("guid", id(8).to_val())]))]))]),
            )])]),
        )]),
    );
    let mut color = node(8, 0, "VARIABLE");
    color.data.set(
        "variableDataValues",
        obj(&[(
            "entries",
            Val::Arr(vec![obj(&[(
                "variableData",
                obj(&[(
                    "value",
                    obj(&[(
                        "colorValue",
                        obj(&[
                            ("r", Val::Float(0.0)),
                            ("g", Val::Float(1.0)),
                            ("b", Val::Float(0.0)),
                            ("a", Val::Float(1.0)),
                        ]),
                    )]),
                )]),
            )])]),
        )]),
    );
    let mut other = alias.clone();
    other.id = id(9);
    other.data.set("guid", id(9).to_val());
    other.data.set("version", Val::Str("v1".into()));
    other
        .data
        .set("variableDataValues", obj(&[("entries", Val::Arr(vec![]))]));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        red,
        alias,
        color,
        other,
        instance(4, 1, 2, vec![]),
    ]);
    assert_eq!(pixel(&pixels(&doc), 10, 10), &[0, 255, 0, 255]);
}
