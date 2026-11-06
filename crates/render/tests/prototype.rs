//! Prototype flows, links and the player, on documents built in the test.
mod common;
#[path = "common/prototype.rs"]
mod proto;

use common::{document, id, instance, node, obj, paint, patch, size};
use figstudio_fig::{Document, Val};
use figstudio_render::Renderer;
use figstudio_render::Viewport;
use figstudio_render::kurbo::Affine;
use figstudio_render::kurbo::Point;
use figstudio_render::prototype::MAX_LINKS;
use figstudio_render::prototype::{self, Layer, Player, Screen, Step};
use proto::*;

fn read(doc: &Document, frame: figstudio_fig::Guid) -> Screen {
    Screen::read(&mut Renderer::new(), doc, frame)
}

#[test]
fn flows_name_their_start_frame() {
    let doc = prototype_doc();
    let flows = prototype::flows(&doc);
    assert_eq!(flows.len(), 1);
    assert_eq!((flows[0].name.as_str(), flows[0].start), ("Start", FIRST));
    assert_eq!(flows[0].page, doc.pages[0]);
    assert_eq!(prototype::start(&doc, doc.pages[0], None), Some(FIRST));
    assert_eq!(
        prototype::start(&doc, doc.pages[0], Some(BACK)),
        Some(SECOND),
        "a selection starts on its own screen"
    );
}

#[test]
fn a_click_on_a_link_navigates_and_back_returns() {
    let doc = prototype_doc();
    let first = read(&doc, FIRST);
    let link = &first.links[0];
    assert_eq!(first.links.len(), 1);
    assert_eq!(link.layer, Layer::Node(LINK));
    assert_eq!(link.step, Some(Step::Navigate(SECOND)));
    assert_eq!(first.hit(link.bounds.center()), Some(link));
    assert_eq!(first.hit(Point::new(-1e6, -1e6)), None);

    let second = read(&doc, SECOND);
    let back = second
        .links
        .iter()
        .find(|l| l.layer == Layer::Node(BACK))
        .unwrap();
    assert_eq!(back.step, Some(Step::Back));
    let hover = second
        .links
        .iter()
        .find(|l| l.layer == Layer::Node(HOVER))
        .unwrap();
    assert_eq!(
        (hover.trigger, hover.action, hover.step.clone()),
        ("ON_HOVER", "OVERLAY", None)
    );
    assert_eq!(
        second.skipped(),
        1,
        "the hover overlay is counted, not played"
    );

    let mut player = Player::new(FIRST);
    assert!(!player.go(Step::Back), "nowhere to go back to");
    assert!(player.go(Step::Navigate(SECOND)));
    assert_eq!(player.screen(), SECOND);
    assert!(player.go(Step::Back));
    assert_eq!(player.screen(), FIRST);
    player.go(Step::Navigate(SECOND));
    assert!(player.restart());
    assert_eq!(player.screen(), FIRST);
}

#[test]
fn reachable_follows_played_links_once() {
    let doc = prototype_doc();
    let (screens, truncated) = prototype::reachable(&mut Renderer::new(), &doc, &[FIRST], 10);
    let frames: Vec<_> = screens.iter().map(|s| s.frame).collect();
    assert_eq!((frames, truncated), (vec![FIRST, SECOND], false));
    let (screens, truncated) = prototype::reachable(&mut Renderer::new(), &doc, &[FIRST], 1);
    assert_eq!((screens.len(), truncated), (1, true));
}

#[test]
fn prototype_data_survives_a_save() {
    let doc = Document::from_bytes(&prototype_doc().to_bytes().unwrap()).unwrap();
    assert_eq!(prototype::flows(&doc), prototype::flows(&prototype_doc()));
    assert_eq!(read(&doc, FIRST), read(&prototype_doc(), FIRST));
}

/// Page 1 holds frame 4 with instance 5 of component 2 (child 3) and frames 10 and 11.
fn instance_doc(component: Val, child: Val, patches: Vec<Val>) -> Document {
    let mut symbol = node(2, 0, "SYMBOL");
    symbol.data.set("prototypeInteractions", component);
    let mut inner = node(3, 2, "RECTANGLE");
    inner.data.set("prototypeInteractions", child);
    document(vec![
        node(1, 0, "CANVAS"),
        symbol,
        inner,
        node(4, 1, "FRAME"),
        instance(5, 4, 2, patches),
        node(10, 1, "FRAME"),
        node(11, 1, "FRAME"),
    ])
}

#[test]
fn links_inside_instances_follow_the_component_and_its_overrides() {
    let doc = instance_doc(navigate(id(10)), navigate(id(10)), vec![]);
    let screen = read(&doc, id(4));
    let layers: Vec<_> = screen
        .links
        .iter()
        .map(|l| (l.layer.to_string(), l.step.clone()))
        .collect();
    assert_eq!(
        layers,
        vec![
            ("99:5".into(), Some(Step::Navigate(id(10)))),
            ("I99:5;99:3".into(), Some(Step::Navigate(id(10)))),
        ],
        "the instance uses its component's links, and its child's"
    );

    let overridden = instance_doc(
        Val::Arr(vec![]),
        navigate(id(10)),
        vec![patch(&[3], &[("prototypeInteractions", navigate(id(11)))])],
    );
    let screen = read(&overridden, id(4));
    assert_eq!(screen.links.len(), 1);
    assert_eq!(
        screen.links[0].step,
        Some(Step::Navigate(id(11))),
        "the override wins"
    );

    let hidden = instance_doc(
        Val::Arr(vec![]),
        navigate(id(10)),
        vec![patch(&[3], &[("visible", Val::Bool(false))])],
    );
    assert!(
        read(&hidden, id(4)).links.is_empty(),
        "hidden layers have no links"
    );
}

#[test]
fn links_to_missing_frames_are_listed_but_not_played() {
    let doc = instance_doc(navigate(id(777)), Val::Arr(vec![]), vec![]);
    let link = &read(&doc, id(4)).links[0];
    assert_eq!((link.destination, link.step.clone()), (Some(id(777)), None));
}

/// Writes the prototype to `$FIGSTUDIO_PROTOTYPE_FIXTURE` for `web/e2e/prototype.spec.mjs`.
#[test]
#[ignore = "writes a file for the browser spec"]
fn write_fixture() {
    let path = std::env::var("FIGSTUDIO_PROTOTYPE_FIXTURE").expect("FIGSTUDIO_PROTOTYPE_FIXTURE");
    prototype_doc().save(path).unwrap();
}

// Found by the adversarial review of PR #71; each failed before its fix.
#[test]
fn one_layer_past_the_link_cap_is_reported_truncated() {
    let mut link = node(3, 2, "RECTANGLE");
    let mut interactions = proto::navigate(id(4));
    let action = interactions.items()[0].get("actions").unwrap().items()[0].clone();
    let mut interaction = interactions.items()[0].clone();
    interaction.set("actions", Val::Arr(vec![action; MAX_LINKS + 1]));
    interactions = Val::Arr(vec![interaction]);
    link.data.set("prototypeInteractions", interactions);
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 1, "FRAME"),
        link,
        node(4, 1, "FRAME"),
    ]);
    let screen = Screen::read(&mut Renderer::new(), &doc, id(2));
    assert!(screen.truncated, "discarded links must be disclosed");
}

#[test]
fn a_rotated_link_is_hit_by_its_shape_not_its_box() {
    let mut frame = node(2, 1, "FRAME");
    frame.data.set("size", size(100.0, 100.0));
    let mut link = node(3, 2, "RECTANGLE");
    let c = std::f64::consts::FRAC_1_SQRT_2;
    link.data.set(
        "transform",
        obj(&[
            ("m00", Val::Float(c)),
            ("m01", Val::Float(-c)),
            ("m10", Val::Float(c)),
            ("m11", Val::Float(c)),
            ("m02", Val::Float(40.0)),
        ]),
    );
    link.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    link.data
        .set("prototypeInteractions", proto::navigate(id(4)));
    let doc = document(vec![node(1, 0, "CANVAS"), frame, link, node(4, 1, "FRAME")]);
    let mut renderer = Renderer::new();
    let screen = Screen::read(&mut renderer, &doc, id(2));
    let p = Point::new(13.0, 1.0);
    let render = renderer.render(
        &doc,
        id(2),
        &Viewport {
            width: 100,
            height: 100,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    let pixels = render.pixmap.data_as_u8_slice();
    let rgba = &pixels[(100 + 13) * 4..(100 + 14) * 4];
    assert_eq!(rgba[3], 0, "the renderer draws nothing at the point");
    assert!(
        screen.hit(p).is_none(),
        "a corner outside the rotated rectangle is not a link"
    );
}

#[test]
fn a_link_clipped_out_of_its_frame_cannot_be_clicked() {
    let mut frame = node(2, 1, "FRAME");
    frame.data.set("frameMaskDisabled", Val::Bool(false));
    let mut link = node(3, 2, "RECTANGLE");
    link.data
        .set("transform", obj(&[("m02", Val::Float(50.0))]));
    link.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    link.data
        .set("prototypeInteractions", proto::navigate(id(4)));
    let doc = document(vec![node(1, 0, "CANVAS"), frame, link, node(4, 1, "FRAME")]);
    let mut renderer = Renderer::new();
    let screen = Screen::read(&mut renderer, &doc, id(2));
    let p = Point::new(60.0, 10.0);
    let render = renderer.render(
        &doc,
        id(2),
        &Viewport {
            width: 100,
            height: 100,
            transform: Affine::IDENTITY,
            background: None,
        },
    );
    let pixels = render.pixmap.data_as_u8_slice();
    let rgba = &pixels[(10 * 100 + 60) * 4..(10 * 100 + 61) * 4];
    assert_eq!(rgba[3], 0, "the renderer draws nothing at the point");
    assert!(
        screen.hit(p).is_none(),
        "a fully clipped layer cannot be clicked"
    );
}

#[test]
fn a_click_plays_its_navigate_even_after_an_unplayed_action() {
    let mut link = node(3, 2, "RECTANGLE");
    let mut interaction = proto::navigate(id(4)).items()[0].clone();
    let navigate = interaction.get("actions").unwrap().items()[0].clone();
    let url = obj(&[
        ("connectionType", proto::name("URL")),
        ("connectionURL", Val::Str("https://example.invalid/".into())),
    ]);
    interaction.set("actions", Val::Arr(vec![navigate, url]));
    link.data
        .set("prototypeInteractions", Val::Arr(vec![interaction]));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        node(2, 1, "FRAME"),
        link,
        node(4, 1, "FRAME"),
    ]);
    let screen = Screen::read(&mut Renderer::new(), &doc, id(2));
    let hit = screen.hit(Point::new(20.0, 20.0));
    assert_eq!(
        hit.and_then(|l| l.step.clone()),
        Some(figstudio_render::prototype::Step::Navigate(id(4))),
        "the supported action of this click must still play"
    );
}

#[test]
fn a_link_to_a_hidden_frame_is_listed_but_not_played() {
    let mut doc = prototype_doc();
    let mut nodes = doc.nodes.clone();
    for n in &mut nodes {
        if n.id == SECOND {
            n.data.set("visible", Val::Bool(false));
        }
    }
    doc.set_nodes(nodes).unwrap();
    let first = read(&doc, FIRST);
    assert_eq!(
        (first.links[0].destination, first.links[0].step.clone()),
        (Some(SECOND), None)
    );
    let (screens, _) = prototype::reachable(&mut Renderer::new(), &doc, &[FIRST], 10);
    assert_eq!(screens.len(), 1, "a hidden frame is never shown");
}

/// A node of `kind` under `parent` with its top-left at (`x`, `y`).
fn placed(n: u32, parent: u32, kind: &str, x: f64, y: f64) -> figstudio_fig::Node {
    let mut node = node(n, parent, kind);
    node.data.set(
        "transform",
        obj(&[("m02", Val::Float(x)), ("m12", Val::Float(y))]),
    );
    node
}

#[test]
fn frames_follow_figmas_present_order() {
    let mut set = placed(7, 1, "FRAME", 0.0, 0.0);
    set.data.set("isStateGroup", Val::Bool(true));
    let mut hidden = placed(8, 1, "FRAME", 10.0, 10.0);
    hidden.data.set("visible", Val::Bool(false));
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        placed(2, 1, "FRAME", 500.0, 100.0),
        placed(3, 1, "FRAME", 0.0, 100.0),
        placed(4, 1, "SECTION", 0.0, 0.0),
        placed(5, 4, "FRAME", 200.0, -50.0),
        placed(6, 1, "SYMBOL", 0.0, -100.0),
        set,
        hidden,
        placed(9, 1, "INSTANCE", 0.0, -200.0),
    ]);
    assert_eq!(
        prototype::frames(&doc, id(1)),
        vec![id(5), id(3), id(2)],
        "frames in sections count; component sets, components, instances and hidden frames do not; rows top to bottom, then left to right"
    );
    assert_eq!(
        prototype::start(&doc, id(1), None),
        Some(id(5)),
        "no flow: the first frame"
    );
}

#[test]
fn frames_keep_figmas_insertion_sort_where_the_comparison_is_not_transitive() {
    // "Left first when they overlap vertically, else top first" is not transitive, so the
    // order depends on the sort: Figma's stable insertion sort from layer order gives this
    // one; starting from another order gives another.
    let sized = |n: u32, x: f64, y: f64, h: f64| {
        let mut f = placed(n, 1, "FRAME", x, y);
        f.data.set("size", size(40.0, h));
        f
    };
    let doc = document(vec![
        node(1, 0, "CANVAS"),
        sized(10, 200.0, 40.0, 80.0),
        sized(11, 100.0, 100.0, 20.0),
        sized(12, 200.0, 40.0, 20.0),
        sized(13, 200.0, 0.0, 50.0),
    ]);
    assert_eq!(
        prototype::frames(&doc, id(1)),
        vec![id(13), id(11), id(10), id(12)]
    );
}

#[test]
fn present_is_black_where_the_page_sets_no_prototype_background() {
    let doc = prototype_doc();
    assert_eq!(
        prototype::background(&doc, doc.pages[0]).components,
        [0.0, 0.0, 0.0, 1.0]
    );
}

/// Variants 2 (red) and 4 (blue), each a 40×40 component with a 20×20 child of its colour,
/// and instance 6 of variant 2 on page 1. `extra` nodes are added.
fn variants(extra: Vec<figstudio_fig::Node>) -> Document {
    let variant = |n: u32, child: u32, (r, g, b): (f64, f64, f64)| {
        let mut v = node(n, 0, "SYMBOL");
        v.data.set("fillPaints", paint(r, g, b));
        let mut c = node(child, n, "RECTANGLE");
        c.data.set("size", size(20.0, 20.0));
        c.data.set("fillPaints", paint(r, g, b));
        c.data.set("overrideKey", id(50).to_val());
        vec![v, c]
    };
    let mut inst = instance(6, 1, 2, vec![]);
    inst.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let mut nodes = vec![node(1, 0, "CANVAS"), inst];
    nodes.extend(variant(2, 3, (1.0, 0.0, 0.0)));
    nodes.extend(variant(4, 5, (0.0, 0.0, 1.0)));
    nodes.extend(extra);
    document(nodes)
}

fn render_with(
    doc: &Document,
    swaps: &[(Vec<figstudio_fig::Guid>, figstudio_fig::Guid)],
) -> Vec<u8> {
    let mut renderer = Renderer::new();
    renderer.system_fonts = false;
    renderer.prototype = std::sync::Arc::new(prototype::State {
        swaps: swaps.iter().cloned().collect(),
    });
    renderer
        .render(
            doc,
            id(1),
            &figstudio_render::Viewport {
                width: 80,
                height: 80,
                transform: Affine::IDENTITY,
                background: None,
            },
        )
        .pixmap
        .data_as_u8_slice()
        .to_vec()
}

#[test]
fn a_swapped_instance_shows_the_other_variant_and_keeps_its_overrides() {
    let doc = variants(vec![]);
    let red = [255, 0, 0, 255];
    let blue = [0, 0, 255, 255];
    assert_eq!(common::pixel(&render_with(&doc, &[]), 30, 30), &red);
    let swapped = render_with(&doc, &[(vec![id(6)], id(4))]);
    assert_eq!(
        common::pixel(&swapped, 30, 30),
        &blue,
        "the variant's own fill"
    );
    assert_eq!(
        common::pixel(&swapped, 10, 10),
        &blue,
        "the variant's child"
    );

    let mut doc = variants(vec![]);
    let mut nodes = doc.nodes.clone();
    nodes
        .iter_mut()
        .find(|n| n.id == id(6))
        .unwrap()
        .data
        .set("fillPaints", paint(0.0, 1.0, 0.0));
    doc.set_nodes(nodes).unwrap();
    let swapped = render_with(&doc, &[(vec![id(6)], id(4))]);
    assert_eq!(
        common::pixel(&swapped, 30, 30),
        &[0, 255, 0, 255],
        "an overridden fill stays"
    );
    assert_eq!(common::pixel(&swapped, 10, 10), &blue);
}

#[test]
fn an_instance_inside_an_instance_is_swapped_by_its_path() {
    // Component 7 holds instance 8 of variant 2; instance 9 of component 7 sits at (40, 40).
    let mut holder = node(7, 0, "SYMBOL");
    holder.data.set("size", size(40.0, 40.0));
    let mut inner = instance(8, 7, 2, vec![]);
    inner.data.set("fillPaints", paint(1.0, 0.0, 0.0));
    let mut outer = instance(9, 1, 7, vec![]);
    outer.data.set(
        "transform",
        obj(&[("m02", Val::Float(40.0)), ("m12", Val::Float(40.0))]),
    );
    let doc = variants(vec![holder, inner, outer]);
    let swapped = render_with(&doc, &[(vec![id(9), id(8)], id(4))]);
    assert_eq!(
        common::pixel(&swapped, 50, 50),
        &[0, 0, 255, 255],
        "inside instance 9"
    );
    assert_eq!(
        common::pixel(&swapped, 10, 10),
        &[255, 0, 0, 255],
        "instance 6 unchanged"
    );
}

fn swap_to(to: figstudio_fig::Guid) -> Vec<(&'static str, Val)> {
    vec![
        ("transitionNodeID", to.to_val()),
        ("connectionType", name("INTERNAL_NODE")),
        ("navigationType", name("SWAP_STATE")),
    ]
}

/// A button set 20 with variants 21 (default) and 22 (hover); screen 30 holds instance 31 of
/// the default at (0, 0). `default` and `hover` are the variants' interactions.
fn button(default: Val, hover: Val) -> Document {
    let mut set = placed(20, 1, "FRAME", 500.0, 0.0);
    set.data.set("isStateGroup", Val::Bool(true));
    let mut a = node(21, 20, "SYMBOL");
    a.data.set("prototypeInteractions", default);
    let mut b = placed(22, 20, "SYMBOL", 50.0, 0.0);
    b.data.set("prototypeInteractions", hover);
    let mut screen = node(30, 1, "FRAME");
    screen.data.set("size", size(100.0, 100.0));
    document(vec![
        node(1, 0, "CANVAS"),
        set,
        a,
        b,
        screen,
        instance(31, 30, 21, vec![]),
    ])
}

fn read_with(doc: &Document, player: &Player) -> Screen {
    let mut renderer = Renderer::new();
    renderer.prototype = std::sync::Arc::new(player.state().clone());
    Screen::read(&mut renderer, doc, id(30))
}

#[test]
fn while_hovering_swaps_the_variant_and_leaving_restores_it() {
    use prototype::Pointer;
    let doc = button(interaction("ON_HOVER", &swap_to(id(22))), Val::Arr(vec![]));
    let mut player = Player::new(id(30));
    let screen = read_with(&doc, &player);
    assert_eq!(
        screen.links[0].step,
        Some(Step::Swap {
            target: Layer::Node(id(31)),
            to: id(22)
        })
    );
    assert!(player.pointer(&screen, Pointer::Move(Point::new(10.0, 10.0)), 0.0));
    assert_eq!(
        player.state().swaps.get(&vec![id(31)]),
        Some(&id(22)),
        "hovered"
    );
    let screen = read_with(&doc, &player);
    assert!(
        screen.links.is_empty(),
        "the hover variant's own (no) links"
    );
    assert!(
        !player.pointer(&screen, Pointer::Move(Point::new(12.0, 12.0)), 0.05),
        "still over the button: the hover holds though the new variant has no hover link"
    );
    assert_eq!(player.state().swaps.get(&vec![id(31)]), Some(&id(22)));
    assert!(player.pointer(&screen, Pointer::Move(Point::new(90.0, 90.0)), 0.1));
    assert!(player.state().swaps.is_empty(), "left: back to the default");
}

#[test]
fn a_click_swaps_for_good_and_after_delay_fires_on_time() {
    use prototype::Pointer;
    let delayed = {
        let mut i = interaction("AFTER_TIMEOUT", &swap_to(id(21)));
        if let Val::Arr(items) = &mut i {
            items[0]
                .get_mut("event")
                .unwrap()
                .set("transitionTimeout", Val::Float(0.5));
        }
        i
    };
    let doc = button(interaction("ON_CLICK", &swap_to(id(22))), delayed);
    let mut player = Player::new(id(30));
    let screen = read_with(&doc, &player);
    let at = Point::new(10.0, 10.0);
    assert!(
        !player.pointer(&screen, Pointer::Down(at), 1.0),
        "a press alone does nothing"
    );
    assert!(
        player.pointer(&screen, Pointer::Up(at), 1.1),
        "a click swaps"
    );
    let screen = read_with(&doc, &player);
    assert_eq!(player.next_due(&screen), Some(1.6));
    assert!(!player.tick(&screen, 1.5), "not yet");
    assert!(
        player.tick(&screen, 1.6),
        "after 0.5 s the hover variant swaps back"
    );
    assert_eq!(player.state().swaps.get(&vec![id(31)]), Some(&id(21)));
}

#[test]
fn the_hover_fixture_button_has_a_played_swap() {
    let doc = hover_doc();
    let screen = read(&doc, FIRST);
    let button = screen.links.iter().find(|l| l.layer == Layer::Node(BUTTON));
    assert_eq!(
        button.map(|l| l.step.clone()),
        Some(Some(Step::Swap {
            target: Layer::Node(BUTTON),
            to: HOVERED
        })),
        "{:?}",
        screen
            .links
            .iter()
            .map(|l| (l.layer.to_string(), l.trigger))
            .collect::<Vec<_>>()
    );
}
