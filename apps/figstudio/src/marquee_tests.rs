//! Marquee geometry: the rectangle of a drag in any direction, the minimum size, the region array.

use egui::{Pos2, Rect, pos2};

use super::*;

#[test]
fn a_drag_in_any_direction_gives_the_same_rectangle() {
    let (a, b) = (pos2(10.0, 80.0), pos2(60.0, 20.0));
    let want = Rect::from_min_max(pos2(10.0, 20.0), pos2(60.0, 80.0));
    assert_eq!(drag_rect(a, b), Some(want));
    assert_eq!(drag_rect(b, a), Some(want));
    assert_eq!(drag_rect(pos2(60.0, 80.0), pos2(10.0, 20.0)), Some(want));
}

#[test]
fn a_drag_with_a_bad_point_gives_nothing() {
    assert_eq!(drag_rect(Pos2::ZERO, pos2(f32::NAN, 1.0)), None);
    assert_eq!(drag_rect(pos2(f32::INFINITY, 0.0), Pos2::ZERO), None);
}

#[test]
fn big_enough_counts_screen_points_not_page_units() {
    let r = Rect::from_min_size(Pos2::ZERO, egui::vec2(10.0, 10.0));
    assert!(big_enough(r, 1.0));
    assert!(!big_enough(r, 0.3), "3 points wide on screen");
    assert!(big_enough(
        Rect::from_min_size(Pos2::ZERO, egui::vec2(2.0, 2.0)),
        4.0
    ));
    // One long, thin side is still a click-sized drag.
    assert!(!big_enough(
        Rect::from_min_size(Pos2::ZERO, egui::vec2(500.0, 1.0)),
        1.0
    ));
}

#[test]
fn region_array_is_origin_then_size() {
    let r = Rect::from_min_max(pos2(-5.0, 7.5), pos2(15.0, 27.5));
    assert_eq!(region_array(r), [-5.0, 7.5, 20.0, 20.0]);
}

#[test]
fn clear_forgets_the_rectangle_and_the_menu_choice() {
    let mut m = Marquee {
        rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(9.0, 9.0))),
        page: Some(figstudio_fig::Guid(1, 2)),
        anchor: Some(Pos2::ZERO),
        menu: true,
    };
    m.clear();
    assert!(m.rect.is_none() && m.page.is_none() && m.anchor.is_none() && !m.menu);
}
