//! Headless egui regression tests for interactive canvas gizmos, overlay input isolation,
//! rotated resize handles, marquee coexistence, and UI edit error feedback.

use egui::{Event, PointerButton, Pos2, pos2, vec2};
use egui_kittest::kittest::Queryable;
use figstudio_fig::Guid;
use figstudio_fig::edit::EditOp;
use figstudio_render::kurbo::Point;
use figstudio_render::{absolute_transform, geom};

use super::*;
use crate::canvas::Tool;
use crate::menu::Action;

/// A standalone rounded rectangle ("social-twitter 1") on the fixture's page.
const NODE: Guid = Guid(1, 34);

fn node_local_screen(h: &Harness<'_, App>, id: Guid, local: Point) -> Pos2 {
    let tab = &h.state().tabs[0];
    let p = absolute_transform(&tab.doc, id) * local;
    tab.view.to_screen(
        tab.canvas_rect.expect("canvas laid out").min,
        pos2(p.x as f32, p.y as f32),
    )
}

fn node_page_point(h: &Harness<'_, App>, id: Guid, local: Point) -> Point {
    let tab = &h.state().tabs[0];
    absolute_transform(&tab.doc, id) * local
}

fn drag_frames(h: &mut Harness<'_, App>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.step();
    press(h, from, PointerButton::Primary, true);
    h.step();
    for i in 1..=3 {
        let p = from + (to - from) * (i as f32 / 3.0);
        h.event(Event::PointerMoved(p));
        h.step();
    }
    press(h, to, PointerButton::Primary, false);
    h.step();
}

#[test]
fn rotated_layer_resize_drags_oriented_edge_handle_and_keeps_opposite_edge() {
    for (degrees, w, h_dim, handle_local, keep_local, drag_local_delta) in [
        // 90° clockwise: local N edge (w/2, 0) lies on the right edge on screen; dragging
        // outward in page space expands height while keeping local S edge fixed.
        (
            90.0_f64,
            80.0_f64,
            40.0_f64,
            Point::new(40.0, 0.0),
            Point::new(40.0, 40.0),
            (0.0_f64, -20.0_f64),
        ),
        // 45° clockwise: local E edge (w, h/2) sits inside the AABB away from AABB handles;
        // dragging along local +X expands width while keeping local W edge fixed.
        (
            45.0_f64,
            100.0_f64,
            40.0_f64,
            Point::new(100.0, 20.0),
            Point::new(0.0, 20.0),
            (24.0_f64, 0.0_f64),
        ),
    ] {
        let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
        let page = h.state().tabs[0].page;
        let ids = crate::editor::apply_tab_ops(
            &mut h.state_mut().tabs[0],
            &[EditOp::CreateShape {
                id: None,
                kind: "RECTANGLE".into(),
                parent: page,
                name: None,
                x: 200.0,
                y: 200.0,
                width: w,
                height: h_dim,
                corner_radius: 0.0,
                fill_hex: None,
                count: None,
                star_inner_scale: None,
            }],
            Some("Create rotated rect"),
            None,
        )
        .unwrap();
        let rect_id = ids[0];
        crate::editor::apply_tab_ops(
            &mut h.state_mut().tabs[0],
            &[EditOp::Rotate {
                target: rect_id,
                degrees,
            }],
            Some("Rotate"),
            None,
        )
        .unwrap();
        h.state_mut().tabs[0].select(Some(rect_id));
        h.state_mut().tabs[0].fit_node = Some(rect_id);
        h.run_steps(4);

        let kept_before = node_page_point(&h, rect_id, keep_local);
        let handle_before = node_local_screen(&h, rect_id, handle_local);
        let target_local = Point::new(
            handle_local.x + drag_local_delta.0,
            handle_local.y + drag_local_delta.1,
        );
        let handle_after = node_local_screen(&h, rect_id, target_local);

        drag_frames(&mut h, handle_before, handle_after);

        let (nw, nh) = geom::size(&h.state().tabs[0].doc.node(rect_id).unwrap().data);
        let want_w = w + drag_local_delta.0.abs();
        let want_h = h_dim + drag_local_delta.1.abs();
        assert!(
            (nw - want_w).abs() <= 2.0 && (nh - want_h).abs() <= 2.0,
            "rot={degrees}°: expected ~{want_w}x{want_h}, got {nw}x{nh}"
        );
        let kept_local_after = Point::new(
            if keep_local.x > 0.0 && drag_local_delta.0 != 0.0 {
                nw
            } else if keep_local.x > 0.0 {
                nw / 2.0
            } else {
                0.0
            },
            if keep_local.y > 0.0 && drag_local_delta.1 != 0.0 {
                nh
            } else if keep_local.y > 0.0 {
                nh / 2.0
            } else {
                0.0
            },
        );
        let kept_after = node_page_point(&h, rect_id, kept_local_after);
        assert!(
            (kept_after - kept_before).hypot() <= 2.0,
            "rot={degrees}°: opposite edge moved from {kept_before:?} to {kept_after:?}"
        );
    }
}

#[test]
fn clicking_tool_pill_with_creation_tool_switches_tool_without_creating_shape() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    let ctx = h.ctx.clone();
    h.state_mut()
        .apply(Action::SelectTool(Tool::Rectangle), &ctx);
    h.run_steps(3);
    let nodes_before = h.state().tabs[0].doc.nodes.len();
    assert!(!h.state().tabs[0].dirty_unsaved);

    let select_btn = h.get_by_label("Select tool").rect().center();
    h.event(Event::PointerMoved(select_btn));
    h.step();
    press(&h, select_btn, PointerButton::Primary, true);
    h.step();
    assert!(
        h.state().tabs[0].edit_drag.is_none(),
        "pressing Select on the tool pill must not start CanvasDrag::Create"
    );
    press(&h, select_btn, PointerButton::Primary, false);
    h.run_steps(2);

    assert_eq!(h.state().tool, Tool::Select);
    assert_eq!(h.state().tabs[0].doc.nodes.len(), nodes_before);
    assert!(!h.state().tabs[0].dirty_unsaved);
}

#[test]
fn clicking_and_dragging_inside_settings_and_command_bar_does_not_edit_canvas() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    let ctx = h.ctx.clone();
    let canvas_rect = h.state().tabs[0].canvas_rect.unwrap();
    h.state_mut()
        .apply(Action::SelectTool(Tool::Rectangle), &ctx);
    h.state_mut().apply(Action::Settings, &ctx);
    h.run_steps(3);
    let nodes_before = h.state().tabs[0].doc.nodes.len();

    // Drag across the middle of Settings where it overlaps the canvas behind it.
    let inside_settings = canvas_rect.center();
    drag_frames(&mut h, inside_settings, inside_settings + vec2(40.0, 30.0));
    assert_eq!(h.state().tabs[0].doc.nodes.len(), nodes_before);
    assert!(!h.state().tabs[0].dirty_unsaved);
    assert!(h.state().tabs[0].edit_drag.is_none());

    // Close Settings, open Command Bar, and drag across it over the canvas.
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    h.state_mut().apply(Action::CommandBar, &ctx);
    h.run_steps(3);
    let inside_command = h
        .get_by_label("Search or paste a Figma link")
        .rect()
        .center();
    assert!(canvas_rect.contains(inside_command));
    drag_frames(&mut h, inside_command, inside_command + vec2(40.0, 20.0));
    assert_eq!(h.state().tabs[0].doc.nodes.len(), nodes_before);
    assert!(!h.state().tabs[0].dirty_unsaved);
    assert!(h.state().tabs[0].edit_drag.is_none());
}

#[test]
fn dragging_selected_layer_moves_it_without_leaving_a_stuck_marquee() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    h.state_mut().tabs[0].select(Some(NODE));
    h.run_steps(3);

    let before_pos = node_page_point(&h, NODE, Point::ZERO);
    let start = centre_of(&h, NODE);
    let mid = start + vec2(20.0, 10.0);
    let end = start + vec2(40.0, 20.0);

    h.event(Event::PointerMoved(start));
    h.step();
    press(&h, start, PointerButton::Primary, true);
    h.step();
    h.event(Event::PointerMoved(mid));
    h.step();
    assert!(
        h.state().tabs[0].marquee.rect().is_none(),
        "dragging a selected layer must not populate tab.marquee.rect mid-drag"
    );
    h.event(Event::PointerMoved(end));
    h.step();
    press(&h, end, PointerButton::Primary, false);
    h.run_steps(2);

    let after_pos = node_page_point(&h, NODE, Point::ZERO);
    assert!(
        (after_pos - before_pos).hypot() > 5.0,
        "selected layer should have moved: {before_pos:?} -> {after_pos:?}"
    );
    assert!(
        h.state().tabs[0].marquee.rect().is_none(),
        "no stale marquee rectangle may remain after releasing a layer drag"
    );
}

#[test]
fn clicking_outer_half_of_resize_handle_keeps_layer_selected() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    h.state_mut().tabs[0].select(Some(NODE));
    h.run_steps(3);

    let (w, h_dim) = geom::size(&h.state().tabs[0].doc.node(NODE).unwrap().data);
    let se = node_local_screen(&h, NODE, Point::new(w, h_dim));
    // Inside the 10x10 handle hit rect centered on SE, strictly outside the node onto empty canvas.
    let outer_handle = se + vec2(3.0, 3.0);

    h.event(Event::PointerMoved(outer_handle));
    h.step();
    press(&h, outer_handle, PointerButton::Primary, true);
    h.step();
    press(&h, outer_handle, PointerButton::Primary, false);
    h.run_steps(2);

    assert_eq!(
        h.state().tabs[0].selection,
        Some(NODE),
        "clicking a resize handle extending over empty canvas must not deselect the layer"
    );
}

#[test]
fn invalid_inspector_fill_hex_surfaces_an_error_toast() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    h.state_mut().tabs[0].select(Some(NODE));
    h.run_steps(3);
    h.get_by_label("Edit Layer").click();
    h.run_steps(3);
    assert!(h.state().toasts.is_empty());

    h.get_by_label("Fill colour hex").click();
    h.step();
    h.key_press(egui::Key::End);
    for _ in 0..8 {
        h.key_press(egui::Key::Backspace);
    }
    h.get_by_label("Fill colour hex").type_text("#ZZZZZZ");
    h.run_steps(3);

    assert!(
        h.state().toasts.iter().any(|t| t.error),
        "invalid fill hex in inspector must surface an error toast instead of dropping Err"
    );
}

#[test]
fn dragging_from_canvas_pills_and_status_pill_does_not_leak_to_canvas() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    let ctx = h.ctx.clone();
    let pan_before = h.state().tabs[0].view.pan;
    let nodes_before = h.state().tabs[0].doc.nodes.len();

    // 1. With Select tool, pressing on tool_pill and dragging > 6 pt must not start a marquee.
    let select_btn = h.get_by_label("Select tool").rect().center();
    drag_frames(&mut h, select_btn, select_btn + vec2(24.0, -24.0));
    assert!(
        h.state().tabs[0].marquee.rect().is_none(),
        "dragging off tool_pill must not leak a marquee drag onto the canvas"
    );
    assert_eq!(h.state().tabs[0].view.pan, pan_before);

    // 2. With Rectangle creation tool, clicking/dragging on status_pill must not create a shape.
    let status_pt = h.get_by_label_contains("layers drawn").rect().center();
    h.state_mut()
        .apply(Action::SelectTool(Tool::Rectangle), &ctx);
    h.run_steps(2);
    drag_frames(&mut h, status_pt, status_pt + vec2(30.0, -20.0));
    assert_eq!(
        h.state().tabs[0].doc.nodes.len(),
        nodes_before,
        "clicking or dragging status_pill with Rectangle tool must not create a shape"
    );
    assert!(h.state().tabs[0].edit_drag.is_none());
    assert!(!h.state().tabs[0].dirty_unsaved);

    // 3. With Select tool, dragging from status_pill must not start a marquee.
    h.state_mut().apply(Action::SelectTool(Tool::Select), &ctx);
    h.run_steps(2);
    drag_frames(&mut h, status_pt, status_pt + vec2(30.0, -20.0));
    assert!(
        h.state().tabs[0].marquee.rect().is_none(),
        "dragging from status_pill must not start a marquee on the canvas"
    );
}

#[test]
fn open_link_window_preserves_nonmodal_canvas_drag_and_modal_activation_blocks_same_frame_click() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    let ctx = h.ctx.clone();
    h.state_mut().tabs[0].select(Some(NODE));
    h.run_steps(3);

    // Open the non-modal "Open Figma link" window and place a layer drag outside it.
    h.state_mut().apply(Action::OpenLink, &ctx);
    h.run_steps(3);
    assert!(h.state().link.window);
    let link_rect = h
        .ctx
        .memory(|m| m.area_rect(egui::Id::new("open-figma-link")))
        .expect("Open Figma link window laid out");
    // Pan NODE away from the floating window if it happens to overlap.
    let mut start = centre_of(&h, NODE);
    if link_rect.contains(start) {
        h.state_mut().tabs[0].view.pan += vec2(0.0, 220.0);
        h.run_steps(2);
        start = centre_of(&h, NODE);
    }
    assert!(
        !link_rect.contains(start),
        "test setup requires NODE ({start:?}) outside Open Figma link ({link_rect:?})"
    );
    let before = node_page_point(&h, NODE, Point::ZERO);
    drag_frames(&mut h, start, start + vec2(30.0, 15.0));
    let after = node_page_point(&h, NODE, Point::ZERO);
    assert!(
        (after - before).hypot() > 5.0,
        "non-modal Open Figma link window must not disable dragging a selected layer on canvas"
    );
    assert!(
        h.state().tabs[0].marquee.rect().is_none(),
        "dragging a selected layer with Open Figma link open must move it, not draw a marquee"
    );

    // Close Open Figma link; now open CommandBar and send a canvas click in the SAME frame
    // before the command-backdrop layer has entered prev_pass.widgets.
    h.state_mut().link.window = false;
    h.run_steps(2);
    let selected_before = h.state().tabs[0].selection;
    let empty_pt = h.state().tabs[0].canvas_rect.unwrap().left_top() + vec2(20.0, 20.0);
    h.state_mut().apply(Action::CommandBar, &ctx);
    h.input_mut().events.push(Event::PointerMoved(empty_pt));
    h.input_mut().events.push(Event::PointerButton {
        pos: empty_pt,
        button: PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.input_mut().events.push(Event::PointerButton {
        pos: empty_pt,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    assert_eq!(
        h.state().tabs[0].selection,
        selected_before,
        "a canvas click in the same frame CommandBar opens must not clear selection"
    );
}

#[test]
fn escape_cancels_active_create_move_and_resize_drags_without_clearing_selection() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    let ctx = h.ctx.clone();
    let canvas_rect = h.state().tabs[0].canvas_rect.unwrap();
    let nodes_before = h.state().tabs[0].doc.nodes.len();

    // 1. Escape mid-Create cancels creation without adding a node.
    h.state_mut()
        .apply(Action::SelectTool(Tool::Rectangle), &ctx);
    h.run_steps(2);
    let c0 = canvas_rect.left_top() + vec2(60.0, 60.0);
    let c1 = c0 + vec2(80.0, 50.0);
    h.event(Event::PointerMoved(c0));
    h.step();
    press(&h, c0, PointerButton::Primary, true);
    h.step();
    h.event(Event::PointerMoved(c1));
    h.step();
    assert!(h.state().tabs[0].edit_drag.is_some());
    h.key_press(egui::Key::Escape);
    h.step();
    press(&h, c1 + vec2(20.0, 20.0), PointerButton::Primary, false);
    h.run_steps(2);
    assert_eq!(
        h.state().tabs[0].doc.nodes.len(),
        nodes_before,
        "Escape during Create must cancel without creating a shape"
    );
    assert!(h.state().tabs[0].edit_drag.is_none());
    assert!(!h.state().tabs[0].dirty_unsaved);

    // 2. Escape mid-Move reverts coordinates, preserves selection, and keeps undo clean.
    h.state_mut().apply(Action::SelectTool(Tool::Select), &ctx);
    h.state_mut().tabs[0].select(Some(NODE));
    h.run_steps(3);
    let orig_pos = node_page_point(&h, NODE, Point::ZERO);
    let start = centre_of(&h, NODE);
    h.event(Event::PointerMoved(start));
    h.step();
    press(&h, start, PointerButton::Primary, true);
    h.step();
    h.event(Event::PointerMoved(start + vec2(30.0, 15.0)));
    h.step();
    assert!((node_page_point(&h, NODE, Point::ZERO) - orig_pos).hypot() > 5.0);
    h.key_press(egui::Key::Escape);
    h.step();
    // Moving further while button stays held must not keep mutating the layer.
    h.event(Event::PointerMoved(start + vec2(70.0, 40.0)));
    h.step();
    press(&h, start + vec2(70.0, 40.0), PointerButton::Primary, false);
    h.run_steps(2);
    assert!(
        (node_page_point(&h, NODE, Point::ZERO) - orig_pos).hypot() < 1e-3,
        "Escape mid-Move must revert the layer to its pre-drag coordinates"
    );
    assert_eq!(
        h.state().tabs[0].selection,
        Some(NODE),
        "Escape mid-Move must keep the layer selected"
    );
    assert_eq!(h.state().tabs[0].history.undo_len(), 0);
    assert!(!h.state().tabs[0].dirty_unsaved);
    assert!(h.state().tabs[0].edit_drag.is_none());

    // 3. Escape mid-Resize reverts size, preserves selection, and keeps undo clean.
    let (orig_w, orig_h) = geom::size(&h.state().tabs[0].doc.node(NODE).unwrap().data);
    let se = node_local_screen(&h, NODE, Point::new(orig_w, orig_h));
    h.event(Event::PointerMoved(se));
    h.step();
    press(&h, se, PointerButton::Primary, true);
    h.step();
    h.event(Event::PointerMoved(se + vec2(30.0, 20.0)));
    h.step();
    let (mid_w, _) = geom::size(&h.state().tabs[0].doc.node(NODE).unwrap().data);
    assert!((mid_w - orig_w).abs() > 5.0);
    h.key_press(egui::Key::Escape);
    h.step();
    h.event(Event::PointerMoved(se + vec2(60.0, 40.0)));
    h.step();
    press(&h, se + vec2(60.0, 40.0), PointerButton::Primary, false);
    h.run_steps(2);
    let (after_w, after_h) = geom::size(&h.state().tabs[0].doc.node(NODE).unwrap().data);
    assert!(
        (after_w - orig_w).abs() < 1e-3 && (after_h - orig_h).abs() < 1e-3,
        "Escape mid-Resize must revert size ({orig_w}x{orig_h}), got {after_w}x{after_h}"
    );
    assert_eq!(h.state().tabs[0].selection, Some(NODE));
    assert_eq!(h.state().tabs[0].history.undo_len(), 0);
    assert!(!h.state().tabs[0].dirty_unsaved);
}

#[test]
fn two_frame_move_and_resize_drags_apply_final_release_position() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    h.state_mut().tabs[0].select(Some(NODE));
    h.run_steps(3);

    // 1. Two-frame Move: Frame 1 press at start, Frame 2 move + release at end.
    let before = node_page_point(&h, NODE, Point::ZERO);
    let start = centre_of(&h, NODE);
    let end = start + vec2(40.0, 20.0);
    let delta_page = {
        let tab = &h.state().tabs[0];
        let origin = tab.canvas_rect.unwrap().min;
        tab.view.to_page(origin, end) - tab.view.to_page(origin, start)
    };
    h.event(Event::PointerMoved(start));
    h.step();
    press(&h, start, PointerButton::Primary, true);
    h.step();
    h.input_mut().events.push(Event::PointerMoved(end));
    h.input_mut().events.push(Event::PointerButton {
        pos: end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    let after = node_page_point(&h, NODE, Point::ZERO);
    assert!(
        ((after.x - before.x) - f64::from(delta_page.x)).abs() <= 1.5
            && ((after.y - before.y) - f64::from(delta_page.y)).abs() <= 1.5,
        "two-frame move must apply final release position: before={before:?} after={after:?} delta={delta_page:?}"
    );
    assert_eq!(h.state().tabs[0].history.undo_len(), 1);
    assert!(h.state().tabs[0].edit_drag.is_none());

    // 2. Two-frame Resize: Frame 1 press on SE handle, Frame 2 move + release.
    let (orig_w, orig_h) = geom::size(&h.state().tabs[0].doc.node(NODE).unwrap().data);
    let se = node_local_screen(&h, NODE, Point::new(orig_w, orig_h));
    let se_end = se + vec2(30.0, 20.0);
    let resize_page = {
        let tab = &h.state().tabs[0];
        let origin = tab.canvas_rect.unwrap().min;
        tab.view.to_page(origin, se_end) - tab.view.to_page(origin, se)
    };
    h.event(Event::PointerMoved(se));
    h.step();
    press(&h, se, PointerButton::Primary, true);
    h.step();
    h.input_mut().events.push(Event::PointerMoved(se_end));
    h.input_mut().events.push(Event::PointerButton {
        pos: se_end,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    let (after_w, after_h) = geom::size(&h.state().tabs[0].doc.node(NODE).unwrap().data);
    assert!(
        ((after_w - orig_w) - f64::from(resize_page.x)).abs() <= 1.5
            && ((after_h - orig_h) - f64::from(resize_page.y)).abs() <= 1.5,
        "two-frame resize must apply final release size: {orig_w}x{orig_h} -> {after_w}x{after_h}, delta={resize_page:?}"
    );
    assert_eq!(h.state().tabs[0].history.undo_len(), 2);
    assert!(h.state().tabs[0].edit_drag.is_none());
}

#[test]
fn interrupting_move_drag_with_space_or_tool_switch_seals_history_and_stops_drag() {
    let mut h = harness_fixture(FIXTURE, 1.0 / 60.0);
    let ctx = h.ctx.clone();
    h.state_mut().tabs[0].select(Some(NODE));
    h.run_steps(3);

    // Drag NODE, then hold Space mid-drag to interrupt with panning.
    let start = centre_of(&h, NODE);
    h.event(Event::PointerMoved(start));
    h.step();
    press(&h, start, PointerButton::Primary, true);
    h.step();
    h.event(Event::PointerMoved(start + vec2(20.0, 0.0)));
    h.step();
    assert_eq!(h.state().tabs[0].history.undo_len(), 1);
    h.input_mut().events.push(Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    assert!(
        h.state().tabs[0].edit_drag.is_none(),
        "Space/panning mid-drag must clear active edit_drag"
    );
    h.input_mut().events.push(Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    press(&h, start + vec2(20.0, 0.0), PointerButton::Primary, false);
    h.run_steps(2);

    // A second drag must create a second distinct undo entry.
    let start2 = centre_of(&h, NODE);
    drag_frames(&mut h, start2, start2 + vec2(20.0, 0.0));
    assert_eq!(
        h.state().tabs[0].history.undo_len(),
        2,
        "a move after a Space-interrupted drag must start a fresh undo step"
    );

    // Now interrupt a Move drag with a tool switch to Hand, then back to Select.
    let start3 = centre_of(&h, NODE);
    h.event(Event::PointerMoved(start3));
    h.step();
    press(&h, start3, PointerButton::Primary, true);
    h.step();
    h.event(Event::PointerMoved(start3 + vec2(20.0, 0.0)));
    h.step();
    assert_eq!(h.state().tabs[0].history.undo_len(), 3);
    h.state_mut().apply(Action::SelectTool(Tool::Hand), &ctx);
    h.step();
    assert!(
        h.state().tabs[0].edit_drag.is_none(),
        "switching to Hand mid-drag must clear active edit_drag"
    );
    h.state_mut().apply(Action::SelectTool(Tool::Select), &ctx);
    press(&h, start3 + vec2(20.0, 0.0), PointerButton::Primary, false);
    h.run_steps(2);

    let start4 = centre_of(&h, NODE);
    drag_frames(&mut h, start4, start4 + vec2(20.0, 0.0));
    assert_eq!(
        h.state().tabs[0].history.undo_len(),
        4,
        "a move after a tool-switch interrupt must start a fresh undo step"
    );
}
