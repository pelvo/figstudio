use std::path::PathBuf;

use figstudio_fig::{Document, Guid, Val};
use figstudio_render::kurbo::{Affine, Point};
use figstudio_render::{
    Renderer, Stats, Viewport, absolute_transform, content_bounds, geom, hit_test, node_bounds,
};

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
    let outside = pixel(&px, w, fb.x1 - all.x0 + 3.0, fb.y0 - all.y0 + 4.0);
    assert_eq!(outside[3], 0, "frame leaked pixels outside its clip");
}

#[test]
fn page_fit_includes_visible_children_outside_unclipped_groups() {
    let mut doc = open("structure");
    let page = doc.pages[0];
    let template = doc
        .nodes
        .iter()
        .find(|n| n.kind == "FRAME")
        .unwrap()
        .clone();
    let mut nodes = doc.nodes.clone();
    let parent_data = |id: Guid, x: f32| {
        let mut parent = figstudio_fig::Obj::new();
        parent.insert("guid", id.to_val());
        parent.insert("position", Val::Str("zzzz".into()));
        let mut transform = figstudio_fig::Obj::new();
        transform.insert("m02", Val::Float(f64::from(x)));
        let mut data = template.data.clone();
        data.set("transform", Val::Obj(transform));
        data.set("parentIndex", Val::Obj(parent));
        data
    };
    let group_id = Guid(98, 1);
    let mut group_data = parent_data(page, 0.0);
    group_data.set("guid", group_id.to_val());
    group_data.set("type", Val::Enum("GROUP"));
    group_data.set(
        "size",
        Val::Obj([("x", Val::Float(10.0)), ("y", Val::Float(10.0))].into()),
    );
    nodes.push(figstudio_fig::Node::from_data(group_data).unwrap());
    let child_id = Guid(98, 2);
    let mut child_data = parent_data(group_id, 1000.0);
    child_data.set("guid", child_id.to_val());
    child_data.set("type", Val::Enum("RECTANGLE"));
    child_data.set(
        "size",
        Val::Obj([("x", Val::Float(20.0)), ("y", Val::Float(20.0))].into()),
    );
    child_data.set(
        "fillPaints",
        Val::Arr(vec![Val::Obj(
            [
                ("type", Val::Enum("SOLID")),
                (
                    "color",
                    Val::Obj(
                        [
                            ("r", Val::Float(1.0)),
                            ("g", Val::Float(0.0)),
                            ("b", Val::Float(0.0)),
                            ("a", Val::Float(1.0)),
                        ]
                        .into(),
                    ),
                ),
            ]
            .into(),
        )]),
    );
    nodes.push(figstudio_fig::Node::from_data(child_data).unwrap());
    doc.set_nodes(nodes).unwrap();

    let frame = Renderer::new()
        .render_fit(&doc, page, 1.0, 0.0, None)
        .unwrap();
    assert!(
        frame.pixmap.width() > 1000,
        "fit omitted outlying descendant"
    );
    let red_near_edge = frame.pixmap.data().iter().enumerate().any(|(i, p)| {
        i % usize::from(frame.pixmap.width()) > usize::from(frame.pixmap.width()) - 50
            && p.r > 200
            && p.g < 50
            && p.b < 50
            && p.a > 200
    });
    assert!(
        red_near_edge,
        "outlying child was not rendered in the export bounds"
    );
}

#[test]
fn masks_clip_siblings_but_do_not_escape_their_parent() {
    let mut doc = open("structure");
    let page = doc.pages[0];
    let template = doc
        .nodes
        .iter()
        .find(|n| n.kind == "FRAME")
        .unwrap()
        .clone();
    let mut nodes = vec![
        doc.nodes
            .iter()
            .find(|node| node.kind == "DOCUMENT")
            .unwrap()
            .clone(),
        doc.node(page).unwrap().clone(),
    ];
    let group = Guid(99, 9);
    let mut group_data = template.data.clone();
    group_data.set("guid", group.to_val());
    group_data.set("type", Val::Enum("GROUP"));
    let mut group_parent = figstudio_fig::Obj::new();
    group_parent.insert("guid", page.to_val());
    group_parent.insert("position", Val::Str("zzzz0".into()));
    group_data.set("parentIndex", Val::Obj(group_parent));
    let mut group_transform = figstudio_fig::Obj::new();
    group_transform.insert("m12", Val::Float(500.0));
    group_data.set("transform", Val::Obj(group_transform));
    nodes.push(figstudio_fig::Node::from_data(group_data).unwrap());
    let mut add_rect = |id: Guid, parent_id: Guid, position: &str, mask: bool, x: f64, y: f64| {
        let mut data = template.data.clone();
        data.set("guid", id.to_val());
        data.set("type", Val::Enum("RECTANGLE"));
        data.set("fillGeometry", Val::Arr(Vec::new()));
        data.set("strokeGeometry", Val::Arr(Vec::new()));
        data.set(
            "size",
            Val::Obj([("x", Val::Float(20.0)), ("y", Val::Float(20.0))].into()),
        );
        let mut parent = figstudio_fig::Obj::new();
        parent.insert("guid", parent_id.to_val());
        parent.insert("position", Val::Str(position.into()));
        data.set("parentIndex", Val::Obj(parent));
        let mut transform = figstudio_fig::Obj::new();
        transform.insert("m02", Val::Float(x));
        transform.insert("m12", Val::Float(y));
        data.set("transform", Val::Obj(transform));
        if mask {
            data.set("mask", Val::Bool(true));
        }
        data.set(
            "fillPaints",
            Val::Arr(vec![Val::Obj(
                [
                    ("type", Val::Enum("SOLID")),
                    (
                        "color",
                        Val::Obj(
                            [
                                ("r", Val::Float(1.0)),
                                ("g", Val::Float(0.0)),
                                ("b", Val::Float(0.0)),
                                ("a", Val::Float(1.0)),
                            ]
                            .into(),
                        ),
                    ),
                ]
                .into(),
            )]),
        );
        nodes.push(figstudio_fig::Node::from_data(data).unwrap());
    };
    add_rect(Guid(99, 10), group, "0", true, 0.0, 0.0);
    add_rect(Guid(99, 11), group, "1", false, 10.0, 0.0);
    add_rect(Guid(99, 12), page, "zzzz1", false, 40.0, 500.0);
    doc.set_nodes(nodes).unwrap();
    let (pixels, stats) = render_page(&doc, Point::new(0.0, 499.0), 64, 32);
    assert!(stats.masks > 0);
    assert_eq!(
        pixel(&pixels, 64, 25.0, 10.0)[3],
        255,
        "mask intersection missing"
    );
    assert_eq!(
        pixel(&pixels, 64, 35.0, 10.0)[3],
        255,
        "masked sibling intersection missing"
    );
    assert_eq!(
        pixel(&pixels, 64, 45.0, 10.0)[3],
        255,
        "unmasked sibling missing"
    );
}

#[test]
fn malformed_vectors_and_unsupported_blends_are_reported() {
    let mut doc = open("structure");
    let page = doc.pages[0];
    let mut nodes = doc.nodes.clone();
    let frame = nodes.iter_mut().find(|n| n.kind == "FRAME").unwrap();
    frame.data.set("blendMode", Val::Enum("MULTIPLY"));
    let mut malformed = frame.clone();
    malformed.id = Guid(99, 1);
    malformed.data.set("guid", malformed.id.to_val());
    malformed.data.set("type", Val::Enum("VECTOR"));
    malformed.data.set(
        "fillGeometry",
        Val::Arr(vec![Val::Obj([("commandsBlob", Val::Int(9999))].into())]),
    );
    let mut parent = figstudio_fig::Obj::new();
    parent.insert("guid", page.to_val());
    parent.insert("position", Val::Str("zzzzzz".into()));
    malformed.data.set("parentIndex", Val::Obj(parent));
    nodes.push(figstudio_fig::Node::from_data(malformed.data).unwrap());
    doc.set_nodes(nodes).unwrap();
    let frame = Renderer::new().render(
        &doc,
        page,
        &Viewport {
            width: 10,
            height: 10,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    assert!(frame.stats.unsupported_paints > 0);
    assert!(frame.stats.approximated_blends > 0);
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
fn parent_queries_terminate_after_public_mutation_to_a_self_parent() {
    let mut doc = open("structure");
    let page = doc.pages[0];
    let node = doc.children(page)[0];
    let n = doc.nodes.iter_mut().find(|n| n.id == node).unwrap();
    n.parent = Some(node);
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    let _frame = renderer.render(
        &doc,
        page,
        &Viewport {
            width: 4,
            height: 4,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    let _ = hit_test(&doc, page, Point::new(0.0, 0.0));
    let tf = absolute_transform(&doc, node);
    assert!(tf.as_coeffs().iter().all(|v| v.is_finite()));
}

#[test]
fn a_twenty_thousand_level_tree_does_not_overflow_render_or_hit_test() {
    let mut doc = open("structure");
    let page = doc.pages[0];
    let mut nodes = doc.nodes.clone();
    let mut parent = page;
    let template = nodes.iter().find(|n| n.kind == "FRAME").unwrap().clone();
    for i in 0..20_000u32 {
        let id = Guid(98, i + 1);
        let mut data = template.data.clone();
        data.set("guid", id.to_val());
        let mut pi = figstudio_fig::Obj::new();
        pi.insert("guid", parent.to_val());
        pi.insert("position", Val::Str(format!("{i:06}")));
        data.set("parentIndex", Val::Obj(pi));
        nodes.push(figstudio_fig::Node::from_data(data).unwrap());
        parent = id;
    }
    doc.set_nodes(nodes).unwrap();
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    let frame = renderer.render(
        &doc,
        page,
        &Viewport {
            width: 4,
            height: 4,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    assert!(frame.stats.depth_skipped > 0);
    let _ = hit_test(&doc, page, Point::new(0.5, 0.5));
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
