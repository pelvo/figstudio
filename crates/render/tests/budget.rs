//! A render has one work budget for everything it expands. Nested instances must stop at the
//! budget, and the work that was skipped must be visible in `Stats`, never silently dropped.

mod common;
use std::path::PathBuf;

use figstudio_fig::{Document, Guid, Node, Obj, Val};
use figstudio_render::{MAX_RENDER_WORK, Renderer, Viewport};

fn structure() -> Document {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    Document::open(path).unwrap()
}

/// Builds a chain of `depth` symbols where each symbol holds two instances of the next one, so the
/// instance count doubles per level. A top instance under the page reaches the chain.
fn fanout(depth: usize) -> Document {
    let mut doc = structure();
    let sym = doc
        .nodes
        .iter()
        .find(|n| n.kind == "SYMBOL")
        .cloned()
        .unwrap();
    let inst = doc
        .nodes
        .iter()
        .find(|n| n.kind == "INSTANCE")
        .cloned()
        .unwrap();
    let page = doc.pages[0];
    let sym_id = |i: usize| Guid(9, 100 + i as u32);
    let sym_parent = sym.parent.unwrap_or(page);
    let parent_of = |guid: Guid, position: String| {
        let mut pi = Obj::new();
        pi.insert("guid", guid.to_val());
        pi.insert("position", Val::Str(position));
        Val::Obj(pi)
    };
    let mut nodes = doc.nodes.clone();
    let mut next = 200u32;
    for i in 0..=depth {
        let mut data = sym.data.clone();
        data.set("guid", sym_id(i).to_val());
        data.set(
            "parentIndex",
            parent_of(sym_parent, format!("{}~{:06}", sym.position, i)),
        );
        nodes.push(Node::from_data(data).unwrap());
        if i < depth {
            for _ in 0..2 {
                let mut data = inst.data.clone();
                data.set("guid", Guid(9, next).to_val());
                data.set("parentIndex", parent_of(sym_id(i), format!("0~{next:06}")));
                data.set("overriddenSymbolID", sym_id(i + 1).to_val());
                nodes.push(Node::from_data(data).unwrap());
                next += 1;
            }
        }
    }
    let mut top = inst.data.clone();
    top.set("guid", Guid(9, next).to_val());
    top.set("parentIndex", parent_of(page, format!("0~{next:06}")));
    top.set("overriddenSymbolID", sym_id(0).to_val());
    nodes.push(Node::from_data(top).unwrap());
    // `set_nodes` rebuilds the id index; assigning `nodes` directly would leave new nodes unseen.
    doc.set_nodes(nodes).unwrap();
    doc
}

#[test]
fn nested_instances_stop_at_the_work_budget_and_the_skip_is_counted() {
    // Depth 18 would expand 1,572,825 instances with no budget (measured on HEAD).
    let doc = fanout(18);
    let mut r = Renderer::new();
    r.system_fonts = false;
    let stats = r
        .render_fit(&doc, doc.pages[0], 0.25, 0.0, None)
        .unwrap()
        .stats;
    // `instances` counts every visit; the budget bounds the expansions, which are the rest.
    let expanded = stats.instances - stats.budget_exhausted;
    assert!(
        expanded <= MAX_RENDER_WORK,
        "work not bounded: {expanded} expanded"
    );
    assert!(
        stats.budget_exhausted > 0,
        "the skipped work is not reported: {stats:?}"
    );
    assert!(
        stats.attempted_work <= MAX_RENDER_WORK,
        "attempted work exceeded cap: {stats:?}"
    );
}

#[test]
fn wide_symbols_and_masks_share_the_same_attempt_budget() {
    let mut doc = structure();
    let page = doc.pages[0];
    let template = doc
        .nodes
        .iter()
        .find(|n| n.kind == "FRAME")
        .unwrap()
        .clone();
    let mut nodes = doc.nodes.clone();
    let parent_of = |id: Guid| {
        let mut p = Obj::new();
        p.insert("guid", id.to_val());
        p.insert("position", Val::Str("0".into()));
        Val::Obj(p)
    };
    let symbol = Guid(77, 1);
    let mut sym = template.data.clone();
    sym.set("guid", symbol.to_val());
    sym.set("type", Val::Enum("SYMBOL"));
    sym.set("parentIndex", parent_of(page));
    nodes.push(Node::from_data(sym).unwrap());
    for i in 0..2500 {
        let mut n = template.data.clone();
        n.set("guid", Guid(77, i + 2).to_val());
        n.set("parentIndex", parent_of(symbol));
        nodes.push(Node::from_data(n).unwrap());
    }
    let mut inst = template.data.clone();
    inst.set("guid", Guid(77, 3000).to_val());
    inst.set("type", Val::Enum("INSTANCE"));
    inst.set("parentIndex", parent_of(page));
    inst.set("overriddenSymbolID", symbol.to_val());
    nodes.push(Node::from_data(inst).unwrap());
    doc.set_nodes(nodes).unwrap();
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    let frame = renderer.render(
        &doc,
        page,
        &Viewport {
            width: 4,
            height: 4,
            transform: Default::default(),
            background: None,
        },
    );
    assert!(frame.stats.attempted_work <= MAX_RENDER_WORK);
    assert!(frame.stats.budget_exhausted > 0 || frame.stats.attempted_work < MAX_RENDER_WORK);
}

/// Page 1 with one rectangle, page 2 with `others` rectangles.
fn two_pages(others: u32) -> Document {
    let mut rect = common::node(3, 1, "RECTANGLE");
    rect.data.set("fillPaints", common::paint(0.0, 0.0, 1.0));
    let mut nodes = vec![
        common::node(1, 0, "CANVAS"),
        rect,
        common::node(2, 0, "CANVAS"),
    ];
    nodes.extend((10..10 + others).map(|n| common::node(n, 2, "RECTANGLE")));
    common::document(nodes)
}

#[test]
fn layers_on_other_pages_do_not_spend_the_frame_budget() {
    // 1,000,000 layers on page 2 used to exhaust the budget before page 1 drew anything.
    let px = common::pixels(&two_pages(1_000_000));
    assert_eq!(common::pixel(&px, 10, 10), &[0, 0, 255, 255]);
}

#[test]
fn frame_work_is_the_same_on_every_frame_and_independent_of_other_pages() {
    let viewport = Viewport {
        width: 80,
        height: 80,
        transform: Default::default(),
        background: None,
    };
    let work = |doc: &Document| {
        let mut r = Renderer::new();
        r.system_fonts = false;
        let first = r.render(doc, common::id(1), &viewport).stats;
        let second = r.render(doc, common::id(1), &viewport).stats;
        assert_eq!(first, second, "a cached frame must report the same work");
        first.attempted_work
    };
    assert_eq!(work(&two_pages(10)), work(&two_pages(5000)));
}

#[test]
fn an_edited_document_is_not_drawn_from_a_stale_style_index() {
    let style = |r: f64, g: f64, b: f64| {
        let mut s = common::node(7, 0, "RECTANGLE");
        s.data.set("styleType", Val::Str("FILL".into()));
        s.data.set("fillPaints", common::paint(r, g, b));
        s
    };
    let mut rect = common::node(3, 1, "RECTANGLE");
    rect.data.set("fillPaints", common::paint(0.5, 0.5, 0.5));
    rect.data.set(
        "styleIdForFill",
        common::obj(&[("guid", common::id(7).to_val())]),
    );
    let mut doc = common::document(vec![
        common::node(1, 0, "CANVAS"),
        rect,
        style(0.0, 1.0, 0.0),
    ]);
    let viewport = Viewport {
        width: 80,
        height: 80,
        transform: Default::default(),
        background: None,
    };
    let mut r = Renderer::new();
    r.system_fonts = false;
    let draw = |r: &mut Renderer, doc: &Document| {
        let frame = r.render(doc, common::id(1), &viewport);
        common::pixel(frame.pixmap.data_as_u8_slice(), 10, 10).to_vec()
    };
    assert_eq!(draw(&mut r, &doc), [0, 255, 0, 255]);
    let mut nodes = doc.nodes.clone();
    nodes.pop();
    nodes.push(style(1.0, 0.0, 0.0));
    doc.set_nodes(nodes).unwrap();
    assert_eq!(
        draw(&mut r, &doc),
        [255, 0, 0, 255],
        "style index rebuilt after the edit"
    );
}
