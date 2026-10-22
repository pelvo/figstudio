use super::*;
use crate::{
    emoji_font::{DECODES, RESOLVES},
    text_test_font,
};
use image::ImageEncoder as _;

fn database(font: Vec<u8>) -> Database {
    let mut db = Database::new();
    db.load_font_data(font);
    db
}

fn png() -> Vec<u8> {
    let mut data = Vec::new();
    image::codecs::png::PngEncoder::new(&mut data)
        .write_image(
            &[20, 200, 80, 255].repeat(4),
            2,
            2,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    data
}

#[test]
fn large_strike_catalog_is_bounded_and_same_zoom_does_not_decode_again() {
    let mut data = png();
    data.resize(3 * 1024 * 1024, 0); // Valid PNG with trailing padding, eight legal <=4-MiB strikes.
    let db = database(text_test_font::emoji_many_strikes(
        &data,
        &[16, 24, 32, 40, 48, 56, 64, 72],
    ));
    let prepared = crate::emoji_font::resolve(&db, '❤', 40).unwrap().bytes();
    let mut fonts = SystemFonts::with_database(db);
    DECODES.set(0);
    let started = std::time::Instant::now();
    for _ in 0..20 {
        assert!(fonts.emoji('❤', 40).is_some());
    }
    let decodes = DECODES.get();
    eprintln!(
        "8 x 3-MiB strikes, 20 frames: prepared={prepared} bytes, retained={} bytes, decodes={decodes}, elapsed={:?}",
        fonts.emoji_bytes,
        started.elapsed()
    );
    assert!(prepared <= 16 * 1024 * 1024 && decodes == 1 && fonts.emojis.contains_key(&'❤'));
}

#[test]
fn native_cache_hits_refresh_lru_recency() {
    let mut fonts = SystemFonts::with_database(database(text_test_font::colour_emoji(&png())));
    RESOLVES.set(0);
    assert!(fonts.emoji('❤', 40).is_some());
    for n in 0..63 {
        fonts.emoji(char::from_u32(0x400 + n).unwrap(), 40);
    }
    assert_eq!(RESOLVES.get(), 64);
    for _ in 0..10 {
        assert!(fonts.emoji('❤', 40).is_some());
    }
    fonts.emoji('A', 40);
    assert!(fonts.emoji('❤', 40).is_some());
    eprintln!(
        "64 cached characters + refreshed first + 65th: {} resolves",
        RESOLVES.get()
    );
    assert_eq!(
        RESOLVES.get(),
        65,
        "the refreshed glyph must survive eviction"
    );
}
