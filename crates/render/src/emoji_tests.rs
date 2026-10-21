//! Synthetic fonts and documents make the whole draw path independent of the host.
use figstudio_fig::{Document, Guid, Node, Obj, Val};
use fontdb::Database;
use image::ImageEncoder as _;
use vello_cpu::kurbo::Affine;
#[cfg(feature = "parallel")]
use vello_cpu::kurbo::Rect;

use crate::{Renderer, Viewport, text::SystemFonts};

pub(super) fn obj(fields: &[(&str, Val)]) -> Val {
    Val::Obj(fields.iter().fold(Obj::new(), |mut o, (k, v)| {
        o.insert(k, v.clone());
        o
    }))
}

pub(super) fn document(points: &[i64]) -> Document {
    let mut doc = Document::open(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/structure.fig"
    ))
    .unwrap();
    let canvas = Node::from_data(obj(&[
        ("guid", Guid(99, 1).to_val()),
        ("type", Val::Str("CANVAS".into())),
        (
            "parentIndex",
            obj(&[
                ("guid", Guid(0, 0).to_val()),
                ("position", Val::Str("a".into())),
            ]),
        ),
    ]))
    .unwrap();
    let text = Node::from_data(obj(&[
        ("guid", Guid(99, 2).to_val()),
        ("type", Val::Str("TEXT".into())),
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
            Val::Arr(vec![obj(&[
                ("type", Val::Str("SOLID".into())),
                (
                    "color",
                    obj(&[
                        ("r", Val::Float(1.0)),
                        ("g", Val::Float(0.0)),
                        ("b", Val::Float(0.0)),
                        ("a", Val::Float(1.0)),
                    ]),
                ),
            ])]),
        ),
        (
            "derivedTextData",
            obj(&[(
                "glyphs",
                Val::Arr(vec![obj(&[
                    (
                        "emojiCodePoints",
                        Val::Arr(points.iter().copied().map(Val::Int).collect()),
                    ),
                    ("fontSize", Val::Float(40.0)),
                    (
                        "position",
                        obj(&[("x", Val::Float(10.0)), ("y", Val::Float(50.0))]),
                    ),
                ])]),
            )]),
        ),
    ]))
    .unwrap();
    let root = Node::from_data(obj(&[
        ("guid", Guid(0, 0).to_val()),
        ("type", Val::Enum("DOCUMENT")),
    ]))
    .unwrap();
    doc.set_nodes(vec![canvas, text, root]).unwrap();
    doc
}

#[test]
fn synthetic_colour_emoji_keeps_position_opacity_and_recording() {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &(0..256)
                .flat_map(|i| {
                    [
                        [0, 0, 255, 128],
                        [0, 255, 0, 255],
                        [255, 255, 0, 255],
                        [255, 0, 255, 255],
                    ][usize::from(i % 16 >= 8) + 2 * usize::from(i / 16 >= 8)]
                })
                .collect::<Vec<_>>(),
            16,
            16,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    let mut db = Database::new();
    db.load_font_data(crate::text_test_font::colour_emoji(&png));
    let mut renderer = Renderer {
        fonts: SystemFonts::with_database(db),
        ..Renderer::new()
    };
    let mut doc = document(&[0x2764]);
    let viewport = Viewport {
        width: 80,
        height: 80,
        transform: Affine::IDENTITY,
        background: None,
    };
    let full = renderer.render(&doc, Guid(99, 1), &viewport);
    let colours: std::collections::HashSet<_> = full
        .pixmap
        .data_as_u8_slice()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] == 255)
        .map(|p| p.to_vec())
        .collect();
    assert!(colours.len() >= 3);
    assert!(
        full.pixmap
            .data_as_u8_slice()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[2] > 0 && p[3] == 128)
    );
    assert!(!colours.contains(&vec![255, 0, 0, 255]));
    assert_eq!(full.stats.emoji_approximated, 0);
    assert_eq!(full.stats.emoji_skipped, 0);
    let mut nodes = doc.nodes.clone();
    nodes[1].data.set("opacity", Val::Float(0.5));
    let Some(Val::Arr(fills)) = nodes[1].data.get_mut("fillPaints") else {
        panic!("missing test fill")
    };
    fills[0].set("opacity", Val::Float(0.5));
    nodes[1].data.set(
        "transform",
        obj(&[("m02", Val::Float(10.0)), ("m12", Val::Float(5.0))]),
    );
    doc.set_nodes(nodes).unwrap();
    let moved = renderer.render(&doc, Guid(99, 1), &viewport);
    let at = |x: usize, y: usize| (y * 80 + x) * 4;
    for (x, y) in [(12, 42), (15, 45), (18, 48)] {
        let a = &full.pixmap.data_as_u8_slice()[at(x, y)..at(x, y) + 4];
        let b = &moved.pixmap.data_as_u8_slice()[at(x + 10, y + 5)..at(x + 10, y + 5) + 4];
        assert!(a[3] > 0);
        assert!(b[3].abs_diff(a[3] / 4) <= 1);
    }
    #[cfg(feature = "parallel")]
    {
        let rec = renderer
            .record(
                &doc,
                Guid(99, 1),
                Affine::IDENTITY,
                Rect::new(0.0, 0.0, 80.0, 80.0),
                1 << 20,
            )
            .0
            .unwrap();
        let replayed = rec.render(&viewport).unwrap().0;
        assert_eq!(
            moved.pixmap.data_as_u8_slice(),
            replayed.pixmap.data_as_u8_slice()
        );
        #[cfg(feature = "gpu")]
        assert_eq!(
            rec.gpu_scene(&viewport, 4096).unwrap().err(),
            Some(crate::GpuUnsupported::Image)
        );
    }
    let mut nodes = doc.nodes.clone();
    let mut frame = nodes[0].data.clone();
    frame.set("type", Val::Enum("FRAME"));
    frame.set(
        "size",
        obj(&[("x", Val::Float(30.0)), ("y", Val::Float(60.0))]),
    );
    nodes[0] = Node::from_data(frame).unwrap();
    doc.set_nodes(nodes).unwrap();
    let clipped = renderer.render(&doc, Guid(99, 1), &viewport);
    assert!(
        clipped
            .pixmap
            .data_as_u8_slice()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] > 0)
    );
    for (i, p) in clipped
        .pixmap
        .data_as_u8_slice()
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
    {
        if i % 80 >= 30 || i / 80 >= 60 {
            assert_eq!(p[3], 0, "emoji escaped its parent clip");
        }
    }
}

#[test]
fn empty_database_skips_and_outline_only_face_is_counted() {
    let mut db = Database::new();
    db.load_font_data(epaint_default_fonts::HACK_REGULAR.to_vec());
    let mut info = db.faces().next().unwrap().clone();
    db.remove_face(info.id);
    info.families = vec![(
        "Noto Color Emoji".into(),
        fontdb::Language::English_UnitedStates,
    )];
    db.push_face_info(info);
    for (db, approximated, skipped) in [(Database::new(), 0, 1), (db, 1, 0)] {
        let mut renderer = Renderer {
            fonts: SystemFonts::with_database(db),
            ..Renderer::new()
        };
        let frame = renderer.render(
            &document(&[65]),
            Guid(99, 1),
            &Viewport {
                width: 80,
                height: 80,
                transform: Affine::IDENTITY,
                background: None,
            },
        );
        assert_eq!(frame.stats.emoji_approximated, approximated);
        assert_eq!(frame.stats.emoji_skipped, skipped);
        assert_eq!(
            frame
                .pixmap
                .data_as_u8_slice()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] > 0),
            approximated == 1
        );
    }
}
