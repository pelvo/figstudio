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
