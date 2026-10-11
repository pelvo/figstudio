//! Instance swaps and partial property overrides (KNOWN-GAPS A9, A10, A12).
mod common;
use common::*;
use figstudio_fig::Val;

fn assignment(def: u32, value: bool) -> Val {
    obj(&[
        ("defID", id(def).to_val()),
        ("value", obj(&[("boolValue", Val::Bool(value))])),
    ])
}

/// Component 2 holds a placeholder instance (3, of component 4) that an override swaps to
/// component 6. Component 4 has a red fill and a red stroke; component 6 has no paints.
fn swap_doc(own: impl FnOnce(&mut figstudio_fig::Node)) -> Vec<u8> {
    let mut placeholder = instance(3, 2, 4, vec![]);
    let mut old = node(4, 0, "SYMBOL");
    for n in [&mut placeholder, &mut old] {
        n.data.set("fillPaints", paint(1.0, 0.0, 0.0));
        n.data.set("strokePaints", paint(1.0, 0.0, 0.0));
        n.data.set("strokeWeight", Val::Float(4.0));
    }
    own(&mut placeholder);
    let swap = patch(&[3], &[("overriddenSymbolID", id(6).to_val())]);
    pixels(&document(vec![
        node(1, 0, "CANVAS"),
        node(2, 0, "SYMBOL"),
        placeholder,
        old,
        node(6, 0, "SYMBOL"),
        instance(10, 1, 2, vec![swap]),
    ]))
}

#[test]
fn a_swapped_instance_takes_paints_from_its_new_component() {
    let px = swap_doc(|_| {});

    assert_eq!(pixel(&px, 20, 20), &[0, 0, 0, 0], "no stale fill");
    assert_eq!(pixel(&px, 0, 20), &[0, 0, 0, 0], "no stale stroke");
}

#[test]
fn a_swapped_instance_keeps_paints_it_overrode_itself() {
    let px = swap_doc(|n| n.data.set("fillPaints", paint(0.0, 1.0, 0.0)));

    assert_eq!(pixel(&px, 20, 20), &[0, 255, 0, 255]);
}

#[test]
fn an_override_setting_one_property_keeps_the_instances_other_assignments() {
    // Component 4 has two layers bound to booleans 90 (left) and 91 (right), both true by
    // default. The nested instance 3 turns both off; an override then sets only 91 again.
    let mut defs = node(4, 0, "SYMBOL");
    defs.data.set(
        "componentPropDefs",
        Val::Arr(
            [90, 91]
                .map(|d| {
                    obj(&[
                        ("id", id(d).to_val()),
                        ("initialValue", obj(&[("boolValue", Val::Bool(true))])),
                    ])
                })
                .to_vec(),
        ),
    );
    let mut layers = Vec::new();
    for (n, def, x) in [(5, 90, 0.0), (7, 91, 40.0)] {
        let mut r = node(n, 4, "RECTANGLE");
        r.data.set("fillPaints", paint(1.0, 0.0, 0.0));
        r.data.set("transform", obj(&[("m02", Val::Float(x))]));
        r.data.set(
            "componentPropRefs",
            Val::Arr(vec![obj(&[
                ("defID", id(def).to_val()),
                ("componentPropNodeField", Val::Str("VISIBLE".into())),
            ])]),
        );
        layers.push(r);
    }
    let mut nested = instance(3, 2, 4, vec![]);
    nested.data.set(
        "componentPropAssignments",
        Val::Arr(vec![assignment(90, false), assignment(91, false)]),
    );
    let set_one = patch(
        &[3],
        &[(
            "componentPropAssignments",
            Val::Arr(vec![assignment(91, false)]),
        )],
    );
    let mut nodes = vec![node(1, 0, "CANVAS"), node(2, 0, "SYMBOL"), nested, defs];
    nodes.extend(layers);
    nodes.push(instance(10, 1, 2, vec![set_one]));

    let px = pixels(&document(nodes));

    assert_eq!(pixel(&px, 20, 20), &[0, 0, 0, 0], "90 stays off");
    assert_eq!(pixel(&px, 60, 20), &[0, 0, 0, 0], "91 off");
}
