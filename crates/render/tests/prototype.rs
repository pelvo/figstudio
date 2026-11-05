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
        (hover.trigger, hover.action, hover.step),
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
        .map(|l| (l.layer.to_string(), l.step))
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
    assert_eq!((link.destination, link.step), (Some(id(777)), None));
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
        hit.and_then(|l| l.step),
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
        (first.links[0].destination, first.links[0].step),
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
