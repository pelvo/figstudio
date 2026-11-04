//! Prototype flows, links and the player, on documents built in the test.
mod common;
#[path = "common/prototype.rs"]
mod proto;

use common::{document, id, instance, node, patch};
use figstudio_fig::{Document, Val};
use figstudio_render::Renderer;
use figstudio_render::kurbo::Point;
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
