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
    let mut nodes = doc.nodes.clone();
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
    let group = nodes.iter_mut().find(|n| n.id == id(101)).unwrap();
    group
        .data
        .set("frameMaskDisabled", figstudio_fig::Val::Bool(false));
    group.kind = "FRAME".into();
    group
        .data
        .get_mut("size")
        .unwrap()
        .set("x", figstudio_fig::Val::Float(10.0));
    doc.set_nodes(nodes).unwrap();
    assert!(hits(&doc).is_empty());
}
