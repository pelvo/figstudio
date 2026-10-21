use super::*;
use std::sync::Arc;

#[test]
fn late_pre_font_tiles_cannot_replace_current_pixels_or_clear_their_request() {
    let doc =
        Arc::new(figstudio_fig::Document::open(crate::ui_tests::fixture("structure.fig")).unwrap());
    let mut tab = Tab::new(1, "synthetic.fig".into(), doc, None, Duration::ZERO);
    let key = tiles::Key {
        doc: tab.doc.instance,
        page: tab.page,
        level: 0,
        x: 0,
        y: 0,
    };
    let done = |images| RenderDone {
        tab: 1,
        key,
        images,
        pixels: Pixels::Cpu {
            size: [1, 1],
            rgba: vec![255, 0, 0, 255],
        },
        stats: Stats::default(),
        elapsed: Duration::ZERO,
    };
    let ctx = egui::Context::default();
    let mut tiles = Tiles::default();
    tiles.fonts_changed(2);
    tab.tile_requests.push(key);
    tiles.accept(&ctx, &mut tab, done(1));
    assert_eq!(tab.tile_requests, vec![key]);
    assert!(tiles.cache.get(&key).is_none());
    tiles.accept(&ctx, &mut tab, done(2));
    assert!(tab.tile_requests.is_empty());
    assert!(tiles.cache.get(&key).is_some());
    tab.tile_requests.push(key);
    tiles.accept(&ctx, &mut tab, done(1));
    assert_eq!(tab.tile_requests, vec![key]);
    assert!(tiles.cache.get(&key).is_some());
    assert_eq!(tab.tiles_received, vec![(key, 1), (key, 2), (key, 1)]);
}
