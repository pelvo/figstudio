//! `get_figjam` on a board built in the test (no public FigJam fixture exists).
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_mcp::tools::{figjam::figjam, metadata::metadata};
use rmcp::model::CallToolResult;
use std::path::PathBuf;

fn obj(fields: &[(&str, Val)]) -> Val {
    let mut out = Obj::new();
    for (key, value) in fields {
        out.insert(key, value.clone());
    }
    Val::Obj(out)
}
fn node(id: u32, parent: Guid, kind: &'static str, fields: &[(&str, Val)]) -> Node {
    let mut node = Node::from_data(obj(&[
        ("guid", Guid(9, id).to_val()),
        ("parentIndex", obj(&[("guid", parent.to_val())])),
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
/// A board with one of each FigJam kind under section 9:1 on the first page.
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
            &[
                ("sourceCode", Val::Str("let x = 1 < 2;".into())),
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
fn text(result: &CallToolResult) -> String {
    let value = serde_json::to_value(result).unwrap();
    value["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}
fn images(result: &CallToolResult) -> usize {
    serde_json::to_value(result).unwrap()["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "image")
        .count()
}
fn call(doc: &Document, id: &str, images: bool, scale: Option<f64>) -> CallToolResult {
    figjam(doc, Some(id), None, images, scale, false)
}

#[test]
fn every_figjam_kind_shows_its_details() {
    let xml = text(&call(&board(), "9:1", false, None));
    for want in [
        r#"<section id="9:1""#,
        r#"status="build""#,
        r##"<sticky id="9:2""##,
        r##"color="#FF0000""##,
        ">Ship it</sticky>",
        r#"<shape-with-text id="9:3""#,
        r#"shape="diamond""#,
        ">Decide</shape-with-text>",
        r#"<connector id="9:4""#,
        r#"start="9:2" startMagnet="right" end="9:3" endMagnet="left" lineStyle="elbowed""#,
        r#"language="rust""#,
        ">let x = 1 &lt; 2;</code-block>",
        ">cell</table-cell>",
        r#"widget="Poll""#,
        "<stamp id=\"9:9\"",
    ] {
        assert!(xml.contains(want), "missing {want} in {xml}");
    }
}

#[test]
fn plain_metadata_has_no_figjam_extras_and_the_approximations_line_counts_kinds() {
    let doc = board();
    let plain = text(&metadata(&doc, Some("9:1"), None));
    assert!(!plain.contains("Ship it") && !plain.contains("startMagnet"));
    let xml = text(&call(&doc, "9:1", false, None));
    let note = xml.split("<!--").nth(1).unwrap();
    assert!(note.contains("approximations:"), "{xml}");
    for kind in [
        "sticky 1",
        "shape-with-text 1",
        "connector 1",
        "code_block 1",
        "table 1",
        "table_cell 1",
        "widget 1",
        "stamp 1",
    ] {
        assert!(
            note.contains(&kind.replace('_', "-")),
            "{kind} missing from {note}"
        );
    }
    // A design file has none of these kinds, so no line appears.
    let design = text(&call(&doc, "1:4", false, None));
    assert!(!design.contains("FigJam"), "{design}");
}

#[test]
fn images_are_opt_in_small_and_capped() {
    let doc = board();
    assert_eq!(images(&call(&doc, "9:1", false, None)), 0);
    // Pictured kinds: section, sticky, shape, code block, table, widget, stamp.
    assert_eq!(images(&call(&doc, "9:1", true, None)), 7);
    for bad in [0.0, 0.6] {
        let result = call(&doc, "9:1", true, Some(bad));
        assert_eq!(result.is_error, Some(true), "{bad}");
    }
    assert_eq!(images(&call(&doc, "9:1", true, Some(0.5))), 7);
}

#[test]
fn more_than_twenty_images_are_cut_with_a_note() {
    let mut doc = board();
    let mut nodes = doc.nodes.clone();
    nodes.extend((100..125).map(|n| node(n, Guid(9, 1), "STICKY", &[])));
    doc.set_nodes(nodes).unwrap();
    let result = call(&doc, "9:1", true, None);
    assert_eq!(images(&result), 20);
    let last = serde_json::to_value(&result).unwrap()["content"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(last.contains("more node(s) not pictured"), "{last}");
}

#[test]
fn bad_ids_list_the_pages_and_no_id_lists_pages() {
    let doc = board();
    let bad = figjam(&doc, Some("9:999"), None, false, None, false);
    assert_eq!(bad.is_error, Some(true));
    assert!(text(&bad).contains("Available pages"));
    assert!(text(&figjam(&doc, None, None, false, None, false)).starts_with("<pages"));
}
