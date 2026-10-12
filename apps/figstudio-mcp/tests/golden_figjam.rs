//! Writes the synthetic FigJam board that the golden harness registers (L0).
//!
//! Ignored by default: `scripts/mcp-golden.sh` runs it before the golden record in each tree.
//! The board is built like `tests/figjam.rs` builds its own, on the `structure.fig` fixture,
//! and saved to `apps/figstudio-mcp/figjam-golden.fig` (git-ignored; never committed).
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use std::path::PathBuf;

fn obj(fields: &[(&str, Val)]) -> Val {
    let mut out = Obj::new();
    for (key, value) in fields {
        out.insert(key, value.clone());
    }
    Val::Obj(out)
}

/// A child node; `position` is the ordering key the encoder requires on `parentIndex`.
fn node(id: u32, parent: Guid, kind: &'static str, fields: &[(&str, Val)]) -> Node {
    let position = format!("a{id:02}");
    let mut node = Node::from_data(obj(&[
        ("guid", Guid(9, id).to_val()),
        (
            "parentIndex",
            obj(&[("guid", parent.to_val()), ("position", Val::Str(position))]),
        ),
        ("type", Val::Enum(kind)),
        ("name", Val::Str(kind.to_lowercase())),
        (
            "size",
            obj(&[("x", Val::Float(40.0)), ("y", Val::Float(30.0))]),
        ),
    ]))
    .unwrap();
    for (key, value) in fields {
        node.data.set(key, value.clone());
    }
    node
}

fn text_data(text: &str) -> Val {
    obj(&[("characters", Val::Str(text.into()))])
}

fn solid(r: f64, g: f64, b: f64) -> Val {
    Val::Arr(vec![obj(&[
        ("type", Val::Enum("SOLID")),
        (
            "color",
            obj(&[
                ("r", Val::Float(r)),
                ("g", Val::Float(g)),
                ("b", Val::Float(b)),
                ("a", Val::Float(1.0)),
            ]),
        ),
    ])])
}

fn endpoint(id: u32, magnet: &'static str) -> Val {
    obj(&[
        ("endpointNodeID", Guid(9, id).to_val()),
        ("magnet", Val::Enum(magnet)),
    ])
}

/// One of each FigJam kind under section 9:1 on the first page; `includeImages=true` renders
/// each of them.
fn board() -> Document {
    let mut doc = Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    let page = Guid(0, 1);
    let section = Guid(9, 1);
    let mut nodes = doc.nodes.clone();
    nodes.extend([
        node(1, page, "SECTION", &[("sectionStatus", Val::Enum("BUILD"))]),
        node(
            2,
            section,
            "STICKY",
            &[
                ("textData", text_data("Ship it")),
                ("fillPaints", solid(1.0, 0.0, 0.0)),
            ],
        ),
        node(
            3,
            section,
            "SHAPE_WITH_TEXT",
            &[
                ("shapeWithTextType", Val::Enum("DIAMOND")),
                ("textData", text_data("Decide")),
                ("fillPaints", solid(0.0, 0.0, 1.0)),
            ],
        ),
        node(
            4,
            section,
            "CONNECTOR",
            &[
                ("connectorStart", endpoint(2, "RIGHT")),
                ("connectorEnd", endpoint(3, "LEFT")),
                ("connectorLineStyle", Val::Enum("ELBOWED")),
            ],
        ),
        node(
            5,
            section,
            "CODE_BLOCK",
            // The fixtures' schema has no `sourceCode` field (newer Figma schemas do): text in textData.
            &[
                ("textData", text_data("let x = 1 < 2;")),
                ("codeBlockLanguage", Val::Enum("RUST")),
            ],
        ),
        node(6, section, "TABLE", &[]),
        node(
            7,
            Guid(9, 6),
            "TABLE_CELL",
            &[("textData", text_data("cell"))],
        ),
        node(
            8,
            section,
            "WIDGET",
            &[(
                "widgetMetadata",
                obj(&[("widgetName", Val::Str("Poll".into()))]),
            )],
        ),
        node(9, section, "STAMP", &[]),
    ]);
    doc.set_nodes(nodes).unwrap();
    doc
}

#[test]
#[ignore = "writes the golden FigJam fixture; run by scripts/mcp-golden.sh"]
fn write_figjam_golden() {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("figjam-golden.fig");
    board().save(&out).unwrap();
}
