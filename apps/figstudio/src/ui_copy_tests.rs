//! Right-click Copy menus, the marquee, click-to-select and Hand-tool pan, through the real canvas.
use egui::{Modifiers, vec2};
use figstudio_mcp_core::link::parse_link;

use super::*;

/// A rounded rectangle ("social-twitter 1") on the only page, away from other nodes.
const NODE: Guid = Guid(1, 34);

/// The TEXT node "ABC" inside the SYMBOL "Component 1".
const TEXT_NODE: Guid = Guid(1, 42);

fn menu_items(h: &Harness<'_, App>) -> Vec<&'static str> {
    [
        "Copy name",
        "Copy node ID",
        "Copy link",
        "Copy for agent",
        "Copy as PNG",
        "Copy as WebP",
        "Copy as SVG",
        "Copy CSS animation",
        "Copy region",
        "Copy region as PNG",
    ]
    .into_iter()
    .filter(|l| h.query_by_label(l).is_some())
    .collect()
}

#[test]
fn right_click_on_a_node_offers_the_copy_items_and_copies_the_link() {
    let (mut h, mut c) = (harness(), Vec::new());
    let at = centre_of(&h, NODE);
    right_click(&mut h, &mut c, at);
    h.step();
    assert_eq!(
        h.state().tabs[0].selection,
        Some(NODE),
        "right-click selects"
    );
    assert_eq!(
        menu_items(&h),
        [
            "Copy name",
            "Copy node ID",
            "Copy link",
            "Copy for agent",
            "Copy as PNG",
            "Copy as WebP",
            "Copy as SVG"
        ]
    );
    click_label(&mut h, &mut c, "Copy link");
    assert_eq!(c, ["figstudio://structure?node-id=1-34"]);
}

#[test]
fn copy_for_agent_is_the_three_line_block_and_the_other_items_copy_their_text() {
    let (mut h, mut c) = (harness(), Vec::new());
    let at = centre_of(&h, NODE);
    for label in ["Copy for agent", "Copy name", "Copy node ID"] {
        right_click(&mut h, &mut c, at);
        h.step();
        click_label(&mut h, &mut c, label);
    }
    let [agent, name, id] = &c[..] else {
        panic!("{c:?}")
    };
    let lines: Vec<&str> = agent.lines().collect();
    assert_eq!(lines.len(), 3, "{agent}");
    assert_eq!(lines[0], "figstudio://structure?node-id=1-34");
    assert_eq!(lines[1], "file: structure.fig · page: Page 1");
    assert!(
        lines[2].starts_with(r#"node: 1:34 "social-twitter 1" (ROUNDED_RECTANGLE) x="#),
        "{}",
        lines[2]
    );
    assert_eq!((name.as_str(), id.as_str()), ("social-twitter 1", "1:34"));
}

#[test]
fn copy_as_png_puts_an_image_on_the_clipboard() {
    let (mut h, mut c) = (harness(), Vec::new());
    let at = centre_of(&h, NODE);
    right_click(&mut h, &mut c, at);
    h.step();
    click_label(&mut h, &mut c, "Copy as PNG");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !c.iter().any(|t| t.starts_with("image ")) {
        assert!(Instant::now() < deadline, "no image copied: {c:?}");
        step(&mut h, &mut c);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Draws a marquee around `NODE`; returns its centre.
fn marquee_around_node(h: &mut Harness<'_, App>, c: &mut Vec<String>) -> Pos2 {
    let at = centre_of(h, NODE);
    drag(h, c, at - vec2(60.0, 60.0), at + vec2(60.0, 60.0));
    h.step();
    assert!(
        h.state().tabs[0].marquee.rect().is_some(),
        "a primary drag draws the marquee"
    );
    at
}

#[test]
fn a_marquee_offers_region_copy_with_a_region_link_and_its_nodes() {
    let (mut h, mut c) = (harness(), Vec::new());
    let at = marquee_around_node(&mut h, &mut c);
    right_click(&mut h, &mut c, at);
    h.step();
    assert_eq!(menu_items(&h), ["Copy region", "Copy region as PNG"]);
    click_label(&mut h, &mut c, "Copy region");
    let [text] = &c[..] else { panic!("{c:?}") };
    let link = parse_link(text.lines().next().unwrap()).unwrap();
    assert_eq!(link.page, Some(h.state().tabs[0].page));
    let rect = link.rect.expect("a rect");
    assert!(rect[2] > 100.0 && rect[3] > 100.0, "{rect:?}");
    assert!(
        text.contains("\nfile: structure.fig · page: Page 1\nnodes ("),
        "{text}"
    );
    assert!(
        text.contains("1:34 \"social-twitter 1\" (ROUNDED_RECTANGLE)"),
        "{text}"
    );
}

#[test]
fn region_png_copies_an_image_and_a_right_click_outside_the_marquee_opens_the_node_menu() {
    let (mut h, mut c) = (harness(), Vec::new());
    let at = marquee_around_node(&mut h, &mut c);
    right_click(&mut h, &mut c, at);
    h.step();
    click_label(&mut h, &mut c, "Copy region as PNG");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !c.iter().any(|t| t.starts_with("image ")) {
        assert!(Instant::now() < deadline, "no image copied: {c:?}");
        step(&mut h, &mut c);
        std::thread::sleep(Duration::from_millis(10));
    }
    // Far from the marquee, over another node: the node menu.
    let other = centre_of(&h, Guid(1, 2));
    assert!(other.distance(at) > 90.0);
    right_click(&mut h, &mut c, other);
    h.step();
    assert!(menu_items(&h).contains(&"Copy link") && !menu_items(&h).contains(&"Copy region"));
}

#[test]
fn a_plain_click_selects_and_does_not_start_a_marquee() {
    let (mut h, mut c) = (harness(), Vec::new());
    assert_eq!(h.state().tabs[0].selection, None);
    let at = centre_of(&h, NODE);
    h.event(Event::PointerMoved(at));
    step(&mut h, &mut c);
    press(&h, at, PointerButton::Primary, true);
    press(&h, at, PointerButton::Primary, false);
    h.run_steps(3);
    assert_eq!(h.state().tabs[0].selection, Some(NODE));
    assert!(h.state().tabs[0].marquee.rect().is_none());
}

#[test]
fn command_c_copies_the_characters_of_a_selected_text_layer() {
    let (mut h, mut c) = (harness(), Vec::new());
    let at = centre_of(&h, TEXT_NODE);
    pick::click(&mut h, at, Modifiers::COMMAND);
    assert_eq!(h.state().tabs[0].selection, Some(TEXT_NODE));
    h.event(Event::Copy);
    step(&mut h, &mut c);
    assert_eq!(c, ["ABC"], "⌘C copies the text layer's characters");
}

#[test]
fn command_c_does_nothing_without_a_text_layer_selected() {
    let (mut h, mut c) = (harness(), Vec::new());
    let at = centre_of(&h, NODE);
    h.event(Event::PointerMoved(at));
    step(&mut h, &mut c);
    press(&h, at, PointerButton::Primary, true);
    press(&h, at, PointerButton::Primary, false);
    h.run_steps(3);
    assert_eq!(h.state().tabs[0].selection, Some(NODE));
    h.event(Event::Copy);
    step(&mut h, &mut c);
    assert!(c.is_empty(), "a non-text selection copies nothing: {c:?}");
}

#[test]
fn a_hand_tool_drag_pans_without_a_marquee() {
    let (mut h, mut c) = (harness(), Vec::new());
    h.get_by_label("Hand tool").click();
    h.run_steps(3);
    let before = h.state().tabs[0].view;
    let at = centre_of(&h, NODE);
    drag(&mut h, &mut c, at, at + vec2(50.0, 30.0));
    h.step();
    let after = h.state().tabs[0].view;
    assert_eq!(after.zoom, before.zoom);
    assert!(
        (after.pan - before.pan - vec2(50.0, 30.0)).length() < 8.0,
        "{:?} -> {:?}",
        before.pan,
        after.pan
    );
    assert!(h.state().tabs[0].marquee.rect().is_none());
    assert_eq!(h.state().tabs[0].selection, None);
}

#[test]
fn copy_as_svg_puts_the_markup_on_the_clipboard() {
    let (mut h, mut c) = (harness(), Vec::new());
    let at = centre_of(&h, NODE);
    right_click(&mut h, &mut c, at);
    h.step();
    click_label(&mut h, &mut c, "Copy as SVG");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !c.iter().any(|t| t.starts_with("<svg xmlns=")) {
        assert!(Instant::now() < deadline, "no SVG copied: {c:?}");
        step(&mut h, &mut c);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn an_animated_layer_offers_and_copies_its_css_animation() {
    let mut h = harness_fixture("motion.fig", 1.0 / 60.0);
    let mut c = Vec::new();
    // The rotating "Object" on the playground page.
    let object = Guid(1, 1106);
    let at = centre_of(&h, object);
    right_click(&mut h, &mut c, at);
    h.step();
    assert_eq!(h.state().tabs[0].selection, Some(object));
    click_label(&mut h, &mut c, "Copy CSS animation");
    let css = c.last().expect("CSS copied");
    assert!(css.contains("@keyframes n1-1106-rotate"), "{css}");
    assert!(css.contains("animation: n1-1106-"), "{css}");
}
