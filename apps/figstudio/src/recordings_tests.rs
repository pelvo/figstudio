//! The render worker walks a page once per level and batch, replays every tile from that
//! recording with the pixels of a direct render, and falls back (counted) when it cannot.

use std::collections::VecDeque;
use std::sync::Arc;

use figstudio_fig::Document;

use super::*;

fn job(doc: &Arc<Document>, tiles: &[Key], images: u64) -> RenderJob {
    RenderJob {
        tab: 1,
        doc: doc.clone(),
        source: None,
        root: doc.pages[0],
        background: Some(figstudio_render::page_background(doc, doc.pages[0])),
        images,
        tiles: tiles.iter().copied().collect::<VecDeque<_>>(),
        received: Vec::new(),
    }
}

fn structure() -> Arc<Document> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/structure.fig"
    );
    Arc::new(Document::open(path).unwrap())
}

fn key(doc: &Document, level: i32, x: i32, y: i32) -> Key {
    Key {
        doc: doc.instance,
        page: doc.pages[0],
        level,
        x,
        y,
    }
}

/// The tiles of `level` holding the fixture's content, plus a ring.
fn tiles_of(doc: &Document, level: i32) -> Vec<Key> {
    let b = figstudio_render::content_bounds(doc, doc.pages[0]).unwrap();
    let range = tiles::Range::covering(level, b).grow(1);
    range.by_distance(doc.instance, doc.pages[0])
}

fn renderer() -> Renderer {
    let mut r = Renderer::new();
    r.system_fonts = false;
    r
}

#[test]
fn tiles_of_a_level_share_one_walk_and_match_direct_renders() {
    let doc = structure();
    for level in [-1, 0, 1, 3] {
        let keys = tiles_of(&doc, level);
        assert!(keys.len() > 1, "level {level} has several tiles");
        let job = job(&doc, &keys, 0);
        let mut recordings = Recordings::default();
        let mut r = renderer();
        for k in &keys {
            let frame = recordings.draw(&mut r, &job, k);
            assert_eq!(frame.stats.unrecorded_tiles, 0);
            let vp = Viewport {
                width: TEXTURE,
                height: TEXTURE,
                transform: k.transform(),
                background: job.background,
            };
            let direct = renderer().render(&doc, doc.pages[0], &vp);
            let worst = direct
                .pixmap
                .data_as_u8_slice()
                .iter()
                .zip(frame.pixmap.data_as_u8_slice())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            assert!(worst <= 1, "level {level} tile {k:?} differs by {worst}");
        }
        assert_eq!(
            recordings.walks, 1,
            "level {level}: one walk for every tile"
        );
    }
}

#[test]
fn a_new_level_page_or_more_images_record_again() {
    let doc = structure();
    let (a, b) = (tiles_of(&doc, 0), tiles_of(&doc, 2));
    let mut recordings = Recordings::default();
    let mut r = renderer();
    recordings.draw(&mut r, &job(&doc, &a, 0), &a[0]);
    recordings.draw(&mut r, &job(&doc, &b, 0), &b[0]);
    assert_eq!(recordings.walks, 2, "another level");
    recordings.draw(&mut r, &job(&doc, &a, 0), &a[1]);
    assert_eq!(recordings.walks, 2, "the first level is still kept");
    // No image of this fixture is loading, so arrivals elsewhere do not redo it.
    recordings.draw(&mut r, &job(&doc, &a, 5), &a[0]);
    assert_eq!(recordings.walks, 2, "nothing was loading");
    // A tile outside the batch's area: record its own batch.
    let far = key(&doc, 0, 50, 50);
    recordings.draw(&mut r, &job(&doc, &[far], 5), &far);
    assert_eq!(recordings.walks, 3, "outside the recorded area");
    recordings.draw(&mut r, &job(&doc, &a, 5), &a[0]);
    assert_eq!(
        recordings.walks, 4,
        "a recording replaced the level's old one"
    );
    // One tile beyond the batch (a short pan) still replays from it.
    let r0 = tiles::Range::covering(
        0,
        figstudio_render::content_bounds(&doc, doc.pages[0]).unwrap(),
    );
    let beyond = key(&doc, 0, r0.x1 + 1, r0.y0);
    recordings.draw(&mut r, &job(&doc, &[beyond], 5), &beyond);
    assert_eq!(
        recordings.walks, 4,
        "the recorded area has a ring beyond the batch"
    );
    recordings.forget(doc.instance);
    recordings.draw(&mut r, &job(&doc, &a, 5), &a[0]);
    assert_eq!(recordings.walks, 5, "forgotten with its document");
}

#[test]
fn an_over_limit_recording_renders_directly_and_is_counted() {
    let doc = structure();
    let keys = tiles_of(&doc, 0);
    let job = job(&doc, &keys, 0);
    let mut recordings = Recordings {
        limit: 1,
        ..Recordings::default()
    };
    let mut r = renderer();
    let frame = recordings.draw(&mut r, &job, &keys[0]);
    assert_eq!(frame.stats.unrecorded_tiles, 1);
    assert!(frame.stats.nodes > 0, "drawn directly");
    recordings.draw(&mut r, &job, &keys[1]);
    assert_eq!(recordings.walks, 1, "not recorded again for each tile");
}

/// A recording drawn while images loaded is redone once more images arrive; its walk's image
/// requests reach the frame (so they are read), and later tiles carry none.
#[test]
fn placeholders_are_recorded_again_when_images_arrive() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/structure.fig"
    );
    let doc = Arc::new(Document::open_lazy(path).unwrap());
    let keys = tiles_of(&doc, 0);
    let mut recordings = Recordings::default();
    let mut r = renderer();
    let first = recordings.draw(&mut r, &job(&doc, &keys, 0), &keys[0]);
    assert!(
        first.stats.images_loading > 0,
        "the fixture's images are unread"
    );
    assert!(
        !first.wanted.is_empty(),
        "the walk's requests are passed on"
    );
    let second = recordings.draw(&mut r, &job(&doc, &keys, 0), &keys[1]);
    assert!(second.wanted.is_empty());
    assert_eq!(recordings.walks, 1);
    recordings.draw(&mut r, &job(&doc, &keys, 1), &keys[1]);
    assert_eq!(recordings.walks, 2, "an image arrived");
}

/// A fill with 20,000 gradient stops makes the recording too big for a limit the plain page
/// fits in many times over: its tiles render directly and are counted.
#[test]
fn a_huge_gradient_falls_back_counted() {
    let plain = structure();
    let keys = tiles_of(&plain, 0);
    let mut recordings = Recordings::default();
    recordings.draw(&mut renderer(), &job(&plain, &keys, 0), &keys[0]);
    let limit = 4 * recordings.entries[0].rec.as_ref().unwrap().bytes();
    let stop = |i: u32| {
        let mut o = figstudio_fig::Obj::new();
        o.insert(
            "position",
            figstudio_fig::Val::Float(f64::from(i) / 20_000.0),
        );
        let mut c = figstudio_fig::Obj::new();
        c.insert("a", figstudio_fig::Val::Float(1.0));
        o.insert("color", figstudio_fig::Val::Obj(c));
        figstudio_fig::Val::Obj(o)
    };
    let mut paint = figstudio_fig::Obj::new();
    paint.insert("type", figstudio_fig::Val::Enum("GRADIENT_LINEAR"));
    paint.insert("visible", figstudio_fig::Val::Bool(true));
    paint.insert(
        "stops",
        figstudio_fig::Val::Arr((0..20_000).map(stop).collect()),
    );
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/structure.fig"
    );
    let mut doc = Document::open(path).unwrap();
    let mut nodes = doc.nodes.clone();
    let target = doc.children(doc.pages[0])[0];
    let rect = nodes.iter_mut().find(|n| n.id == target).unwrap();
    rect.data.set(
        "fillPaints",
        figstudio_fig::Val::Arr(vec![figstudio_fig::Val::Obj(paint)]),
    );
    doc.set_nodes(nodes).unwrap();
    let doc = Arc::new(doc);
    // Same tile positions; keys carry the document instance.
    let keys: Vec<Key> = keys
        .iter()
        .map(|k| Key {
            doc: doc.instance,
            ..*k
        })
        .collect();
    for (d, counted) in [(&plain, 0), (&doc, 1)] {
        let mut recordings = Recordings {
            limit,
            ..Recordings::default()
        };
        let key = Key {
            doc: d.instance,
            ..keys[0]
        };
        let frame = recordings.draw(&mut renderer(), &job(d, &keys, 0), &key);
        assert_eq!(frame.stats.unrecorded_tiles, counted);
    }
}
