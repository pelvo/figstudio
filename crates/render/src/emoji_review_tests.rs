//! Review regressions use only synthetic documents and fonts.
use figstudio_fig::Guid;
use fontdb::Database;
use image::ImageEncoder as _;
use vello_cpu::kurbo::Affine;

use crate::{Renderer, Viewport, emoji_tests::document, text::SystemFonts, text_test_font};

fn renderer(fonts: Vec<Vec<u8>>) -> Renderer {
    let mut db = Database::new();
    for font in fonts {
        db.load_font_data(font);
    }
    Renderer {
        fonts: SystemFonts::with_database(db),
        ..Renderer::new()
    }
}

fn viewport() -> Viewport {
    Viewport {
        width: 80,
        height: 80,
        transform: Affine::IDENTITY,
        background: None,
    }
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[0, 200, 0, 255].repeat((w * h) as usize),
            w,
            h,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    png
}

#[test]
fn outline_emoji_family_draws_and_counts_as_mono() {
    let mut renderer = renderer(vec![text_test_font::outline_emoji()]);
    let frame = renderer.render(&document(&[0x2764]), Guid(99, 1), &viewport());
    assert_eq!(frame.stats.emoji_skipped, 0);
    assert_eq!(frame.stats.emoji_approximated, 1);
    assert!(
        frame
            .pixmap
            .data_as_u8_slice()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] > 0)
    );
}

#[test]
fn unsupported_sbix_falls_through_to_own_outline_or_next_face() {
    for tag in [b"jpg ", b"tiff", b"flip"] {
        for own_outline in [true, false] {
            let mut fonts = vec![text_test_font::emoji_strike(
                &png(2, 2),
                (0, 0),
                tag,
                own_outline,
            )];
            if !own_outline {
                fonts.push(text_test_font::outline_emoji());
            }
            let mut renderer = renderer(fonts);
            let frame = renderer.render(&document(&[0x2764]), Guid(99, 1), &viewport());
            assert_eq!(frame.stats.emoji_skipped, 0, "{tag:?}, own={own_outline}");
            assert_eq!(frame.stats.emoji_approximated, 1);
        }
    }
}

#[test]
fn modifiers_are_not_multi_emoji_approximations() {
    let mut renderer = renderer(vec![text_test_font::colour_emoji(&png(16, 16))]);
    for (points, approximated) in [
        (vec![0x2764, 0xFE0F], 0),
        (vec![0x2764, 0xFE0E], 0),
        (vec![0x2764, 0x1F3FD], 0),
        (vec![0x2764, 0x200D, 0x2764, 0x200D, 0x2764], 1),
        (vec![0x2764, 0x20E3], 1),
    ] {
        let frame = renderer.render(&document(&points), Guid(99, 1), &viewport());
        assert_eq!(frame.stats.emoji_skipped, 0);
        assert_eq!(frame.stats.emoji_approximated, approximated, "{points:?}");
        let counts = renderer.emoji_counts(&document(&points));
        assert_eq!(counts.emoji_approximated, frame.stats.emoji_approximated);
        assert_eq!(counts.emoji_skipped, frame.stats.emoji_skipped);
    }
}

#[test]
fn strike_origin_dimensions_and_ppem_determine_placement() {
    // Size 40 is equidistant from the strikes, choosing 64 ppem. The 4x2 bitmap
    // becomes 2.5x1.25, with top left (11.25, 46.875), baseline (10, 50).
    let mut renderer = renderer(vec![text_test_font::emoji_strike(
        &png(4, 2),
        (2, 3),
        b"png ",
        false,
    )]);
    let frame = renderer.render(&document(&[0x2764]), Guid(99, 1), &viewport());
    let px = frame.pixmap.data_as_u8_slice();
    let alpha = |x: usize, y: usize| px[(y * 80 + x) * 4 + 3];
    assert_eq!(alpha(12, 47), 255);
    for (x, y) in [(10, 47), (14, 47), (12, 45), (12, 49), (40, 40)] {
        assert_eq!(alpha(x, y), 0, "unexpected ink at {x},{y}");
    }
}

#[test]
fn emoji_only_and_mixed_text_cast_counted_rectangular_shadows() {
    use crate::emoji_tests::obj;
    use figstudio_fig::Val;
    let mut rgba = [0, 200, 0, 255].repeat(16 * 16);
    for y in 5..11 {
        for x in 5..11 {
            rgba[(y * 16 + x) * 4 + 3] = 0;
        }
    }
    let mut image = Vec::new();
    image::codecs::png::PngEncoder::new(&mut image)
        .write_image(&rgba, 16, 16, image::ExtendedColorType::Rgba8)
        .unwrap();
    for mixed in [false, true] {
        let mut renderer = renderer(vec![text_test_font::colour_emoji(&image)]);
        let mut doc = document(&[0x2764]);
        let mut nodes = doc.nodes.clone();
        nodes[1].data.set(
            "effects",
            Val::Arr(vec![obj(&[
                ("type", Val::Enum("DROP_SHADOW")),
                ("radius", Val::Float(0.0)),
                (
                    "offset",
                    obj(&[("x", Val::Float(30.0)), ("y", Val::Float(0.0))]),
                ),
                (
                    "color",
                    obj(&[
                        ("r", Val::Float(0.0)),
                        ("g", Val::Float(0.0)),
                        ("b", Val::Float(0.0)),
                        ("a", Val::Float(1.0)),
                    ]),
                ),
            ])]),
        );
        if mixed {
            let index = doc.blobs.len();
            let mut commands = Vec::new();
            for (cmd, x, y) in [(1, 0f32, 0f32), (2, 1.0, 0.0), (2, 1.0, 1.0), (2, 0.0, 1.0)] {
                commands.push(cmd);
                commands.extend(x.to_le_bytes());
                commands.extend(y.to_le_bytes());
            }
            commands.push(0);
            doc.blobs.push(commands);
            let ordinary = obj(&[
                ("commandsBlob", Val::Int(index as i64)),
                ("fontSize", Val::Float(8.0)),
                (
                    "position",
                    obj(&[("x", Val::Float(35.0)), ("y", Val::Float(20.0))]),
                ),
            ]);
            let mut derived = nodes[1].data.get("derivedTextData").unwrap().clone();
            let mut glyphs = derived.get("glyphs").unwrap().items().to_vec();
            glyphs.push(ordinary);
            derived.set("glyphs", Val::Arr(glyphs));
            nodes[1].data.set("derivedTextData", derived);
        }
        doc.set_nodes(nodes).unwrap();
        let frame = renderer.render(&doc, Guid(99, 1), &viewport());
        let px = frame.pixmap.data_as_u8_slice();
        let at = |x: usize, y: usize| &px[(y * 80 + x) * 4..(y * 80 + x) * 4 + 4];
        // The hole is not filled in text colour. Its shadow is deliberately a counted rect,
        // distinguishing the silhouette from fallback_shadow's whole-node alpha filter.
        assert_eq!(at(15, 45)[3], 0, "text-colour box, mixed={mixed}");
        assert_eq!(
            at(45, 45),
            &[0, 0, 0, 255],
            "missing emoji shadow, mixed={mixed}"
        );
        assert_eq!(frame.stats.shadows, 1);
        assert_eq!(frame.stats.approximated_effects, 1);
    }
}

#[test]
fn override_only_emoji_is_counted_and_drawn_with_a_synthetic_font() {
    use crate::emoji_tests::obj;
    use figstudio_fig::{Node, Val};
    let mut doc = document(&[0x2764]);
    let mut nodes = doc.nodes.clone();
    let derived = nodes[1].data.get("derivedTextData").unwrap().clone();
    nodes[1]
        .data
        .set("derivedTextData", obj(&[("glyphs", Val::Arr(vec![]))]));
    nodes[1].data.set(
        "parentIndex",
        obj(&[
            ("guid", Guid(99, 4).to_val()),
            ("position", Val::Str("a".into())),
        ]),
    );
    nodes[1] = Node::from_data(nodes[1].data.clone()).unwrap();
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
                obj(&[("x", Val::Float(80.0)), ("y", Val::Float(80.0))]),
            ),
            ("symbolData", obj(&[("symbolID", Guid(99, 4).to_val())])),
            (
                "derivedSymbolData",
                Val::Arr(vec![obj(&[
                    (
                        "guidPath",
                        obj(&[("guids", Val::Arr(vec![Guid(99, 2).to_val()]))]),
                    ),
                    ("derivedTextData", derived),
                ])]),
            ),
        ]))
        .unwrap(),
    );
    doc.set_nodes(nodes).unwrap();
    assert!(doc.nodes.iter().all(|n| {
        n.data
            .path(&["derivedTextData", "glyphs"])
            .is_none_or(|g| g.items().is_empty())
    }));
    let mut renderer = renderer(vec![text_test_font::colour_emoji(&png(16, 16))]);
    let counts = renderer.emoji_counts(&doc);
    let frame = renderer.render(&doc, Guid(99, 1), &viewport());
    assert_eq!(counts.emoji_approximated, 0);
    assert_eq!(counts.emoji_skipped, 0);
    assert_eq!(frame.stats.text_stored_glyphs, 1, "{:?}", frame.stats);
    assert_eq!(frame.stats.emoji_skipped, 0);
    assert!(
        frame
            .pixmap
            .data_as_u8_slice()
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[1] == 200 && p[3] == 255)
    );
    renderer.system_fonts = false;
    assert_eq!(renderer.emoji_counts(&doc).emoji_skipped, 1);
}
