mod common;
use common::*;
use figstudio_fig::Val;
use figstudio_render::{Renderer, Viewport};
use vello_cpu::kurbo::Affine;

fn alias(reference: Val) -> Val {
    obj(&[("value", obj(&[("alias", reference)]))])
}
fn asset() -> Val {
    obj(&[(
        "assetRef",
        obj(&[
            ("key", Val::Str("shared".into())),
            ("version", Val::Str("v1".into())),
        ]),
    )])
}
fn variable(n: u32, values: Vec<Val>) -> figstudio_fig::Node {
    let mut v = node(n, 0, "VARIABLE");
    v.parent = None;
    v.data.set(
        "variableDataValues",
        obj(&[(
            "entries",
            Val::Arr(
                values
                    .into_iter()
                    .map(|v| obj(&[("variableData", v)]))
                    .collect(),
            ),
        )]),
    );
    v
}
fn rectangle(n: u32, x: f64, binding: Val) -> figstudio_fig::Node {
    let mut r = node(n, 1, "RECTANGLE");
    r.data.set("size", size(20.0, 20.0));
    r.data.set("transform", obj(&[("m02", Val::Float(x))]));
    let mut paints = paint(1.0, 0.0, 0.0);
    if let Val::Arr(p) = &mut paints {
        p[0].set("colorVar", binding);
    }
    r.data.set("fillPaints", paints);
    r
}
#[test]
fn first_mode_depth_ceiling_and_duplicate_empty_assets_match_legacy() {
    let green = obj(&[(
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
    )]);
    let blue = obj(&[(
        "value",
        obj(&[(
            "colorValue",
            obj(&[
                ("r", Val::Float(0.0)),
                ("g", Val::Float(0.0)),
                ("b", Val::Float(1.0)),
                ("a", Val::Float(1.0)),
            ]),
        )]),
    )]);
    let mut target = variable(100, vec![green, blue]);
    target
        .data
        .set("variableSetID", obj(&[("guid", id(300).to_val())]));
    if let Some(Val::Arr(entries)) = target
        .data
        .get_mut("variableDataValues")
        .and_then(|v| v.get_mut("entries"))
    {
        entries[0].set("modeID", id(301).to_val());
        entries[1].set("modeID", id(302).to_val());
    }
    let mut collection = node(300, 0, "VARIABLE_SET");
    collection.parent = None;
    collection.data.set(
        "variableSetModes",
        Val::Arr(vec![
            obj(&[
                ("id", id(301).to_val()),
                ("sortPosition", Val::Str("b".into())),
            ]),
            obj(&[
                ("id", id(302).to_val()),
                ("sortPosition", Val::Str("a".into())),
            ]),
        ]),
    );
    target.data.set("key", Val::Str("shared".into()));
    target.data.set("version", Val::Str("v1".into()));
    let mut empty = variable(101, vec![]);
    empty.data.set("key", Val::Str("shared".into()));
    empty.data.set("version", Val::Str("v1".into()));
    let mut nodes = vec![
        node(1, 0, "CANVAS"),
        rectangle(2, 0.0, alias(asset())),
        rectangle(3, 30.0, alias(obj(&[("guid", id(200).to_val())]))),
        rectangle(4, 60.0, alias(obj(&[("guid", id(999).to_val())]))),
        target,
        empty,
        collection,
    ];
    for n in 200..224 {
        nodes.push(variable(
            n,
            vec![alias(obj(&[(
                "guid",
                id(if n == 223 { 100 } else { n + 1 }).to_val(),
            )]))],
        ));
    }
    let doc = document(nodes);
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    let output = renderer.render(
        &doc,
        id(1),
        &Viewport {
            width: 80,
            height: 80,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    let px = output.pixmap.data_as_u8_slice();
    assert_eq!(pixel(px, 10, 10), &[0, 255, 0, 255]);
    assert_eq!(pixel(px, 40, 10), &[255, 0, 0, 255]);
    assert_eq!(pixel(px, 70, 10), &[255, 0, 0, 255]);
    assert_eq!(output.stats.approximated_paints, 2);
}
