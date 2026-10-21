use super::*;
use image::ImageEncoder as _;

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
    let resolved = resolve(&db, '❤', 30).unwrap();
    assert!(matches!(
        resolved.glyph(50),
        Some(Glyph::Bitmap { ppem: 64, .. })
    ));
    let Some(Glyph::Bitmap {
        image: pixmap,
        ppem: 16,
        ..
    }) = resolved.glyph(30)
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

#[test]
fn malformed_or_oversized_strikes_are_not_decoded() {
    assert!(decode_png(b"not a PNG").is_none());
    assert!(decode_png(&png(1025, 1)).is_none());
    let mut db = Database::new();
    db.load_font_data(crate::text_test_font::colour_emoji(b"not a PNG"));
    assert!(resolve(&db, '❤', 30).is_none());
}
