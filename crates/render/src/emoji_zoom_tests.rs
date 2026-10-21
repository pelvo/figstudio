//! Native zoom work must depend on the font's strikes, not the number of frames.
use fontdb::Database;
use image::ImageEncoder as _;
use vello_cpu::kurbo::Affine;

use crate::{Renderer, Viewport, emoji_font::DECODES, emoji_tests::document, text::SystemFonts};

#[test]
fn zoom_sweep_decodes_each_native_strike_once() {
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            &[0, 200, 40, 255].repeat(64 * 64),
            64,
            64,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    let mut db = Database::new();
    db.load_font_data(crate::text_test_font::colour_emoji(&png));
    let mut renderer = Renderer {
        fonts: SystemFonts::with_database(db),
        ..Renderer::new()
    };
    let doc = document(&[0x2764]);
    DECODES.set(0);
    let started = std::time::Instant::now();
    // Sweep back again to detect pixel-size keys as well as whole-cache eviction.
    for step in (0..50).chain((0..50).rev()) {
        let zoom = 0.25 + f64::from(step) * 0.035;
        let frame = renderer.render(
            &doc,
            figstudio_fig::Guid(99, 1),
            &Viewport {
                width: 160,
                height: 160,
                transform: Affine::scale(zoom),
                background: None,
            },
        );
        assert_eq!(frame.stats.emoji_skipped, 0);
        assert!(frame.pixmap.data_as_u8_slice().iter().any(|v| *v != 0));
    }
    let decodes = DECODES.get();
    eprintln!(
        "zoom sweep: 100 frames, {decodes} PNG decodes, {:?}",
        started.elapsed()
    );
    assert!(decodes <= 2, "two strikes must serve every zoom: {decodes}");
}
