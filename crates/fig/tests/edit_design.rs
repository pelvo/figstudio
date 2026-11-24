//! Styles of every kind and variables (collections, modes, four types, binding).

use std::path::PathBuf;

use figstudio_fig::edit::{
    BindField, EditOp, EffectSpec, History, PaintSpec, StyleKind, VarType, VarValue,
    execute_edit_ops,
};
use figstudio_fig::{Document, Guid, Val};

struct Fx {
    doc: Document,
    h: History,
    page: Guid,
}

impl Fx {
    fn new() -> Fx {
        let doc = Document::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
        )
        .unwrap();
        let page = doc.pages[0];
        Fx {
            doc,
            h: History::new(50),
            page,
        }
    }

    fn run(&mut self, ops: Vec<EditOp>) -> Result<Vec<Guid>, String> {
        execute_edit_ops(&mut self.doc, &mut self.h, &ops, None, None, false)
            .map(|r| r.0.created_ids)
            .map_err(|e| e.to_string())
    }

    fn json(&mut self, op: serde_json::Value) -> Result<Vec<Guid>, String> {
        let op: EditOp = serde_json::from_value(op).map_err(|e| e.to_string())?;
        self.run(vec![op])
    }

    fn rect(&mut self) -> Guid {
        let page = self.page;
        self.run(vec![EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: None,
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 50.0,
            corner_radius: 0.0,
            fill_hex: Some("#FFFFFF".into()),
            count: None,
            star_inner_scale: None,
        }])
        .unwrap()[0]
    }

    fn data(&self, id: Guid) -> &Val {
        &self.doc.node(id).unwrap().data
    }
}

fn solid(hex: &str) -> PaintSpec {
    PaintSpec::Solid {
        hex: hex.into(),
        opacity: None,
        visible: None,
        blend_mode: None,
    }
}

fn red(v: &Val, field: &str) -> f32 {
    v.get(field).unwrap().items()[0]
        .path(&["color", "r"])
        .and_then(Val::as_f32)
        .unwrap()
}

#[test]
fn a_paint_style_update_reaches_every_layer_and_delete_detaches() {
    let mut f = Fx::new();
    let (a, b) = (f.rect(), f.rect());
    let style = f
        .run(vec![EditOp::CreateStyle {
            kind: StyleKind::Paint,
            name: "Brand".into(),
            from_node: None,
            paints: Some(vec![solid("#00FF00")]),
            effects: None,
        }])
        .unwrap()[0];
    let st = style.to_string();
    f.json(serde_json::json!({ "op": "bind_style", "target": a.to_string(), "slot": "fill", "style": st }))
        .unwrap();
    f.json(serde_json::json!({ "op": "bind_style", "target": b.to_string(), "slot": "stroke", "style": st }))
        .unwrap();
    f.run(vec![EditOp::UpdateStyle {
        style,
        name: Some("Brand/Red".into()),
        from_node: None,
        paints: Some(vec![solid("#FF0000")]),
        effects: None,
    }])
    .unwrap();
    assert_eq!(red(f.data(a), "fillPaints"), 1.0);
    assert_eq!(red(f.data(b), "strokePaints"), 1.0);
    assert_eq!(f.doc.node(style).unwrap().name, "Brand/Red");

    f.run(vec![EditOp::DeleteStyle { style }]).unwrap();
    assert!(f.doc.node(style).is_none());
    assert!(f.data(a).get("styleIdForFill").is_none());
    assert_eq!(
        red(f.data(a), "fillPaints"),
        1.0,
        "the look stays after delete"
    );
    let reopened = Document::from_bytes(&f.doc.to_bytes().unwrap()).unwrap();
    assert!(reopened.node(a).is_some());
}

#[test]
fn effect_and_text_styles_apply_their_settings() {
    let mut f = Fx::new();
    let a = f.rect();
    let fx = f
        .run(vec![EditOp::CreateStyle {
            kind: StyleKind::Effect,
            name: "Shadow".into(),
            from_node: None,
            paints: None,
            effects: Some(vec![EffectSpec::LayerBlur {
                radius: 3.0,
                visible: None,
            }]),
        }])
        .unwrap()[0];
    f.json(serde_json::json!({ "op": "bind_style", "target": a.to_string(), "slot": "effect", "style": fx.to_string() }))
        .unwrap();
    assert_eq!(f.data(a).get("effects").unwrap().items().len(), 1);

    let page = f.page;
    let text = |y: f64| EditOp::CreateText {
        id: None,
        parent: page,
        x: 0.0,
        y,
        characters: "Text".into(),
        font_family: "Inter".into(),
        font_style: "Regular".into(),
        font_size: 12.0,
        fill_hex: "#000000".into(),
    };
    let texts = f.run(vec![text(100.0), text(200.0)]).unwrap();
    f.json(serde_json::json!({ "op": "set_typography", "target": texts[0].to_string(), "font_size": 31.0 }))
        .unwrap();
    let ts = f
        .run(vec![EditOp::CreateStyle {
            kind: StyleKind::Text,
            name: "Heading".into(),
            from_node: Some(texts[0]),
            paints: None,
            effects: None,
        }])
        .unwrap()[0];
    f.json(serde_json::json!({ "op": "bind_style", "target": texts[1].to_string(), "slot": "text", "style": ts.to_string() }))
        .unwrap();
    assert_eq!(f.data(texts[1]).num("fontSize"), Some(31.0));
    // A text style needs a text layer; an empty name is refused.
    assert!(
        f.run(vec![EditOp::CreateStyle {
            kind: StyleKind::Text,
            name: "X".into(),
            from_node: Some(a),
            paints: None,
            effects: None,
        }])
        .is_err()
    );
    assert!(
        f.run(vec![EditOp::CreateStyle {
            kind: StyleKind::Paint,
            name: " ".into(),
            from_node: Some(a),
            paints: None,
            effects: None,
        }])
        .is_err()
    );
}

#[test]
fn variables_bind_to_fields_follow_modes_and_survive_deletion() {
    let mut f = Fx::new();
    let a = f.rect();
    let ids = f
        .run(vec![EditOp::CreateVariableCollection {
            name: "Tokens".into(),
            modes: vec!["Light".into(), "Dark".into()],
        }])
        .unwrap();
    let (set, light, dark) = (ids[0], ids[1], ids[2]);
    let mk = |name: &str, kind: VarType, value: VarValue| EditOp::CreateVariable {
        collection: set,
        name: name.into(),
        kind,
        value,
    };
    let v = f
        .run(vec![
            mk("radius", VarType::Number, VarValue::Number(4.0)),
            mk("visible", VarType::Bool, VarValue::Bool(true)),
            mk("label", VarType::String, VarValue::String("Hi".into())),
            mk("brand", VarType::Color, VarValue::Color("#336699".into())),
        ])
        .unwrap();
    let (radius, visible) = (v[0], v[1]);
    let alias = f
        .run(vec![mk(
            "radius-alias",
            VarType::Number,
            VarValue::Alias(radius),
        )])
        .unwrap()[0];
    f.run(vec![
        EditOp::BindVariable {
            target: a,
            field: BindField::CornerRadius,
            variable: Some(alias),
        },
        EditOp::BindVariable {
            target: a,
            field: BindField::Visible,
            variable: Some(visible),
        },
    ])
    .unwrap();
    assert_eq!(f.data(a).num("cornerRadius"), Some(4.0));

    // A value change rewrites bound layers; a mode switch picks the mode's value.
    f.run(vec![
        EditOp::SetVariableValue {
            variable: radius,
            mode: light,
            value: VarValue::Number(8.0),
        },
        EditOp::SetVariableValue {
            variable: radius,
            mode: dark,
            value: VarValue::Number(20.0),
        },
    ])
    .unwrap();
    assert_eq!(f.data(a).num("cornerRadius"), Some(8.0));
    f.json(
        serde_json::json!({ "op": "set_variable_mode", "container": a.to_string(),
                               "collection": set.to_string(), "mode": dark.to_string() }),
    )
    .unwrap();
    assert_eq!(f.data(a).num("cornerRadius"), Some(20.0));

    // Type checks: a bool variable on a number field, a string value in a number variable.
    assert!(
        f.run(vec![EditOp::BindVariable {
            target: a,
            field: BindField::Opacity,
            variable: Some(visible),
        }])
        .is_err()
    );
    assert!(
        f.run(vec![EditOp::SetVariableValue {
            variable: radius,
            mode: light,
            value: VarValue::String("x".into()),
        }])
        .is_err()
    );

    // Modes: add copies the first mode's values; the last mode cannot be removed.
    let m3 = f
        .run(vec![EditOp::AddVariableMode {
            collection: set,
            name: "Contrast".into(),
        }])
        .unwrap()[0];
    let entries = |f: &Fx, id: Guid| {
        f.data(id)
            .path(&["variableDataValues", "entries"])
            .unwrap()
            .items()
            .len()
    };
    assert_eq!(entries(&f, radius), 3);
    f.run(vec![
        EditOp::RemoveVariableMode {
            collection: set,
            mode: m3,
        },
        EditOp::RemoveVariableMode {
            collection: set,
            mode: dark,
        },
    ])
    .unwrap();
    assert!(
        f.run(vec![EditOp::RemoveVariableMode {
            collection: set,
            mode: light,
        }])
        .is_err()
    );

    // Deleting the aliased variable keeps the alias's value and the layer's radius.
    f.run(vec![EditOp::DeleteVariable { variable: radius }])
        .unwrap();
    let alias_value = f
        .data(alias)
        .path(&["variableDataValues", "entries"])
        .unwrap()
        .items()[0]
        .path(&["variableData", "value", "floatValue"])
        .and_then(Val::as_f32);
    assert_eq!(alias_value, Some(8.0));
    assert_eq!(f.data(a).num("cornerRadius"), Some(8.0));
    let reopened = Document::from_bytes(&f.doc.to_bytes().unwrap()).unwrap();
    assert!(reopened.node(alias).is_some());
}
