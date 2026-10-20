//! Emoji reporting exposes counts, never marker contents.
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use std::{path::PathBuf, process::Command};

fn obj(fields: &[(&str, Val)]) -> Val {
    Val::Obj(fields.iter().fold(Obj::new(), |mut o, (k, v)| {
        o.insert(k, v.clone());
        o
    }))
}

#[test]
fn info_and_doctor_report_unavailable_emoji_as_counts() {
    let mut doc = Document::open(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig"),
    )
    .unwrap();
    let root = Node::from_data(obj(&[
        ("guid", Guid(0, 0).to_val()),
        ("type", Val::Enum("DOCUMENT")),
    ]))
    .unwrap();
    let canvas = Node::from_data(obj(&[
        ("guid", Guid(99, 1).to_val()),
        ("type", Val::Enum("CANVAS")),
        (
            "parentIndex",
            obj(&[
                ("guid", root.id.to_val()),
                ("position", Val::Str("a".into())),
            ]),
        ),
    ]))
    .unwrap();
    let text = Node::from_data(obj(&[
        ("guid", Guid(99, 2).to_val()),
        ("type", Val::Enum("TEXT")),
        (
            "parentIndex",
            obj(&[
                ("guid", canvas.id.to_val()),
                ("position", Val::Str("a".into())),
            ]),
        ),
        (
            "size",
            obj(&[("x", Val::Float(40.0)), ("y", Val::Float(40.0))]),
        ),
        (
            "fillPaints",
            Val::Arr(vec![obj(&[("type", Val::Enum("SOLID"))])]),
        ),
        (
            "derivedTextData",
            obj(&[(
                "glyphs",
                Val::Arr(vec![obj(&[
                    // The highest Unicode noncharacter has no installed emoji glyph.
                    ("emojiCodePoints", Val::Arr(vec![Val::Int(0x10FFFF)])),
                    ("fontSize", Val::Float(20.0)),
                    (
                        "position",
                        obj(&[("x", Val::Float(0.0)), ("y", Val::Float(20.0))]),
                    ),
                ])]),
            )]),
        ),
    ]))
    .unwrap();
    doc.set_nodes(vec![root, canvas, text]).unwrap();
    let dir = std::env::temp_dir().join(format!("figstudio-emoji-counts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("synthetic.fig");
    std::fs::write(&path, doc.to_bytes().unwrap()).unwrap();
    for (command, counts) in [
        ("info", "emoji:    0 approximated, 1 skipped"),
        ("doctor", "1 emoji skipped"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
            .arg(command)
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = String::from_utf8(output.stdout).unwrap();
        assert!(report.contains(counts), "{command}: {report}");
        assert!(!report.contains("10FFFF"));
        assert!(!report.contains("1114111"));
    }
    // Two visible markers, a truncated one, and a hidden subtree. Metadata must not
    // depend on host fonts or shrink the tiny text away while fitting a huge page.
    let mut nodes = doc.nodes.clone();
    let glyph = nodes[2]
        .data
        .path(&["derivedTextData", "glyphs"])
        .unwrap()
        .items()[0]
        .clone();
    let mut heart = glyph.clone();
    heart.set(
        "emojiCodePoints",
        Val::Arr(vec![Val::Int(0x2764), Val::Int(0xFE0F)]),
    );
    nodes[2].data.set(
        "derivedTextData",
        obj(&[
            (
                "glyphs",
                Val::Arr(vec![heart.clone(), glyph, heart.clone()]),
            ),
            ("truncationStartIndex", Val::Int(2)),
        ]),
    );
    let mut distant = nodes[2].clone();
    distant.data.set("guid", Guid(99, 3).to_val());
    distant.data.set("type", Val::Enum("RECTANGLE"));
    distant.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(1.0)),
            ("m01", Val::Float(0.0)),
            ("m02", Val::Float(1e9)),
            ("m10", Val::Float(0.0)),
            ("m11", Val::Float(1.0)),
            ("m12", Val::Float(0.0)),
        ]),
    );
    nodes.push(Node::from_data(distant.data).unwrap());
    let mut hidden = nodes[2].data.clone();
    hidden.set("guid", Guid(99, 4).to_val());
    hidden.set("type", Val::Enum("FRAME"));
    hidden.set("visible", Val::Bool(false));
    nodes.push(Node::from_data(hidden).unwrap());
    let mut child = nodes[2].data.clone();
    child.set("guid", Guid(99, 5).to_val());
    child.set(
        "parentIndex",
        obj(&[
            ("guid", Guid(99, 4).to_val()),
            ("position", Val::Str("a".into())),
        ]),
    );
    nodes.push(Node::from_data(child).unwrap());
    doc.set_nodes(nodes).unwrap();
    std::fs::write(&path, doc.to_bytes().unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
        .arg("info")
        .arg(&path)
        .arg("--no-system-fonts")
        .output()
        .unwrap();
    assert!(output.status.success());
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(
        report.contains("emoji:    0 approximated, 2 skipped"),
        "{report}"
    );
    let mut renderer = figstudio_render::Renderer::new();
    renderer.system_fonts = false;
    let frame = renderer.render(
        &doc,
        Guid(99, 1),
        &figstudio_render::Viewport {
            width: 80,
            height: 80,
            transform: figstudio_render::kurbo::Affine::IDENTITY,
            background: None,
        },
    );
    assert_eq!(frame.stats.emoji_skipped, 2);
    assert_eq!(frame.stats.emoji_approximated, 0);
    // The only marker now lives in an instance's derived patch, not on a raw node.
    let mut nodes = doc.nodes[..3].to_vec();
    nodes[2].data.set(
        "parentIndex",
        obj(&[
            ("guid", Guid(99, 4).to_val()),
            ("position", Val::Str("a".into())),
        ]),
    );
    nodes[2]
        .data
        .set("derivedTextData", obj(&[("glyphs", Val::Arr(vec![]))]));
    nodes[2] = Node::from_data(nodes[2].data.clone()).unwrap();
    nodes.push(
        Node::from_data(obj(&[
            ("guid", Guid(99, 4).to_val()),
            ("type", Val::Enum("SYMBOL")),
        ]))
        .unwrap(),
    );
    nodes.push(
        Node::from_data(obj(&[
            ("guid", Guid(99, 5).to_val()),
            ("type", Val::Enum("INSTANCE")),
            (
                "parentIndex",
                obj(&[
                    ("guid", Guid(99, 1).to_val()),
                    ("position", Val::Str("b".into())),
                ]),
            ),
            (
                "size",
                obj(&[("x", Val::Float(40.0)), ("y", Val::Float(40.0))]),
            ),
            ("symbolData", obj(&[("symbolID", Guid(99, 4).to_val())])),
            (
                "derivedSymbolData",
                Val::Arr(vec![obj(&[
                    (
                        "guidPath",
                        obj(&[("guids", Val::Arr(vec![Guid(99, 2).to_val()]))]),
                    ),
                    ("derivedTextData", obj(&[("glyphs", Val::Arr(vec![heart]))])),
                ])]),
            ),
        ]))
        .unwrap(),
    );
    doc.set_nodes(nodes).unwrap();
    std::fs::write(&path, doc.to_bytes().unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_figstudio-cli"))
        .arg("info")
        .arg(&path)
        .arg("--no-system-fonts")
        .output()
        .unwrap();
    assert!(output.status.success());
    let report = String::from_utf8(output.stdout).unwrap();
    assert!(
        report.contains("emoji:    0 approximated, 1 skipped"),
        "override-only marker: {report}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
