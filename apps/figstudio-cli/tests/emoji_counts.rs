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
    std::fs::remove_dir_all(dir).unwrap();
}
