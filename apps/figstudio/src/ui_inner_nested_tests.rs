use super::inner_fixture::{id, path};
use super::*;
use egui::{Key, Modifiers};
use figstudio_render::{Renderer, kurbo::Point};

pub(super) fn at(h: &Harness<'_, App>, keys: &[u32]) -> Pos2 {
    let tab = &h.state().tabs[0];
    let bounds = Renderer::new().path_bounds(&tab.doc, &path(keys)).unwrap();
    let centre = bounds.center();
    tab.view.to_screen(
        tab.canvas_rect.unwrap().min,
        Pos2::new(centre.x as f32, centre.y as f32),
    )
}

#[test]
fn nested_instances_drill_and_shift_enter_visits_ordinary_and_instance_parents() {
    let (mut h, mut c) = (inner_fixture::harness(), Vec::new());
    let at = at(&h, &[106, 112]);
    pick::click(&mut h, at, Modifiers::NONE);
    assert_eq!(h.state().tabs[0].selection, Some(id(200)));
    for keys in [&[101][..], &[106], &[106, 111], &[106, 112]] {
        text_select::double_click(&mut h, &mut c, at);
        assert_eq!(h.state().tabs[0].inner, Some(path(keys)));
        assert!(h.state().tabs[0].text_select.is_none());
    }
    text_select::double_click(&mut h, &mut c, at);
    assert!(h.state().tabs[0].text_select.is_some());
    for keys in [&[106, 111][..], &[106], &[101]] {
        text_select::key(&mut h, &mut c, Key::Enter, Modifiers::SHIFT);
        assert_eq!(h.state().tabs[0].inner, Some(path(keys)));
        assert!(h.state().tabs[0].text_select.is_none());
    }
    text_select::key(&mut h, &mut c, Key::Enter, Modifiers::SHIFT);
    assert!(h.state().tabs[0].inner.is_none());
    assert_eq!(h.state().tabs[0].selection, Some(id(200)));
}

#[test]
fn inner_cousin_click_keeps_the_nearest_common_context() {
    let mut h = inner_fixture::harness();
    let from = at(&h, &[103]);
    pick::click(&mut h, from, Modifiers::COMMAND);
    assert_eq!(h.state().tabs[0].inner, Some(path(&[103])));
    let cousin = at(&h, &[105]);
    pick::click(&mut h, cousin, Modifiers::NONE);
    assert_eq!(h.state().tabs[0].inner, Some(path(&[104])));
    assert_eq!(h.state().tabs[0].selection, Some(id(200)));
}

#[test]
fn plain_click_inside_inner_target_keeps_it_and_siblings_keep_depth() {
    let mut h = inner_fixture::harness();
    let text = at(&h, &[103]);
    pick::click(&mut h, text, Modifiers::COMMAND);
    pick::click(&mut h, text, Modifiers::NONE);
    assert_eq!(h.state().tabs[0].inner, Some(path(&[103])));
    let sibling = at(&h, &[107]);
    pick::click(&mut h, sibling, Modifiers::NONE);
    assert_eq!(h.state().tabs[0].inner, Some(path(&[107])));
}

#[test]
fn inner_outline_uses_axis_aligned_resolved_bounds() {
    let mut h = inner_fixture::harness();
    let at = at(&h, &[103]);
    pick::click(&mut h, at, Modifiers::COMMAND);
    let tab = &h.state().tabs[0];
    let b = Renderer::new()
        .path_bounds(&tab.doc, &path(&[103]))
        .unwrap();
    let origin = tab.canvas_rect.unwrap().min;
    let rect = egui::Rect::from_min_max(
        tab.view
            .to_screen(origin, Pos2::new(b.x0 as f32, b.y0 as f32)),
        tab.view
            .to_screen(origin, Pos2::new(b.x1 as f32, b.y1 as f32)),
    );
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Rect(r) if r.rect == rect && r.stroke.width == 1.5)));
}

#[test]
fn inner_outline_uses_resolved_rotated_quad() {
    use figstudio_fig::Val;
    let mut h = inner_fixture::harness();
    let mut doc = inner_fixture::document();
    let mut nodes = doc.nodes.clone();
    let tf = nodes
        .iter_mut()
        .find(|n| n.id == id(103))
        .unwrap()
        .data
        .get_mut("transform")
        .unwrap();
    let c = std::f64::consts::FRAC_1_SQRT_2;
    for (key, value) in [("m00", c), ("m01", -c), ("m10", c), ("m11", c)] {
        tf.set(key, Val::Float(value));
    }
    doc.set_nodes(nodes).unwrap();
    h.state_mut().tabs[0].doc = std::sync::Arc::new(doc);
    h.state_mut().tabs[0].select(Some(id(200)));
    h.state_mut().tabs[0].set_inner(Some(path(&[103])));
    h.run_steps(1);
    let tab = &h.state().tabs[0];
    let (tf, data) = Renderer::new()
        .path_layout(&tab.doc, &path(&[103]))
        .unwrap();
    let (w, height) = figstudio_render::geom::size(&data);
    let expected = [(0.0, 0.0), (w, 0.0), (w, height), (0.0, height)].map(|p| {
        let p = tf * figstudio_render::kurbo::Point::from(p);
        tab.view.to_screen(
            tab.canvas_rect.unwrap().min,
            Pos2::new(p.x as f32, p.y as f32),
        )
    });
    assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Path(p) if p.closed && p.stroke.width == 1.5
            && p.points.len() == 4 && p.points.iter().zip(expected).all(|(a,b)| a.distance(b) < 0.01))));
}

#[test]
fn overridden_text_with_stale_glyphs_enters_whole_text_fallback() {
    use super::pick::object as obj;
    use figstudio_fig::Val;
    let (mut h, mut c) = (inner_fixture::harness(), Vec::new());
    let mut doc = inner_fixture::document();
    let mut nodes = doc.nodes.clone();
    let glyphs = (0..6)
        .map(|i| {
            obj(&[
                ("fontSize", Val::Float(10.0)),
                (
                    "position",
                    obj(&[
                        ("x", Val::Float(f64::from(i) * 8.0)),
                        ("y", Val::Float(10.0)),
                    ]),
                ),
            ])
        })
        .collect();
    nodes
        .iter_mut()
        .find(|n| n.id == id(103))
        .unwrap()
        .data
        .set("derivedTextData", obj(&[("glyphs", Val::Arr(glyphs))]));
    nodes
        .iter_mut()
        .find(|n| n.id == id(200))
        .unwrap()
        .data
        .get_mut("symbolData")
        .unwrap()
        .set(
            "symbolOverrides",
            Val::Arr(vec![obj(&[
                (
                    "guidPath",
                    obj(&[("guids", Val::Arr(vec![id(103).to_val()]))]),
                ),
                (
                    "textData",
                    obj(&[("characters", Val::Str("resolved characters".into()))]),
                ),
            ])]),
        );
    doc.set_nodes(nodes).unwrap();
    h.state_mut().tabs[0].doc = std::sync::Arc::new(doc);
    let at = at(&h, &[103]);
    pick::click(&mut h, at, Modifiers::COMMAND);
    text_select::double_click(&mut h, &mut c, at);
    let selection = h.state().tabs[0].text_select.unwrap();
    assert_eq!((selection.anchor, selection.head), (0, 19));
    text_select::key(&mut h, &mut c, Key::C, Modifiers::COMMAND);
    assert_eq!(c, ["resolved characters"]);
}

#[test]
fn inner_hit_order_and_cyclic_symbols_are_bounded() {
    let mut doc = inner_fixture::document();
    let mut nodes = doc.nodes.clone();
    nodes
        .iter_mut()
        .find(|n| n.id == id(104))
        .unwrap()
        .data
        .get_mut("transform")
        .unwrap()
        .set("m02", figstudio_fig::Val::Float(0.0));
    doc.set_nodes(nodes.clone()).unwrap();
    let hits = |doc: &figstudio_fig::Document| {
        crate::instance_pick::hit_chain(doc, id(200), Point::new(30.0, 30.0))
    };
    assert_eq!(hits(&doc).last(), Some(&path(&[105])));
    nodes
        .iter_mut()
        .find(|n| n.id == id(101))
        .unwrap()
        .data
        .set("stackReverseZIndex", figstudio_fig::Val::Bool(true));
    nodes
        .iter_mut()
        .find(|n| n.id == id(106))
        .unwrap()
        .data
        .get_mut("symbolData")
        .unwrap()
        .set("symbolID", id(100).to_val());
    doc.set_nodes(nodes).unwrap();
    assert_eq!(hits(&doc).last(), Some(&path(&[103])));
    let cycle = crate::instance_pick::hit_chain(&doc, id(200), Point::new(30.0, 130.0));
    assert_eq!(cycle.last(), Some(&path(&[106])));
}

#[test]
fn instance_children_use_the_definitions_stacking_order() {
    use figstudio_fig::{Node, Val};
    let mut doc = inner_fixture::document();
    let mut nodes = doc.nodes.clone();
    let mut front = doc.node(id(105)).unwrap().data.clone();
    front.set("guid", id(115).to_val());
    front
        .get_mut("parentIndex")
        .unwrap()
        .set("guid", id(100).to_val());
    front
        .get_mut("parentIndex")
        .unwrap()
        .set("position", Val::Str("115".into()));
    nodes.push(Node::from_data(front).unwrap());
    doc.set_nodes(nodes.clone()).unwrap();
    let hits = |doc: &figstudio_fig::Document| {
        crate::instance_pick::hit_chain(doc, id(200), Point::new(30.0, 30.0))
    };
    assert_eq!(hits(&doc).last(), Some(&path(&[115])));
    nodes
        .iter_mut()
        .find(|n| n.id == id(100))
        .unwrap()
        .data
        .set("stackReverseZIndex", Val::Bool(true));
    doc.set_nodes(nodes).unwrap();
    assert_eq!(hits(&doc).last(), Some(&path(&[103])));
}

#[test]
fn inner_hit_chain_respects_masks_hidden_ancestors_and_clips() {
    let mut doc = inner_fixture::document();
    let point = Point::new(30.0, 30.0);
    let hits = |doc: &figstudio_fig::Document| crate::instance_pick::hit_chain(doc, id(200), point);
    assert_eq!(hits(&doc).last(), Some(&path(&[103])));

    // Empty groups (no visible non-mask child under the pointer) pass through rather than
    // catching clicks in empty space.
    let mut masked_group = doc.nodes.clone();
    masked_group
        .iter_mut()
        .find(|n| n.id == id(103))
        .unwrap()
        .data
        .set("mask", figstudio_fig::Val::Bool(true));
    let mut group_doc = doc.clone();
    group_doc.set_nodes(masked_group).unwrap();
    assert!(hits(&group_doc).is_empty());

    // Box containers (FRAME) still catch hits when their child is masked or hidden, and clip
    // when `frameMaskDisabled` is false.
    let mut nodes = doc.nodes.clone();
    for container in [id(101), id(102)] {
        let node = nodes.iter_mut().find(|n| n.id == container).unwrap();
        node.kind = "FRAME".into();
        node.data
            .set("type", figstudio_fig::Val::Str("FRAME".into()));
    }
    nodes
        .iter_mut()
        .find(|n| n.id == id(103))
        .unwrap()
        .data
        .set("mask", figstudio_fig::Val::Bool(true));
    doc.set_nodes(nodes).unwrap();
    assert_eq!(hits(&doc).last(), Some(&path(&[102])));
    let mut nodes = doc.nodes.clone();
    nodes
        .iter_mut()
        .find(|n| n.id == id(102))
        .unwrap()
        .data
        .set("visible", figstudio_fig::Val::Bool(false));
    doc.set_nodes(nodes).unwrap();
    assert_eq!(hits(&doc).last(), Some(&path(&[101])));
    let mut nodes = doc.nodes.clone();
    let frame = nodes.iter_mut().find(|n| n.id == id(101)).unwrap();
    frame
        .data
        .set("frameMaskDisabled", figstudio_fig::Val::Bool(false));
    frame
        .data
        .get_mut("size")
        .unwrap()
        .set("x", figstudio_fig::Val::Float(10.0));
    doc.set_nodes(nodes).unwrap();
    assert!(hits(&doc).is_empty());
}

#[test]
fn inner_pick_handles_bent_vector_styles_clips_and_locked() {
    use super::pick::object as obj;
    use figstudio_fig::{Node, Val};
    use figstudio_render::HitOptions;

    let mut doc = inner_fixture::document();
    // Bent 2px stroke ribbon along top (0..220, y=0..2) and right (x=218..220, y=0..160).
    let poly = |pts: &[(f32, f32)]| {
        let mut b = Vec::new();
        for (i, &(x, y)) in pts.iter().enumerate() {
            b.push(if i == 0 { 1 } else { 2 });
            b.extend(x.to_le_bytes());
            b.extend(y.to_le_bytes());
        }
        b.push(0);
        b
    };
    let top = doc.blobs.len() as i64;
    doc.blobs
        .push(poly(&[(0.0, 0.0), (220.0, 0.0), (220.0, 2.0), (0.0, 2.0)]));
    let right = doc.blobs.len() as i64;
    doc.blobs.push(poly(&[
        (218.0, 0.0),
        (220.0, 0.0),
        (220.0, 160.0),
        (218.0, 160.0),
    ]));
    let stroke_geom = Val::Arr(vec![
        obj(&[("commandsBlob", Val::Int(top))]),
        obj(&[("commandsBlob", Val::Int(right))]),
    ]);
    let solid = Val::Arr(vec![obj(&[
        ("type", Val::Str("SOLID".into())),
        (
            "color",
            obj(&[
                ("r", Val::Float(0.0)),
                ("g", Val::Float(0.0)),
                ("b", Val::Float(0.0)),
                ("a", Val::Float(1.0)),
            ]),
        ),
    ])]);

    // Attached stroke style with stale empty `strokePaints: []` on the vector itself.
    let style_node = Node::from_data(obj(&[
        ("guid", id(190).to_val()),
        ("type", Val::Str("RECTANGLE".into())),
        ("styleType", Val::Str("FILL".into())),
        ("fillPaints", solid.clone()),
    ]))
    .unwrap();
    let vec_node = Node::from_data(obj(&[
        ("guid", id(199).to_val()),
        ("type", Val::Str("VECTOR".into())),
        (
            "parentIndex",
            obj(&[
                ("guid", id(100).to_val()),
                ("position", Val::Str("999".into())),
            ]),
        ),
        (
            "size",
            obj(&[("x", Val::Float(220.0)), ("y", Val::Float(160.0))]),
        ),
        ("fillPaints", Val::Arr(vec![])),
        ("strokePaints", Val::Arr(vec![])),
        ("styleIdForStrokeFill", obj(&[("guid", id(190).to_val())])),
        ("strokeWeight", Val::Float(2.0)),
        ("strokeGeometry", stroke_geom),
    ]))
    .unwrap();
    // Clipping frame inside SYMBOL with a clipped-out child LINE at y = -2.0.
    let clip_frame = Node::from_data(obj(&[
        ("guid", id(191).to_val()),
        ("type", Val::Str("FRAME".into())),
        (
            "parentIndex",
            obj(&[
                ("guid", id(100).to_val()),
                ("position", Val::Str("998".into())),
            ]),
        ),
        (
            "size",
            obj(&[("x", Val::Float(60.0)), ("y", Val::Float(40.0))]),
        ),
        (
            "transform",
            obj(&[
                ("m00", Val::Float(1.0)),
                ("m11", Val::Float(1.0)),
                ("m02", Val::Float(0.0)),
                ("m12", Val::Float(200.0)),
            ]),
        ),
        ("frameMaskDisabled", Val::Bool(false)),
    ]))
    .unwrap();
    let clipped_line = Node::from_data(obj(&[
        ("guid", id(192).to_val()),
        ("type", Val::Str("LINE".into())),
        (
            "parentIndex",
            obj(&[
                ("guid", id(191).to_val()),
                ("position", Val::Str("001".into())),
            ]),
        ),
        (
            "size",
            obj(&[("x", Val::Float(60.0)), ("y", Val::Float(0.0))]),
        ),
        (
            "transform",
            obj(&[
                ("m00", Val::Float(1.0)),
                ("m11", Val::Float(1.0)),
                ("m02", Val::Float(0.0)),
                ("m12", Val::Float(-2.0)),
            ]),
        ),
        ("strokePaints", solid),
        ("strokeWeight", Val::Float(1.0)),
    ]))
    .unwrap();

    let mut nodes = doc.nodes.clone();
    nodes.extend([style_node, vec_node, clip_frame, clipped_line]);
    doc.set_nodes(nodes).unwrap();

    let interactive = HitOptions {
        tolerance: 4.0,
        skip_locked: true,
    };
    let mut r = Renderer::new();
    // Off-stroke passes through styled bent vector to underlying TEXT 103.
    assert_eq!(
        crate::instance_pick::hit_chain_with_options(
            &doc,
            &mut r,
            id(200),
            Point::new(30.0, 30.0),
            interactive,
        )
        .last(),
        Some(&path(&[103]))
    );
    // On-stroke hits the styled bent vector even though raw strokePaints is empty.
    assert_eq!(
        crate::instance_pick::hit_chain_with_options(
            &doc,
            &mut r,
            id(200),
            Point::new(219.0, 80.0),
            interactive,
        )
        .last(),
        Some(&path(&[199]))
    );
    // Clipped-out child LINE at y = -2 inside clipping frame must not steal clicks inside frame.
    assert_eq!(
        crate::instance_pick::hit_chain_with_options(
            &doc,
            &mut r,
            id(200),
            Point::new(30.0, 201.0),
            interactive,
        )
        .last(),
        Some(&path(&[191]))
    );
    // Locking the bent vector passes direct on-stroke hits through when skip_locked is true.
    let mut locked_nodes = doc.nodes.clone();
    locked_nodes
        .iter_mut()
        .find(|n| n.id == id(199))
        .unwrap()
        .data
        .set("locked", Val::Bool(true));
    doc.set_nodes(locked_nodes).unwrap();
    assert_eq!(
        crate::instance_pick::hit_chain_with_options(
            &doc,
            &mut r,
            id(200),
            Point::new(30.0, 1.0),
            HitOptions {
                tolerance: 4.0,
                skip_locked: false,
            },
        )
        .last(),
        Some(&path(&[199]))
    );
    assert_ne!(
        crate::instance_pick::hit_chain_with_options(
            &doc,
            &mut r,
            id(200),
            Point::new(30.0, 1.0),
            interactive,
        )
        .last(),
        Some(&path(&[199]))
    );
}

#[test]
fn nested_instance_drills_within_triple_click_window_and_when_zoomed_out() {
    let mut h = inner_fixture::harness();
    let p = at(&h, &[106, 112]);

    primary_clicks(&mut h, p, egui::Vec2::ZERO, 1);
    assert_eq!(h.state().tabs[0].selection, Some(id(200)));
    assert!(h.state().tabs[0].inner.is_none());

    // Natural 400 ms pauses between double-clicks through nested instance levels and into text.
    for keys in [&[101][..], &[106], &[106, 111], &[106, 112]] {
        h.run_steps(24);
        primary_clicks(&mut h, p, egui::Vec2::ZERO, 2);
        assert_eq!(
            h.state().tabs[0].inner,
            Some(path(keys)),
            "double-click 400 ms after prior gesture must drill inside nested instance to {keys:?}"
        );
        assert!(h.state().tabs[0].text_select.is_none());
    }
    h.run_steps(24);
    primary_clicks(&mut h, p, egui::Vec2::ZERO, 2);
    assert!(
        h.state().tabs[0].text_select.is_some(),
        "final double-click on inner TEXT must enter text selection"
    );

    // Zoom out so outer instance id(200) (240x240) is 12x12 screen pt and handles cover it.
    let mut h = inner_fixture::harness();
    h.state_mut().tabs[0].zoom_centre(0.05);
    h.run_steps(4);
    let p = at(&h, &[106, 112]);
    primary_clicks(&mut h, p, egui::Vec2::ZERO, 1);
    assert_eq!(h.state().tabs[0].selection, Some(id(200)));
    for keys in [&[101][..], &[106]] {
        h.run_steps(60);
        primary_clicks(&mut h, p, egui::Vec2::ZERO, 2);
        assert_eq!(
            h.state().tabs[0].inner,
            Some(path(keys)),
            "double-clicking a zoomed-out instance covered by handles must drill into {keys:?}"
        );
    }
    assert!(!h.state().tabs[0].dirty_unsaved);
}
