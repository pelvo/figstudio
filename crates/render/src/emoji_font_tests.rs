use super::*;
use crate::emoji::Glyph;
use image::ImageEncoder as _;
#[path = "emoji_bitmap_test_font.rs"]
mod bitmap_font;

fn png(width: u32, height: u32) -> Vec<u8> {
    let rgba: Vec<u8> = (0..width * height)
        .flat_map(|i| [i as u8, 80, 200, 255])
        .collect();
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(&rgba, width, height, image::ExtendedColorType::Rgba8)
        .unwrap();
    out
}

#[test]
fn synthetic_sbix_decodes_colour_and_chooses_the_nearest_strike() {
    let data = crate::text_test_font::colour_emoji(&png(2, 2));
    let face = Face::parse(&data, 0).unwrap();
    let id = face.glyph_index('❤').unwrap();
    assert_eq!(strikes(&face, id).len(), 2);
    let mut db = Database::new();
    db.load_font_data(data);
    let mut resolved = resolve(&db, '❤', 30).unwrap();
    assert!(matches!(
        resolved.glyph(&db, 50),
        Some(Glyph::Bitmap { ppem: 64, .. })
    ));
    let Some(Glyph::Bitmap {
        image: pixmap,
        ppem: 16,
        ..
    }) = resolved.glyph(&db, 30)
    else {
        panic!("missing sbix colour glyph")
    };
    assert_eq!(pixmap.width(), 2);
    assert_eq!(
        pixmap.data_as_u8_slice(),
        &[
            0, 80, 200, 255, 1, 80, 200, 255, 2, 80, 200, 255, 3, 80, 200, 255
        ]
    );
    assert!(resolve(&db, 'A', 30).is_none());
}

fn embedded_bitmap_tables(tags: (&[u8; 4], &[u8; 4])) {
    let good = png(2, 2);
    for (ppems, corrupt) in [([16, 64], false), ([16, 16], true)] {
        let data = bitmap_font::font(
            tags,
            [if corrupt { b"bad PNG" } else { &good }, &good],
            ppems,
        );
        let face = Face::parse(&data, 0).unwrap();
        let id = face.glyph_index('❤').unwrap();
        let tables = face.tables();
        let table = tables.cbdt.or(tables.bdat).or(tables.ebdt).unwrap();
        assert!(
            table.get(id, 16).is_some(),
            "fixture reaches the font library"
        );
        let mut db = Database::new();
        db.load_font_data(data);
        let mut resolved =
            resolve(&db, '❤', 16).unwrap_or_else(|| panic!("missing {tags:?}, {ppems:?}"));
        for requested in [16, 64] {
            let Some(Glyph::Bitmap {
                image, ppem, x, y, ..
            }) = resolved.glyph(&db, requested)
            else {
                panic!("must decode colour")
            };
            assert_eq!(ppem, if corrupt { 16 } else { requested });
            assert_eq!((x, y), (1.0, 1.0));
            assert_eq!((image.width(), image.height()), (2, 2));
            assert_eq!(&image.data_as_u8_slice()[..4], &[0, 80, 200, 255]);
        }
    }
}

#[test]
fn cbdt_walk_reaches_a_usable_duplicate_ppem_record() {
    embedded_bitmap_tables((b"CBDT", b"CBLC"));
}

#[test]
fn bdat_png_strikes_are_read() {
    embedded_bitmap_tables((b"bdat", b"bloc"));
}

#[test]
fn ebdt_png_strikes_are_read() {
    embedded_bitmap_tables((b"EBDT", b"EBLC"));
}

#[test]
fn malformed_or_oversized_strikes_are_not_decoded() {
    assert!(decode_png(b"not a PNG").is_none());
    assert!(decode_png(&png(1025, 1)).is_none());
    let mut db = Database::new();
    db.load_font_data(crate::text_test_font::colour_emoji(b"not a PNG"));
    assert!(resolve(&db, '❤', 30).is_none());
}
