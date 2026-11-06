//! A prototype built on the public `structure.fig` fixture (no fixture has prototype data).
//! Uses only `figstudio_fig`, so other crates' tests include it with `#[path]`.
#![allow(
    dead_code,
    reason = "each suite that includes this file uses a different subset"
)]
use std::path::PathBuf;

use figstudio_fig::{Document, Guid, Obj, Val, intern};

/// The "Groups" frame: the flow's first screen.
pub const FIRST: Guid = Guid(1, 2);
/// The "Symbols and images" frame: where the first screen's link goes.
pub const SECOND: Guid = Guid(1, 4);
/// A group on the first screen that links to [`SECOND`].
pub const LINK: Guid = Guid(1, 53);
/// An instance on the second screen that goes back.
pub const BACK: Guid = Guid(1, 38);
/// An instance on the second screen with a hover overlay, which is not played.
pub const HOVER: Guid = Guid(1, 45);

pub fn fields(fields: &[(&str, Val)]) -> Val {
    Val::Obj(fields.iter().fold(Obj::new(), |mut o, (k, v)| {
        o.insert(k, v.clone());
        o
    }))
}

pub fn name(value: &str) -> Val {
    Val::Enum(intern(value))
}

/// One interaction with one action, as Figma stores it.
pub fn interaction(trigger: &str, action: &[(&str, Val)]) -> Val {
    Val::Arr(vec![fields(&[
        ("id", Guid(1, 9000).to_val()),
        ("event", fields(&[("interactionType", name(trigger))])),
        ("actions", Val::Arr(vec![fields(action)])),
    ])])
}

pub fn navigate(to: Guid) -> Val {
    interaction(
        "ON_CLICK",
        &[
            ("transitionNodeID", to.to_val()),
            ("connectionType", name("INTERNAL_NODE")),
            ("navigationType", name("NAVIGATE")),
            ("transitionType", name("INSTANT_TRANSITION")),
        ],
    )
}

pub fn fixture(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(file)
}

/// `structure.fig` with a flow "Start" on [`FIRST`], [`LINK`] → [`SECOND`], [`BACK`] going
/// back and [`HOVER`] opening an overlay on hover.
pub fn prototype_doc() -> Document {
    let mut doc = Document::open(fixture("structure.fig")).unwrap();
    let mut nodes = doc.nodes.clone();
    for node in &mut nodes {
        let set = |key: &str, value: Val| (key.to_owned(), value);
        let change = match node.id {
            FIRST => Some(set(
                "prototypeStartingPoint",
                fields(&[
                    ("name", Val::Str("Start".into())),
                    ("description", Val::Str(String::new())),
                    ("position", Val::Str("a".into())),
                ]),
            )),
            LINK => Some(set("prototypeInteractions", navigate(SECOND))),
            BACK => Some(set(
                "prototypeInteractions",
                interaction("ON_CLICK", &[("connectionType", name("BACK"))]),
            )),
            HOVER => Some(set(
                "prototypeInteractions",
                interaction(
                    "ON_HOVER",
                    &[
                        ("transitionNodeID", FIRST.to_val()),
                        ("connectionType", name("INTERNAL_NODE")),
                        ("navigationType", name("OVERLAY")),
                    ],
                ),
            )),
            _ => None,
        };
        if let Some((key, value)) = change {
            node.data.set(&key, value);
        }
    }
    doc.set_nodes(nodes).unwrap();
    doc
}

/// [`prototype_doc`] written to a temporary `.fig` file.
pub fn prototype_file(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("prototype.fig");
    prototype_doc().save(&path).unwrap();
    path
}

fn at(
    id: Guid,
    parent: Guid,
    kind: &str,
    (x, y, w, h): (f64, f64, f64, f64),
) -> figstudio_fig::Node {
    figstudio_fig::Node::from_data(fields(&[
        ("guid", id.to_val()),
        ("type", Val::Str(kind.into())),
        ("visible", Val::Bool(true)),
        (
            "parentIndex",
            fields(&[
                ("guid", parent.to_val()),
                ("position", Val::Str("~".into())),
            ]),
        ),
        (
            "size",
            fields(&[("x", Val::Float(w)), ("y", Val::Float(h))]),
        ),
        (
            "transform",
            fields(&[
                ("m00", Val::Float(1.0)),
                ("m11", Val::Float(1.0)),
                ("m02", Val::Float(x)),
                ("m12", Val::Float(y)),
            ]),
        ),
    ]))
    .unwrap()
}

/// The hover button of [`hover_doc`]: an instance on [`FIRST`].
pub const BUTTON: Guid = Guid(1, 9101);
/// Its hover variant.
pub const HOVERED: Guid = Guid(1, 9103);

/// [`prototype_doc`] plus a button on [`FIRST`]: instance [`BUTTON`] of a default variant
/// that changes to [`HOVERED`] while hovered (a component set off the screens).
pub fn hover_doc() -> Document {
    let mut doc = prototype_doc();
    let page = doc.pages[0];
    let set = Guid(1, 9100);
    let default = Guid(1, 9102);
    let mut nodes = doc.nodes.clone();
    let mut s = at(set, page, "FRAME", (-500.0, -500.0, 200.0, 100.0));
    s.data.set("isStateGroup", Val::Bool(true));
    let mut d = at(default, set, "SYMBOL", (0.0, 0.0, 60.0, 30.0));
    d.data.set(
        "prototypeInteractions",
        interaction(
            "ON_HOVER",
            &[
                ("transitionNodeID", HOVERED.to_val()),
                ("connectionType", name("INTERNAL_NODE")),
                ("navigationType", name("SWAP_STATE")),
            ],
        ),
    );
    let h = at(HOVERED, set, "SYMBOL", (100.0, 0.0, 60.0, 30.0));
    let mut b = at(BUTTON, FIRST, "INSTANCE", (300.0, 20.0, 60.0, 30.0));
    b.data
        .set("symbolData", fields(&[("symbolID", default.to_val())]));
    nodes.extend([s, d, h, b]);
    doc.set_nodes(nodes).unwrap();
    doc
}
