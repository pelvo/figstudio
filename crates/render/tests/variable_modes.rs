//! A variable mode set on a page or a frame reaches everything below it, including a layer
//! drawn on its own (a selection, an export) and a layer inside an instance.
mod common;
use common::*;
use figstudio_fig::resolve::InstancePath;
use figstudio_fig::{Document, Node, Val};
use figstudio_render::{Renderer, Viewport};
use vello_cpu::kurbo::Affine;

const SET: u32 = 300;
const VARIABLE: u32 = 100;
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];

fn colour(r: f64, g: f64, b: f64) -> Val {
    obj(&[(
        "value",
        obj(&[(
            "colorValue",
            obj(&[
                ("r", Val::Float(r)),
                ("g", Val::Float(g)),
                ("b", Val::Float(b)),
                ("a", Val::Float(1.0)),
            ]),
        )]),
    )])
}

/// Mode 301 (the collection's first, so its default) is green, 302 blue and 303 red.
fn tokens() -> Vec<Node> {
    let mut variable = node(VARIABLE, 0, "VARIABLE");
    variable.parent = None;
    variable
        .data
        .set("variableSetID", obj(&[("guid", id(SET).to_val())]));
    let values = [
        (301, colour(0.0, 1.0, 0.0)),
        (302, colour(0.0, 0.0, 1.0)),
        (303, colour(1.0, 0.0, 0.0)),
    ]
    .into_iter()
    .map(|(mode, value)| {
        let mut entry = obj(&[("variableData", value)]);
        entry.set("modeID", id(mode).to_val());
        entry
    })
    .collect();
    variable
        .data
        .set("variableDataValues", obj(&[("entries", Val::Arr(values))]));
    let mut set = node(SET, 0, "VARIABLE_SET");
    set.parent = None;
    set.data.set(
        "variableSetModes",
        Val::Arr(
            [301, 302, 303]
                .into_iter()
                .enumerate()
                .map(|(i, mode)| {
                    obj(&[
                        ("id", id(mode).to_val()),
                        ("sortPosition", Val::Str(format!("{i}"))),
                    ])
                })
                .collect(),
        ),
    );
    vec![variable, set]
}

/// `node` explicitly in `mode` of the collection.
fn in_mode(mut node: Node, mode: u32) -> Node {
    let entry = obj(&[
        ("variableSetID", obj(&[("guid", id(SET).to_val())])),
        ("variableModeID", id(mode).to_val()),
    ]);
    node.data.set(
        "variableModeBySetMap",
        obj(&[("entries", Val::Arr(vec![entry]))]),
    );
    node
}

/// A 20 x 20 rectangle at the origin of `parent`, filled by the variable.
fn bound_rectangle(n: u32, parent: u32) -> Node {
    let mut r = node(n, parent, "RECTANGLE");
    r.data.set("size", size(20.0, 20.0));
    let mut paints = paint(1.0, 1.0, 1.0);
    if let Val::Arr(p) = &mut paints {
        let alias = obj(&[("guid", id(VARIABLE).to_val())]);
        p[0].set("colorVar", obj(&[("value", obj(&[("alias", alias)]))]));
    }
    r.data.set("fillPaints", paints);
    r
}

fn viewport() -> Viewport {
    Viewport {
        width: 80,
        height: 80,
        transform: Affine::IDENTITY,
        background: None,
    }
}

fn top_left(frame: figstudio_render::Frame) -> [u8; 4] {
    let px = frame.pixmap.data_as_u8_slice();
    pixel(px, 10, 10).try_into().unwrap()
}

fn drawn(doc: &Document, root: u32) -> [u8; 4] {
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    top_left(renderer.render(doc, id(root), &viewport()))
}

fn build(nodes: Vec<Node>) -> Document {
    document([nodes, tokens()].concat())
}

#[test]
fn a_mode_set_on_a_page_applies_to_its_layers() {
    let page = |mode: Option<u32>| {
        let canvas = node(1, 0, "CANVAS");
        let canvas = mode.map_or(canvas.clone(), |m| in_mode(canvas, m));
        build(vec![canvas, bound_rectangle(2, 1)])
    };
    assert_eq!(
        drawn(&page(None), 1),
        GREEN,
        "the default mode without one set"
    );
    assert_eq!(drawn(&page(Some(302)), 1), BLUE, "the page's mode");
}

#[test]
fn a_layer_drawn_alone_keeps_the_modes_of_the_frames_and_page_above_it() {
    // Page in red, frame in blue, group untouched: the nearest explicit mode wins.
    let doc = build(vec![
        in_mode(node(1, 0, "CANVAS"), 303),
        in_mode(node(3, 1, "FRAME"), 302),
        node(4, 3, "GROUP"),
        bound_rectangle(2, 4),
    ]);
    assert_eq!(
        drawn(&doc, 1),
        BLUE,
        "inside the page, as the page shows it"
    );
    assert_eq!(drawn(&doc, 2), BLUE, "the layer alone");
    // Only the page sets a mode: the layer alone follows it too.
    let doc = build(vec![
        in_mode(node(1, 0, "CANVAS"), 303),
        node(3, 1, "FRAME"),
        bound_rectangle(2, 3),
    ]);
    assert_eq!(
        drawn(&doc, 2),
        RED,
        "the page's mode reaches a layer drawn alone"
    );
}

#[test]
fn a_node_inside_an_instance_keeps_the_modes_of_the_frame_the_instance_sits_in() {
    let mut symbol = node(5, 1, "SYMBOL");
    symbol
        .data
        .set("transform", obj(&[("m02", Val::Float(40.0))]));
    let doc = build(vec![
        node(1, 0, "CANVAS"),
        in_mode(node(3, 1, "FRAME"), 302),
        instance(4, 3, 5, vec![]),
        symbol,
        bound_rectangle(6, 5),
    ]);
    assert_eq!(drawn(&doc, 3), BLUE, "the frame as the page shows it");
    let path = InstancePath::new(vec![id(4), id(6)]).unwrap();
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    let alone = top_left(renderer.render_path(&doc, &path, &viewport()));
    assert_eq!(alone, BLUE, "the node alone, inside its instance");
}

fn drawn_path(doc: &Document, ids: &[u32]) -> [u8; 4] {
    let path = InstancePath::new(ids.iter().copied().map(id).collect()).unwrap();
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    top_left(renderer.render_path(doc, &path, &viewport()))
}

fn mode_map(mode: Option<u32>) -> Val {
    let entries = mode
        .map(|mode| {
            obj(&[
                ("variableSetID", obj(&[("guid", id(SET).to_val())])),
                ("variableModeID", id(mode).to_val()),
            ])
        })
        .into_iter()
        .collect();
    obj(&[("entries", Val::Arr(entries))])
}

#[test]
fn a_node_inside_an_instance_uses_overridden_ancestor_variable_modes() {
    // 1. Set override on intermediate frame inside component.
    let mut symbol = node(5, 1, "SYMBOL");
    symbol
        .data
        .set("transform", obj(&[("m02", Val::Float(40.0))]));
    let set_doc = build(vec![
        node(1, 0, "CANVAS"),
        instance(
            4,
            1,
            5,
            vec![patch(
                &[7],
                &[("variableModeBySetMap", mode_map(Some(302)))],
            )],
        ),
        symbol.clone(),
        node(7, 5, "FRAME"),
        bound_rectangle(6, 7),
    ]);
    assert_eq!(drawn(&set_doc, 1), BLUE, "set override on page");
    assert_eq!(drawn(&set_doc, 4), BLUE, "set override on instance");
    assert_eq!(
        drawn_path(&set_doc, &[4, 6]),
        BLUE,
        "set override on focused instance path"
    );

    // 2. Clear-to-inherit override ({ entries: [] }) lets outer page mode win over master frame.
    let clear_doc = build(vec![
        in_mode(node(1, 0, "CANVAS"), 302),
        instance(
            4,
            1,
            5,
            vec![patch(&[7], &[("variableModeBySetMap", mode_map(None))])],
        ),
        symbol.clone(),
        in_mode(node(7, 5, "FRAME"), 303),
        bound_rectangle(6, 7),
    ]);
    assert_eq!(drawn(&clear_doc, 1), BLUE, "clear override on page");
    assert_eq!(drawn(&clear_doc, 4), BLUE, "clear override on instance");
    assert_eq!(
        drawn_path(&clear_doc, &[4, 6]),
        BLUE,
        "clear override on focused instance path"
    );

    // 3. Replace override changes master frame mode (Red -> Green).
    let replace_doc = build(vec![
        in_mode(node(1, 0, "CANVAS"), 302),
        instance(
            4,
            1,
            5,
            vec![patch(
                &[7],
                &[("variableModeBySetMap", mode_map(Some(301)))],
            )],
        ),
        symbol.clone(),
        in_mode(node(7, 5, "FRAME"), 303),
        bound_rectangle(6, 7),
    ]);
    assert_eq!(drawn(&replace_doc, 1), GREEN, "replace override on page");
    assert_eq!(
        drawn(&replace_doc, 4),
        GREEN,
        "replace override on instance"
    );
    assert_eq!(
        drawn_path(&replace_doc, &[4, 6]),
        GREEN,
        "replace override on focused instance path"
    );

    // 4. Nested instance override with overriddenSymbolID swap.
    let mut inner_default = node(8, 1, "SYMBOL");
    inner_default
        .data
        .set("transform", obj(&[("m02", Val::Float(80.0))]));
    let mut inner_swapped = node(9, 1, "SYMBOL");
    inner_swapped
        .data
        .set("transform", obj(&[("m02", Val::Float(120.0))]));
    let nested_doc = build(vec![
        node(1, 0, "CANVAS"),
        instance(
            4,
            1,
            5,
            vec![
                patch(
                    &[7],
                    &[
                        ("overriddenSymbolID", id(9).to_val()),
                        ("variableModeBySetMap", mode_map(Some(302))),
                    ],
                ),
                patch(&[7, 10], &[("variableModeBySetMap", mode_map(Some(303)))]),
            ],
        ),
        symbol,
        instance(7, 5, 8, vec![]),
        inner_default,
        inner_swapped,
        node(10, 9, "FRAME"),
        bound_rectangle(6, 10),
    ]);
    assert_eq!(drawn(&nested_doc, 1), RED, "nested swap override on page");
    assert_eq!(
        drawn(&nested_doc, 4),
        RED,
        "nested swap override on outer instance"
    );
    assert_eq!(
        drawn_path(&nested_doc, &[4, 7, 6]),
        RED,
        "nested swap override on focused instance path"
    );

    // Nested instance mode itself (without overriding inner frame 10).
    let nested_instance_mode_doc = build(vec![
        node(1, 0, "CANVAS"),
        instance(
            4,
            1,
            5,
            vec![patch(
                &[7],
                &[
                    ("overriddenSymbolID", id(9).to_val()),
                    ("variableModeBySetMap", mode_map(Some(302))),
                ],
            )],
        ),
        {
            let mut s = node(5, 1, "SYMBOL");
            s.data.set("transform", obj(&[("m02", Val::Float(40.0))]));
            s
        },
        instance(7, 5, 8, vec![]),
        {
            let mut s = node(8, 1, "SYMBOL");
            s.data.set("transform", obj(&[("m02", Val::Float(80.0))]));
            s
        },
        {
            let mut s = node(9, 1, "SYMBOL");
            s.data.set("transform", obj(&[("m02", Val::Float(120.0))]));
            s
        },
        node(10, 9, "FRAME"),
        bound_rectangle(6, 10),
    ]);
    assert_eq!(
        drawn(&nested_instance_mode_doc, 1),
        BLUE,
        "nested instance mode override on page"
    );
    assert_eq!(
        drawn_path(&nested_instance_mode_doc, &[4, 7, 6]),
        BLUE,
        "nested instance mode override on focused instance path"
    );
}
