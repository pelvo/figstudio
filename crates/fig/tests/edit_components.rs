//! Defining components: properties, bindings, variant sets, generic instance overrides.

use std::path::PathBuf;

use figstudio_fig::edit::instances::ComponentPropVal;
use figstudio_fig::edit::{ComponentPropKind, EditOp, History, execute_edit_ops};
use figstudio_fig::resolve::resolve_instance;
use figstudio_fig::{Document, Guid, Val};

struct Fx {
    doc: Document,
    h: History,
    page: Guid,
}

impl Fx {
    fn new() -> Fx {
        Fx::open("structure")
    }

    fn open(name: &str) -> Fx {
        let doc = Document::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures")
                .join(format!("{name}.fig")),
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

    fn frame(&mut self, name: &str, x: f64) -> Guid {
        let page = self.page;
        self.run(vec![EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some(name.into()),
            x,
            y: 0.0,
            width: 100.0,
            height: 40.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }])
        .unwrap()[0]
    }

    /// A component holding a text layer and a rectangle; returns (component, text, rect).
    fn button(&mut self, name: &str, x: f64) -> (Guid, Guid, Guid) {
        let frame = self.frame(name, x);
        let kids = self
            .run(vec![
                EditOp::CreateText {
                    id: None,
                    parent: frame,
                    x: 4.0,
                    y: 4.0,
                    characters: "Label".into(),
                    font_family: "Inter".into(),
                    font_style: "Regular".into(),
                    font_size: 12.0,
                    fill_hex: "#111111".into(),
                },
                EditOp::CreateShape {
                    id: None,
                    kind: "RECTANGLE".into(),
                    parent: frame,
                    name: Some("Icon".into()),
                    x: 80.0,
                    y: 10.0,
                    width: 10.0,
                    height: 10.0,
                    corner_radius: 0.0,
                    fill_hex: None,
                    count: None,
                    star_inner_scale: None,
                },
                EditOp::CreateComponent { frame },
            ])
            .unwrap();
        (frame, kids[0], kids[1])
    }

    fn instance(&mut self, symbol: Guid) -> Guid {
        let page = self.page;
        self.run(vec![EditOp::InstantiateComponent {
            symbol,
            parent: page,
            x: 0.0,
            y: 200.0,
        }])
        .unwrap()[0]
    }

    /// The resolved data of the instance's child drawn from `source`.
    fn resolved(&self, instance: Guid, source: Guid) -> Val {
        resolve_instance(&self.doc, instance)
            .unwrap()
            .nodes
            .into_iter()
            .find(|n| n.node == source)
            .unwrap()
            .data
    }
}

#[test]
fn bool_and_text_properties_drive_bound_layers_in_instances() {
    let mut f = Fx::new();
    let (button, text, icon) = f.button("Button", 0.0);
    let ids = f
        .run(vec![
            EditOp::AddComponentProp {
                component: button,
                name: "Show icon".into(),
                kind: ComponentPropKind::Bool,
                default: ComponentPropVal::Bool(true),
            },
            EditOp::AddComponentProp {
                component: button,
                name: "Label".into(),
                kind: ComponentPropKind::Text,
                default: ComponentPropVal::Text("Click".into()),
            },
            EditOp::BindComponentProp {
                node: icon,
                prop: "Show icon".into(),
            },
            EditOp::BindComponentProp {
                node: text,
                prop: "Label".into(),
            },
        ])
        .unwrap();
    assert_eq!(ids.len(), 2, "both property ids are reported");
    let inst = f.instance(button);
    let chars = |v: &Val| {
        v.path(&["textData", "characters"])
            .and_then(Val::as_str)
            .map(str::to_owned)
    };
    assert_eq!(chars(&f.resolved(inst, text)).as_deref(), Some("Click"));

    f.run(vec![
        EditOp::SetComponentProp {
            instance: inst,
            prop: "Show icon".into(),
            val: ComponentPropVal::Bool(false),
        },
        EditOp::SetComponentProp {
            instance: inst,
            prop: "Label".into(),
            val: ComponentPropVal::Text("Buy".into()),
        },
    ])
    .unwrap();
    assert_eq!(f.resolved(inst, icon).flag("visible"), Some(false));
    assert_eq!(chars(&f.resolved(inst, text)).as_deref(), Some("Buy"));

    // Renaming keeps the binding; a duplicate name or a wrong-kind default is refused.
    f.run(vec![EditOp::EditComponentProp {
        component: button,
        prop: "Label".into(),
        name: Some("Text".into()),
        default: Some(ComponentPropVal::Text("Go".into())),
    }])
    .unwrap();
    assert_eq!(chars(&f.resolved(inst, text)).as_deref(), Some("Buy"));
    assert!(
        f.run(vec![EditOp::EditComponentProp {
            component: button,
            prop: "Text".into(),
            name: Some("Show icon".into()),
            default: None,
        }])
        .is_err()
    );
    assert!(
        f.run(vec![EditOp::EditComponentProp {
            component: button,
            prop: "Text".into(),
            name: None,
            default: Some(ComponentPropVal::Bool(true)),
        }])
        .is_err()
    );
    // A text property cannot bind to a rectangle.
    assert!(
        f.run(vec![EditOp::BindComponentProp {
            node: icon,
            prop: "Text".into(),
        }])
        .is_err()
    );

    // Removing a property drops its binding and the instance's assignment.
    f.run(vec![EditOp::RemoveComponentProp {
        component: button,
        prop: "Show icon".into(),
    }])
    .unwrap();
    assert!(
        f.doc
            .node(icon)
            .unwrap()
            .data
            .get("componentPropRefs")
            .is_none_or(|r| r.items().is_empty())
    );
    assert_eq!(
        f.doc
            .node(inst)
            .unwrap()
            .data
            .get("componentPropAssignments")
            .unwrap()
            .items()
            .len(),
        1
    );
    assert_ne!(f.resolved(inst, icon).flag("visible"), Some(false));

    f.run(vec![EditOp::UnbindComponentProp {
        node: text,
        kind: ComponentPropKind::Text,
    }])
    .unwrap();
    assert_eq!(chars(&f.resolved(inst, text)).as_deref(), Some("Label"));
    let reopened = Document::from_bytes(&f.doc.to_bytes().unwrap()).unwrap();
    assert_eq!(
        reopened
            .node(button)
            .unwrap()
            .data
            .get("componentPropDefs")
            .unwrap()
            .items()
            .len(),
        1
    );
}

#[test]
fn combine_as_variants_builds_a_set_that_switch_variant_uses() {
    // A file whose schema stores variant properties.
    let mut f = Fx::open("shaders");
    let (small, _, _) = f.button("Size=Small, State=Default", 0.0);
    let (large, _, _) = f.button("Size=Large, State=Default", 150.0);
    let set = f
        .run(vec![EditOp::CombineAsVariants {
            components: vec![small, large],
            name: Some("Button".into()),
        }])
        .unwrap()[0];
    let s = f.doc.node(set).unwrap();
    assert_eq!(s.data.flag("isStateGroup"), Some(true));
    let defs = s.data.get("componentPropDefs").unwrap().items();
    let names: Vec<_> = defs.iter().map(|d| d.text("name").unwrap()).collect();
    assert_eq!(names, ["Size", "State"]);
    assert!(defs.iter().all(|d| d.text("type") == Some("VARIANT")));
    assert_eq!(f.doc.children(set), &[small, large]);
    let specs = f
        .doc
        .node(large)
        .unwrap()
        .data
        .get("variantPropSpecs")
        .unwrap()
        .items();
    assert_eq!(specs[0].text("value"), Some("Large"));

    let inst = f.instance(small);
    f.run(vec![EditOp::SwitchVariant {
        instance: inst,
        variants: vec![("Size".into(), "Large".into())],
    }])
    .unwrap();
    let over = f
        .doc
        .node(inst)
        .unwrap()
        .data
        .get("overriddenSymbolID")
        .and_then(Val::as_guid);
    assert_eq!(over, Some(large));

    // Adding a variant copies one with a fresh value and grows the set to show it.
    let w0 = f
        .doc
        .node(set)
        .unwrap()
        .data
        .path(&["size", "x"])
        .and_then(Val::as_f32)
        .unwrap();
    let added = f.run(vec![EditOp::AddVariant { from: large }]).unwrap()[0];
    let n = f.doc.node(added).unwrap();
    assert_eq!(n.name, "Size=Large 2, State=Default");
    assert_eq!(n.kind, "SYMBOL");
    assert!(
        f.doc
            .node(set)
            .unwrap()
            .data
            .path(&["size", "x"])
            .and_then(Val::as_f32)
            .unwrap()
            > w0
    );

    // Plain names become `Property 1`; a component already in a set is refused. An older
    // file (no variant property schema) gets the names alone.
    let mut f = Fx::new();
    let (small, _, _) = f.button("Size=Small", 0.0);
    let (a, _, _) = f.button("Primary", 400.0);
    let (b, _, _) = f.button("Primary", 550.0);
    let set2 = f
        .run(vec![EditOp::CombineAsVariants {
            components: vec![a, b],
            name: None,
        }])
        .unwrap()[0];
    let names: Vec<String> = f
        .doc
        .children(set2)
        .iter()
        .map(|c| f.doc.node(*c).unwrap().name.clone())
        .collect();
    assert_eq!(names, ["Property 1=Primary", "Property 1=Primary 2"]);
    assert!(
        f.doc
            .node(set2)
            .unwrap()
            .data
            .get("componentPropDefs")
            .is_none()
    );
    f.run(vec![EditOp::CombineAsVariants {
        components: vec![small],
        name: None,
    }])
    .unwrap();
    assert!(
        f.run(vec![EditOp::CombineAsVariants {
            components: vec![small],
            name: None,
        }])
        .is_err()
    );
}

#[test]
fn generic_instance_overrides_are_schema_checked_and_refuse_structure() {
    let mut f = Fx::new();
    let (button, _, icon) = f.button("Button", 0.0);
    let inst = f.instance(button);
    let key = f.doc.node(icon).unwrap().override_key();
    let fields = |json: serde_json::Value| json.as_object().cloned();
    let raw = serde_json::to_value(Val::Float(6.0)).unwrap();
    f.run(vec![EditOp::SetInstanceOverride {
        instance: inst,
        override_path: vec![key],
        characters: None,
        visible: None,
        fill_hex: None,
        opacity: None,
        fields: fields(serde_json::json!({ "cornerRadius": raw })),
    }])
    .unwrap();
    assert_eq!(f.resolved(inst, icon).num("cornerRadius"), Some(6.0));
    for bad in [
        // A valid matrix, so only the structural-field rule refuses it.
        serde_json::json!({ "transform": serde_json::to_value(
            figstudio_fig::edit::space::Xform::IDENTITY.to_val()
        ).unwrap() }),
        serde_json::json!({ "notAField": serde_json::to_value(Val::Float(1.0)).unwrap() }),
        serde_json::json!({ "cornerRadius": serde_json::to_value(Val::Str("x".into())).unwrap() }),
    ] {
        assert!(
            f.run(vec![EditOp::SetInstanceOverride {
                instance: inst,
                override_path: vec![key],
                characters: None,
                visible: None,
                fill_hex: None,
                opacity: None,
                fields: fields(bad.clone()),
            }])
            .is_err(),
            "{bad}"
        );
    }
}

#[test]
fn panels_read_component_and_instance_properties_with_variant_options() {
    use figstudio_fig::edit::component_props::{PropViewValue, component_props, instance_props};
    let mut f = Fx::open("shaders");
    let (small, _, _) = f.button("Size=Small", 0.0);
    let (large, _, _) = f.button("Size=Large", 150.0);
    let set = f
        .run(vec![EditOp::CombineAsVariants {
            components: vec![small, large],
            name: Some("Button".into()),
        }])
        .unwrap()[0];
    f.run(vec![EditOp::AddComponentProp {
        component: set,
        name: "Icon".into(),
        kind: ComponentPropKind::Bool,
        default: ComponentPropVal::Bool(true),
    }])
    .unwrap();
    let props = component_props(&f.doc, large);
    let names: Vec<_> = props.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Size", "Icon"]);
    assert_eq!(
        props[0].value,
        PropViewValue::Variant {
            value: "Large".into(),
            options: vec!["Small".into(), "Large".into()]
        }
    );
    assert_eq!(props[1].value, PropViewValue::Bool(true));

    // An instance shows its own values: the variant it shows and its assignments.
    let inst = f.instance(small);
    f.run(vec![
        EditOp::SwitchVariant {
            instance: inst,
            variants: vec![("Size".into(), "Large".into())],
        },
        EditOp::SetComponentProp {
            instance: inst,
            prop: "Icon".into(),
            val: ComponentPropVal::Bool(false),
        },
    ])
    .unwrap();
    let props = instance_props(&f.doc, inst);
    assert!(matches!(&props[0].value, PropViewValue::Variant { value, .. } if value == "Large"));
    assert_eq!(props[1].value, PropViewValue::Bool(false));
    // Not an instance: nothing.
    assert!(instance_props(&f.doc, set).is_empty());
}
