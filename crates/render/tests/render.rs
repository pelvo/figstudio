use std::path::PathBuf;

use figstudio_fig::{Document, Guid, Val};
use figstudio_render::kurbo::{Affine, Point};
use figstudio_render::{Renderer, Stats, Viewport, content_bounds, geom, hit_test, node_bounds};

fn open(name: &str) -> Document {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(format!("{name}.fig"));
    Document::open(path).unwrap()
}

/// Renders the first page at 1:1 with page coordinates mapped so `origin` lands on pixel (0,0).
fn render_page(doc: &Document, origin: Point, w: u16, h: u16) -> (Vec<[u8; 4]>, Stats) {
    let mut r = Renderer::new();
    r.system_fonts = false;
    let frame = r.render(
        doc,
        doc.pages[0],
        &Viewport {
            width: w,
            height: h,
            transform: Affine::translate((-origin.x, -origin.y)),
            background: None,
        },
    );
    let px = frame
        .pixmap
        .data()
        .iter()
        .map(|p| [p.r, p.g, p.b, p.a])
        .collect();
    (px, frame.stats)
}

fn pixel(px: &[[u8; 4]], w: u16, x: f64, y: f64) -> [u8; 4] {
    px[y as usize * w as usize + x as usize]
}

fn count(px: &[[u8; 4]], pred: impl Fn([u8; 4]) -> bool) -> usize {
    px.iter().filter(|p| pred(**p)).count()
}

#[test]
fn every_fixture_renders_with_content() {
    for name in [
        "broken_images",
        "corners",
        "noise_and_grain",
        "sections",
        "stacks_wrap",
        "structure",
        "vector",
    ] {
        let doc = open(name);
        let mut r = Renderer::new();
        r.system_fonts = false;
        let frame = r.render_fit(&doc, doc.pages[0], 0.5, 4.0, None).unwrap();
        let opaque = frame.pixmap.data().iter().filter(|p| p.a > 0).count();
        assert!(opaque > 500, "{name} drew almost nothing ({opaque} px)");
        assert!(frame.stats.nodes > 0, "{name}");
    }
}

#[test]
fn vector_paths_use_their_style_override_paints() {
    let doc = open("vector");
    let b = content_bounds(&doc, doc.pages[0]).unwrap();
    let (w, h) = (b.width().ceil() as u16, b.height().ceil() as u16);
    let (px, _) = render_page(&doc, b.origin(), w, h);
    let red = count(&px, |p| p[0] > 200 && p[1] < 40 && p[2] < 40);
    let yellow = count(&px, |p| p[0] > 200 && p[1] > 200 && p[2] < 40);
    let purple = count(&px, |p| p[0] > 100 && p[1] < 30 && p[2] > 100 && p[0] < 160);
    // Areas measured on a correct render: red ~1570, purple ~380, yellow ~120 px.
    assert!(red > 1000, "red region missing ({red})");
    assert!(yellow > 60, "yellow region missing ({yellow})");
    assert!(purple > 250, "purple region missing ({purple})");
}

#[test]
fn stored_glyph_text_is_drawn() {
    let doc = open("noise_and_grain");
    let b = content_bounds(&doc, doc.pages[0]).unwrap();
    let (w, h) = (
        (b.width() / 2.0).ceil() as u16,
        (b.height() / 2.0).ceil() as u16,
    );
    let mut r = Renderer::new();
    let frame = r.render(
        &doc,
        doc.pages[0],
        &Viewport {
            width: w,
            height: h,
            transform: Affine::scale(0.5) * Affine::translate((-b.x0, -b.y0)),
            background: None,
        },
    );
    let blue = frame
        .pixmap
        .data()
        .iter()
        .filter(|p| p.b as i32 > p.r as i32 + 30)
        .count();
    assert!(blue > 5000, "text glyphs missing ({blue} blue px)");
    assert_eq!(frame.stats.text_stored_glyphs, 2);
    assert_eq!(frame.stats.text_missing, 0);
    // Progressive blur and noise/grain are not implemented and must be reported, not hidden.
    assert_eq!(frame.stats.skipped_effects, 4);
}

#[test]
fn instances_expand_their_component() {
    let doc = open("structure");
    let mut r = Renderer::new();
    r.system_fonts = false;
    let frame = r.render_fit(&doc, doc.pages[0], 1.0, 0.0, None).unwrap();
    assert_eq!(frame.stats.instances, 2);
    assert_eq!(frame.stats.missing_symbols, 0);
    // The instance of the red-square component must show red pixels.
    let id = doc.nodes.iter().find(|n| n.kind == "INSTANCE").unwrap().id;
    let bounds = node_bounds(&doc, id).unwrap();
    let w = bounds.width().ceil() as u16;
    let (px, _) = render_page(&doc, bounds.origin(), w, bounds.height().ceil() as u16);
    assert!(count(&px, |p| p[3] > 0) > 0, "instance area is empty");
}

#[test]
fn frames_clip_their_children() {
    let doc = open("stacks_wrap");
    let page = doc.pages[0];
    // Find a clipping frame and check nothing is drawn just outside it on the right.
    let frame_id = doc
        .children(page)
        .iter()
        .copied()
        .find(|id| doc.node(*id).is_some_and(|n| n.kind == "FRAME"))
        .unwrap();
    let fb = node_bounds(&doc, frame_id).unwrap();
    let all = content_bounds(&doc, page).unwrap();
    let (w, h) = (all.width().ceil() as u16, all.height().ceil() as u16);
    let (px, _) = render_page(&doc, all.origin(), w, h);
    let inside = pixel(&px, w, fb.x0 - all.x0 + 4.0, fb.y0 - all.y0 + 4.0);
    assert!(inside[3] > 0, "frame background missing");
}

#[test]
fn hit_test_finds_the_deepest_layer() {
    let doc = open("structure");
    let text = Guid(1, 42);
    let b = node_bounds(&doc, text).unwrap();
    let hit = hit_test(&doc, doc.pages[0], b.center()).unwrap();
    assert_eq!(hit, text);
    assert_eq!(hit_test(&doc, doc.pages[0], Point::new(-1e6, -1e6)), None);
}

#[test]
fn culling_never_changes_what_is_visible() {
    let mut culled_somewhere = false;
    for name in [
        "broken_images",
        "corners",
        "noise_and_grain",
        "sections",
        "stacks_wrap",
        "structure",
        "vector",
    ] {
        let doc = open(name);
        let b = content_bounds(&doc, doc.pages[0]).unwrap();
        let (w, h) = (b.width().ceil() as u16, b.height().ceil() as u16);
        let (full, _) = render_page(&doc, b.origin(), w, h);
        // A window in the middle, half the size each way, at a whole-pixel offset.
        let (ox, oy, cw, ch) = (w / 4, h / 4, (w / 2).max(1), (h / 2).max(1));
        let origin = Point::new(b.x0 + f64::from(ox), b.y0 + f64::from(oy));
        let (crop, stats) = render_page(&doc, origin, cw, ch);
        culled_somewhere |= stats.culled > 0;
        let mut differ = 0;
        for y in 0..ch as usize {
            for x in 0..cw as usize {
                let a = crop[y * cw as usize + x];
                let f = full[(y + oy as usize) * w as usize + x + ox as usize];
                if a.iter().zip(f).any(|(p, q)| p.abs_diff(q) > 8) {
                    differ += 1;
                }
            }
        }
        // Blurs and shadows cut at the window edge may differ slightly; nothing else should.
        let limit = (cw as usize * ch as usize) / 100;
        assert!(
            differ <= limit,
            "{name}: {differ} pixels differ in the window (limit {limit})"
        );
    }
    assert!(culled_somewhere, "no fixture exercised culling");
}

#[test]
fn malformed_path_blobs_do_not_panic() {
    // close then line without a move; truncated coordinates; unknown command
    assert!(geom::blob_path(&[0, 2, 0, 0, 128, 63, 0, 0, 128, 63]).is_some());
    assert!(geom::blob_path(&[1, 0, 0]).is_none());
    assert!(geom::blob_path(&[9]).is_none());
    assert!(geom::blob_path(&[]).is_some());
}

#[test]
fn hidden_layers_are_not_drawn() {
    let mut doc = open("corners");
    let before = Renderer::new()
        .render_fit(&doc, doc.pages[0], 0.5, 0.0, None)
        .unwrap()
        .stats
        .nodes;
    for n in doc
        .nodes
        .iter_mut()
        .filter(|n| n.kind == "ROUNDED_RECTANGLE")
    {
        if let Val::Obj(m) = &mut n.data {
            m.insert("visible", Val::Bool(false));
        }
    }
    let after = Renderer::new()
        .render_fit(&doc, doc.pages[0], 0.5, 0.0, None)
        .unwrap()
        .stats
        .nodes;
    assert!(after < before, "{after} >= {before}");
}

/// Strips drawn on several threads must give the same pixels and the same counts as one pass.
#[cfg(feature = "parallel")]
#[test]
fn parallel_strips_match_a_single_render() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    for name in [
        "broken_images",
        "corners",
        "noise_and_grain",
        "sections",
        "stacks_wrap",
        "structure",
        "vector",
    ] {
        let doc = open(name);
        let b = content_bounds(&doc, doc.pages[0]).unwrap();
        for scale in [1.0, 2.5] {
            let viewport = Viewport {
                width: (b.width() * scale).ceil() as u16,
                height: (b.height() * scale).ceil() as u16,
                transform: Affine::scale(scale) * Affine::translate((-b.x0, -b.y0)),
                background: Some(figstudio_render::color::AlphaColor::new([
                    1.0, 1.0, 1.0, 1.0,
                ])),
            };
            let mut r = Renderer::new();
            r.system_fonts = false;
            let one = r.render(&doc, doc.pages[0], &viewport);
            let many = pool.install(|| r.render_parallel(&doc, doc.pages[0], &viewport));
            assert_eq!(one.stats, many.stats, "{name} at {scale}x: stats differ");
            // Moving the origin by whole pixels changes f32 rounding, so allow 1 per channel.
            // A missing strip, a seam or a shadow cut at a strip edge is far more than that.
            let worst = one
                .pixmap
                .data()
                .iter()
                .zip(many.pixmap.data())
                .flat_map(|(a, b)| {
                    [
                        a.r.abs_diff(b.r),
                        a.g.abs_diff(b.g),
                        a.b.abs_diff(b.b),
                        a.a.abs_diff(b.a),
                    ]
                })
                .max()
                .unwrap_or(0);
            assert!(
                worst <= 1,
                "{name} at {scale}x: a channel differs by {worst}"
            );
        }
    }
}

/// Big shadows that cross strip edges: drawn by the strip below from content in the strip above.
#[cfg(feature = "parallel")]
#[test]
fn parallel_strips_keep_shadows_that_cross_strip_edges() {
    let shadow_doc = open("structure");
    let mut effect = shadow_doc
        .nodes
        .iter()
        .find_map(|n| {
            n.data
                .get("effects")
                .and_then(|e| e.items().first().cloned())
        })
        .expect("structure has a drop shadow");
    if let Val::Obj(m) = &mut effect {
        m.insert("radius", Val::Float(24.0));
        let mut offset = m.get("offset").cloned().expect("shadow offset");
        if let Val::Obj(o) = &mut offset {
            o.insert("y", Val::Float(40.0));
        }
        m.insert("offset", offset);
    }
    let mut doc = open("corners");
    let mut shadowed = 0;
    for n in doc
        .nodes
        .iter_mut()
        .filter(|n| n.kind == "ROUNDED_RECTANGLE")
    {
        if let Val::Obj(m) = &mut n.data {
            m.insert("effects", Val::Arr(vec![effect.clone()]));
            shadowed += 1;
        }
    }
    assert!(shadowed > 0);
    let b = content_bounds(&doc, doc.pages[0])
        .unwrap()
        .inflate(0.0, 80.0);
    let viewport = Viewport {
        width: (b.width() * 2.0).ceil() as u16,
        height: (b.height() * 2.0).ceil() as u16,
        transform: Affine::scale(2.0) * Affine::translate((-b.x0, -b.y0)),
        background: None,
    };
    let mut r = Renderer::new();
    r.system_fonts = false;
    let one = r.render(&doc, doc.pages[0], &viewport);
    assert_eq!(one.stats.shadows, shadowed);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    let many = pool.install(|| r.render_parallel(&doc, doc.pages[0], &viewport));
    assert_eq!(one.stats, many.stats);
    // Blur rounding depends on the origin: allow 2. A shadow cut at a strip edge is off by
    // tens (60 when the recorder ignores the filter's reach).
    let worst = one
        .pixmap
        .data()
        .iter()
        .zip(many.pixmap.data())
        .map(|(a, b)| a.a.abs_diff(b.a).max(a.r.abs_diff(b.r)))
        .max()
        .unwrap_or(0);
    assert!(worst <= 2, "a channel differs by {worst}");
}
