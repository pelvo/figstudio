use super::*;
use crate::canvas::Tool;
use egui::{Event, Modifiers, PointerButton, Vec2};

fn walks() -> usize {
    crate::canvas::HIT_CHAIN_CALLS.with(|calls| calls.replace(0))
}

fn drag_without_walks(tool: Tool) {
    let mut h = inner_fixture::harness();
    h.state_mut().tool = tool;
    let pan = h.state().tabs[0].view.pan;
    let start = h.state().tabs[0].canvas_rect.unwrap().right_bottom() - Vec2::splat(100.0);
    h.event(Event::PointerMoved(start));
    h.event(Event::PointerButton {
        pos: start,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(1);
    h.event(Event::PointerMoved(start - Vec2::splat(20.0)));
    h.run_steps(1);
    walks();
    for i in 1..=8 {
        h.event(Event::PointerMoved(
            start - Vec2::new(20.0 + i as f32 * 3.0, 20.0),
        ));
        h.run_steps(1);
    }
    if tool == Tool::Hand {
        assert_ne!(h.state().tabs[0].view.pan, pan);
    } else {
        assert!(h.state().tabs[0].marquee.rect().is_some());
    }
    assert_eq!(walks(), 0, "{tool:?} drag must not inspect the scene");
    h.event(Event::PointerButton {
        pos: start - Vec2::new(44.0, 20.0),
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    });
    h.run_steps(1);
}

#[test]
fn marquee_drag_does_not_walk_hit_chain() {
    drag_without_walks(Tool::Select);
}

#[test]
fn hand_drag_does_not_walk_hit_chain() {
    drag_without_walks(Tool::Hand);
}

#[test]
fn stationary_hover_walks_hit_chain_at_most_once_per_frame() {
    let mut h = inner_fixture::harness();
    let at = centre_of(&h, inner_fixture::id(200));
    h.event(Event::PointerMoved(at));
    h.run_steps(1);
    walks();
    for _ in 0..8 {
        h.run_steps(1);
        assert_eq!(walks(), 1, "hover needs one shared chain per frame");
    }
}
