//! Components and assets on the real app: the Assets tab places components, applies styles
//! and edits variables; the Design panel sets instance properties and variants, defines a
//! component's properties and binds a layer to one.
use figstudio_fig::edit::instances::ComponentPropVal;
use figstudio_fig::edit::{ComponentPropKind, EditOp};

use super::design::type_into;
use super::*;

fn ops(h: &mut Harness<'_, App>, ops: &[EditOp]) -> Vec<Guid> {
    let ids = crate::editor::apply_tab_ops(&mut h.state_mut().tabs[0], ops, None, None).unwrap();
    h.run_steps(2);
    ids
}

fn click(h: &mut Harness<'_, App>, label: &str) {
    reveal(h, label);
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn show_assets(h: &mut Harness<'_, App>) {
    h.get_by_label("Assets tab").click();
    h.run_steps(3);
}

#[test]
fn the_assets_tab_places_a_component_and_applies_a_style() {
    let mut h = harness();
    let style = ops(
        &mut h,
        &[EditOp::CreateFillStyle {
            name: "Brand".into(),
            hex: "#FF0000".into(),
        }],
    )[0];
    show_assets(&mut h);
    click(&mut h, "Place Component 1");
    let placed = h.state().tabs[0]
        .selection
        .expect("the new instance is selected");
    assert_eq!(h.state().tabs[0].doc.node(placed).unwrap().kind, "INSTANCE");
    // A fill style applies to the selection.
    click(&mut h, "Apply Brand");
    let bound = h.state().tabs[0]
        .doc
        .node(placed)
        .unwrap()
        .data
        .get("styleIdForFill")
        .map(|v| v.clone().into_owned());
    assert!(bound.is_some(), "the fill style is bound");
    let _ = style;
}

#[test]
fn the_variables_table_adds_a_collection_a_variable_and_edits_a_value() {
    let mut h = harness();
    show_assets(&mut h);
    click(&mut h, "+ Collection");
    click(&mut h, "Add Number variable to Collection 1");
    type_into(&mut h, "number 1 in Mode 1", "12");
    let doc = &h.state().tabs[0].doc;
    let var = doc
        .nodes
        .iter()
        .find(|n| n.kind == "VARIABLE" && n.name == "number 1")
        .unwrap();
    let v = var
        .data
        .path(&["variableDataValues", "entries"])
        .unwrap()
        .items()[0]
        .path(&["variableData", "value", "floatValue"])
        .and_then(figstudio_fig::Val::as_f32);
    assert_eq!(v, Some(12.0));
}

/// A variant set Size=Small/Large with a boolean `Icon`, and an instance of Small, selected.
fn button_set(h: &mut Harness<'_, App>) -> (Guid, Guid, Guid) {
    let page = h.state().tabs[0].page;
    let frame = |name: &str, x: f64| EditOp::CreateShape {
        id: None,
        kind: "FRAME".into(),
        parent: page,
        name: Some(name.into()),
        x,
        y: 0.0,
        width: 80.0,
        height: 30.0,
        corner_radius: 0.0,
        fill_hex: Some("#3366FF".into()),
        count: None,
        star_inner_scale: None,
    };
    let f = ops(h, &[frame("Size=Small", 0.0), frame("Size=Large", 120.0)]);
    ops(
        h,
        &[
            EditOp::CreateComponent { frame: f[0] },
            EditOp::CreateComponent { frame: f[1] },
        ],
    );
    let set = ops(
        h,
        &[EditOp::CombineAsVariants {
            components: f.clone(),
            name: Some("Button".into()),
        }],
    )[0];
    ops(
        h,
        &[EditOp::AddComponentProp {
            component: set,
            name: "Icon".into(),
            kind: ComponentPropKind::Bool,
            default: ComponentPropVal::Bool(true),
        }],
    );
    let inst = ops(
        h,
        &[EditOp::InstantiateComponent {
            symbol: f[0],
            parent: page,
            x: 0.0,
            y: 100.0,
        }],
    )[0];
    h.state_mut().tabs[0].select(Some(inst));
    h.run_steps(3);
    (f[0], f[1], inst)
}

#[test]
fn the_instance_section_switches_variants_and_sets_properties() {
    let mut h = harness_fixture("shaders.fig", 0.25);
    let (_, large, inst) = button_set(&mut h);
    click(&mut h, "Size value");
    click(&mut h, "Large");
    let symbol = |h: &Harness<'_, App>| {
        figstudio_fig::resolve::symbol_id(&h.state().tabs[0].doc.node(inst).unwrap().data)
    };
    assert_eq!(symbol(&h), Some(large));
    click(&mut h, "Icon value");
    let props = figstudio_fig::edit::component_props::instance_props(&h.state().tabs[0].doc, inst);
    let icon = props.iter().find(|p| p.name == "Icon").unwrap();
    assert_eq!(
        icon.value,
        figstudio_fig::edit::component_props::PropViewValue::Bool(false)
    );
}

#[test]
fn a_component_defines_a_property_and_a_layer_inside_binds_to_it() {
    let mut h = harness();
    let page = h.state().tabs[0].page;
    let frame = ops(
        &mut h,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: Some("Card".into()),
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 60.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    let dot = ops(
        &mut h,
        &[EditOp::CreateShape {
            id: None,
            kind: "ELLIPSE".into(),
            parent: frame,
            name: Some("Dot".into()),
            x: 10.0,
            y: 10.0,
            width: 10.0,
            height: 10.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
    )[0];
    ops(&mut h, &[EditOp::CreateComponent { frame }]);
    h.state_mut().tabs[0].select(Some(frame));
    h.run_steps(3);
    click(&mut h, "Add Boolean property");
    type_into(&mut h, "Boolean 1 name", "Show dot");
    let props =
        figstudio_fig::edit::component_props::component_props(&h.state().tabs[0].doc, frame);
    assert_eq!(props.len(), 1);
    assert_eq!(props[0].name, "Show dot");
    // The dot's visibility follows the property.
    h.state_mut().tabs[0].select(Some(dot));
    h.run_steps(3);
    click(&mut h, "Visible property");
    click(&mut h, "Show dot");
    let refs = h.state().tabs[0]
        .doc
        .node(dot)
        .unwrap()
        .data
        .get("componentPropRefs")
        .map(|v| v.items().len());
    assert_eq!(refs, Some(1));
}
