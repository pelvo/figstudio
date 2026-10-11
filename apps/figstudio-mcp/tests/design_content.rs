//! get_design_context: text and runs, rendered assets, the fixture, caps and errors.
#[path = "design/support.rs"]
mod support;
use figstudio_fig::{Document, Val};
use figstudio_mcp::tools::design::{Assets, MAX_CODE_BYTES, Options, design_context};
use support::*;

fn number(value: f64, units: &'static str) -> Val {
    obj(&[("value", f(value)), ("units", e(units))])
}

#[test]
fn text_typography_and_style_runs() {
    let bold = obj(&[
        ("styleID", Val::Int(1)),
        (
            "fontName",
            obj(&[
                ("family", Val::Str("Inter".into())),
                ("style", Val::Str("Bold Italic".into())),
            ]),
        ),
        (
            "fillPaints",
            Val::Arr(vec![solid(rgba(1.0, 0.0, 0.0, 1.0))]),
        ),
    ]);
    let ids = [0, 0, 0, 1, 1, 0, 0].map(Val::Int).to_vec();
    let text = node(
        1,
        PAGE,
        "TEXT",
        "label",
        &[
            (
                "textData",
                obj(&[
                    ("characters", Val::Str("a<{b}\nc".into())),
                    ("characterStyleIDs", Val::Arr(ids)),
                    ("styleOverrideTable", Val::Arr(vec![bold])),
                ]),
            ),
            (
                "fontName",
                obj(&[
                    ("family", Val::Str("Noto Sans".into())),
                    ("style", Val::Str("Medium".into())),
                ]),
            ),
            ("fontSize", f(14.0)),
            ("lineHeight", number(150.0, "PERCENT")),
            ("letterSpacing", number(-2.0, "PERCENT")),
            ("textAutoResize", e("HEIGHT")),
            ("textAlignHorizontal", e("CENTER")),
            ("textCase", e("UPPER")),
            (
                "fillPaints",
                Val::Arr(vec![solid(rgba(0.0, 0.0, 0.0, 1.0))]),
            ),
        ],
    );
    let out = code(&doc_with(vec![text]), "9:1");
    let want = r#"<p data-node-id="9:1" data-name="label" className="w-[10px] font-['Noto_Sans'] font-[500] text-[14px] leading-[150%] tracking-[-0.02em] uppercase text-center text-[color:#000000]">a&lt;&#123;<span className="font-['Inter'] font-[700] italic text-[color:#FF0000]">b&#125;</span><br />c</p>"#;
    assert_eq!(out.trim_end(), want);
}

fn picture_doc() -> Document {
    let image = obj(&[
        ("type", e("IMAGE")),
        ("image", obj(&[("hash", Val::Bytes(vec![0; 20]))])),
    ]);
    let vector = node(
        1,
        PAGE,
        "ELLIPSE",
        "dot",
        &[(
            "fillPaints",
            Val::Arr(vec![solid(rgba(1.0, 0.0, 0.0, 1.0))]),
        )],
    );
    let mut vector = vector;
    vector.kind = "VECTOR".into();
    vector.data.set("type", e("VECTOR"));
    let frame = node(
        2,
        PAGE,
        "FRAME",
        "../../photo",
        &[("transform", at(20.0, 0.0))],
    );
    let photo = node(
        3,
        g(2),
        "RECTANGLE",
        "photo",
        &[("fillPaints", Val::Arr(vec![image]))],
    );
    doc_with(vec![vector, frame, photo])
}

#[test]
fn vectors_and_image_fills_become_inline_png_assets() {
    let doc = picture_doc();
    let result = design_context(&doc, "9:2", &inline());
    let text = texts(&result);
    assert!(text[0].contains(r#"<img data-node-id="9:3" data-name="photo" alt="photo" src="structure_9-3.png" className="absolute left-[0px] top-[0px] w-[10px] h-[10px]" />"#), "{}", text[0]);
    assert!(
        text[1].contains("- structure_9-3.png (node 9:3): structure_9-3.png"),
        "{}",
        text[1]
    );
    let json = serde_json::to_value(&result).unwrap();
    let images: Vec<_> = json["content"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["type"] == "image")
        .collect();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0]["mimeType"], "image/png");
    let vector = code(&doc, "9:1");
    assert!(vector.contains(r#"src="structure_9-1.png""#), "{vector}");
}

#[test]
fn asset_files_stay_inside_the_assets_directory() {
    let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("design-assets");
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("out");
    let options = Options {
        file_key: "../evil key".into(),
        assets: Assets::dir(dir.clone()),
        ..Default::default()
    };
    let result = design_context(&picture_doc(), "9:1", &options);
    let text = texts(&result);
    let file = dir.join("___evil_key_9-1.png");
    assert!(
        text[0].contains(&format!("src=\"{}\"", file.display())),
        "{}",
        text[0]
    );
    let png = std::fs::read(&file).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Public read like a new Code Connect sidecar, whatever the process umask is.
        let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "asset file mode");
    }
    let listing: Vec<_> = std::fs::read_dir(&base)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        listing,
        ["out"],
        "nothing written next to the assets directory"
    );
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "no temporary files left"
    );
    // A non-empty directory where the asset goes makes the final rename fail.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir_all(file.join("x")).unwrap();
    let result = design_context(&picture_doc(), "9:1", &options);
    let text = texts(&result);
    assert!(
        text[1].contains("1 assets not written (assets directory error)"),
        "{}",
        text[1]
    );
    assert!(
        text[0].contains(r#"<div data-node-id="9:1""#),
        "{}",
        text[0]
    );
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "temporary file removed"
    );
}

#[test]
fn hidden_layers_do_not_use_the_node_cap() {
    let parent = node(10, PAGE, "FRAME", "parent", &[]);
    let hidden = node(
        1,
        g(10),
        "FRAME",
        "hidden",
        &[("visible", Val::Bool(false))],
    );
    let inner = node(2, g(1), "FRAME", "inner", &[]);
    let shown = node(3, g(10), "FRAME", "shown", &[]);
    let doc = doc_with(vec![parent, hidden, inner, shown]);
    let result = design_context(
        &doc,
        "9:10",
        &Options {
            max_nodes: Some(2),
            ..inline()
        },
    );
    let text = texts(&result);
    assert!(text[0].contains(r#"data-name="shown""#), "{}", text[0]);
    assert!(
        !text[0].contains("hidden") && !text[1].contains("TRUNCATED"),
        "{text:?}"
    );
}
use std::path::PathBuf;

#[test]
fn fixture_page_with_instances_and_overrides() {
    let doc = Document::open(fixture()).unwrap();
    let out = code(&doc, "1:4");
    for want in [
        r#"<div data-node-id="1:38" data-name="Component 2""#,
        r#"<p data-node-id="I1:38;1:42" data-name="ABC""#,
        ">XYZ</p>",
        r#"data-node-id="I1:45;1:35" data-name="Rectangle 1" className="absolute left-[0px] top-[0px] w-[45px] h-[45px] bg-[#FF0000]""#,
        r#"<img data-node-id="1:26" data-name="Vector""#,
    ] {
        assert!(out.contains(want), "missing {want} in {out}");
    }
    let child = code(&doc, "I1:38;1:42");
    assert!(
        child.starts_with(r#"<p data-node-id="I1:38;1:42""#) && child.contains(">XYZ</p>"),
        "{child}"
    );
    let notes = notes(&doc, "1:4");
    assert!(
        notes.starts_with("## Instructions for the agent\n"),
        "{notes}"
    );
    assert!(
        notes.contains("get_screenshot")
            && notes.contains("get_metadata")
            && notes.contains("## Assets")
    );
    let echoed = Options {
        frameworks: Some("Vue".into()),
        languages: Some("typescript".into()),
        ..inline()
    };
    let result = design_context(&doc, "1:38", &echoed);
    assert!(texts(&result)[1].contains("Rewrite it for Vue / typescript"));
}

#[test]
fn caps_truncate_with_a_note_pointing_to_get_metadata() {
    let doc = Document::open(fixture()).unwrap();
    let capped = Options {
        max_nodes: Some(3),
        ..inline()
    };
    let result = design_context(&doc, "1:4", &capped);
    let text = texts(&result);
    assert_eq!(text[0].matches("data-node-id=").count(), 3, "{}", text[0]);
    assert!(
        text[1].contains("TRUNCATED") && text[1].contains("get_metadata"),
        "{}",
        text[1]
    );
    assert!(!notes(&doc, "1:4").contains("TRUNCATED"));
    let wide: Vec<_> = (1..=1200)
        .map(|i| node(i, PAGE, "FRAME", &"n".repeat(200), &[]))
        .collect();
    let doc = doc_with(wide);
    let result = design_context(
        &doc,
        "0:1",
        &Options {
            max_nodes: Some(5_000),
            ..inline()
        },
    );
    let text = texts(&result);
    assert!(text[0].len() < MAX_CODE_BYTES + 1_000, "{}", text[0].len());
    assert!(
        text[0].trim_end().ends_with("</div>"),
        "closed after the cut"
    );
    assert!(text[1].contains("TRUNCATED"));
}

#[test]
fn unknown_and_invalid_ids_are_tool_errors() {
    let doc = Document::open(fixture()).unwrap();
    let broken = node(
        1,
        PAGE,
        "INSTANCE",
        "broken",
        &[("symbolData", obj(&[("symbolID", g(99).to_val())]))],
    );
    let note = notes(&doc_with(vec![broken]), "9:1");
    assert!(
        note.contains("approximations: 1 instances not expanded"),
        "{note}"
    );
    for id in ["99:99", "I1:38;9:9", "I1:4;1:5", "nonsense", "0:0"] {
        let result = design_context(&doc, id, &inline());
        assert_eq!(result.is_error, Some(true), "{id}");
    }
}
