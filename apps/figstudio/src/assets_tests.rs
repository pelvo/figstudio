//! The Assets index (components, styles, variables by mode) and placing a component.

use std::sync::Arc;
use std::time::Duration;

use figstudio_fig::Document;

use super::*;

fn tab() -> Tab {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    Tab::new(
        1,
        "t.fig".into(),
        Arc::new(Document::open(path).unwrap()),
        None,
        Duration::ZERO,
    )
}

#[test]
fn the_index_lists_components_styles_and_variables_by_mode() {
    let mut tab = tab();
    let made = apply_tab_ops(
        &mut tab,
        &[
            EditOp::CreateFillStyle {
                name: "Brand".into(),
                hex: "#FF0000".into(),
            },
            EditOp::CreateVariableCollection {
                name: "Theme".into(),
                modes: vec!["Light".into(), "Dark".into()],
            },
        ],
        None,
        None,
    )
    .unwrap();
    let (collection, dark) = (made[1], made[3]);
    let var = apply_tab_ops(
        &mut tab,
        &[EditOp::CreateVariable {
            collection,
            name: "gap".into(),
            kind: VarType::Number,
            value: VarValue::Number(4.0),
        }],
        None,
        None,
    )
    .unwrap()[0];
    apply_tab_ops(
        &mut tab,
        &[EditOp::SetVariableValue {
            variable: var,
            mode: dark,
            value: VarValue::Number(8.0),
        }],
        None,
        None,
    )
    .unwrap();
    let i = build(&tab, tab.doc.instance, tab.doc.revision);
    assert!(
        i.components.iter().any(|(_, l)| l == "Component 1"),
        "{:?}",
        i.components
    );
    let brand = i
        .styles
        .iter()
        .find(|s| s.name == "Brand")
        .expect("the style");
    assert_eq!(brand.slot, "fill");
    assert_eq!(brand.swatch, Some(Color32::from_rgb(255, 0, 0)));
    let theme = i.collections.iter().find(|c| c.name == "Theme").unwrap();
    let modes: Vec<&str> = theme.modes.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(modes, ["Light", "Dark"]);
    let gap = &theme.variables[0];
    let nums: Vec<Option<f32>> = gap
        .values
        .iter()
        .map(|v| {
            v.as_ref()
                .and_then(|v| v.get("floatValue"))
                .and_then(Val::as_f32)
        })
        .collect();
    assert_eq!(nums, [Some(4.0), Some(8.0)]);
}

#[test]
fn a_placed_component_is_centred_on_the_point_and_selected() {
    let mut tab = tab();
    let symbol = Guid(1, 37);
    place(&mut tab, symbol, Pos2::new(500.0, 300.0));
    let id = tab.selection.expect("selected");
    let n = tab.doc.node(id).unwrap();
    assert_eq!(n.kind, "INSTANCE");
    assert_eq!(n.parent, Some(tab.page));
    let b = figstudio_render::node_bounds(&tab.doc, id).unwrap();
    assert!(
        (b.center().x - 500.0).abs() <= 0.5 && (b.center().y - 300.0).abs() <= 0.5,
        "{b:?}"
    );
}
