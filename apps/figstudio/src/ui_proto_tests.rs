//! Boolean and mask buttons and the Prototype section on the real app.
#![expect(non_snake_case, reason = "A, B and F name the layers")]
use figstudio_fig::edit::{Action, Trigger, reactions_of};

use super::design::type_into;
use super::multi_select::layout;
use super::*;

fn click(h: &mut Harness<'_, App>, label: &str) {
    reveal(h, label);
    h.get_by_label(label).click();
    h.run_steps(3);
}

fn pick(h: &mut Harness<'_, App>, combo: &str, option: &str) {
    click(h, combo);
    h.get_by_label(option).click();
    h.run_steps(3);
}

fn select(h: &mut Harness<'_, App>, ids: &[Guid]) {
    h.state_mut().tabs[0].select_all_of(ids);
    h.run_steps(3);
}

#[test]
fn boolean_buttons_combine_and_a_mask_changes_kind_and_stops() {
    let mut h = harness();
    let (A, B, _) = layout(&mut h);
    select(&mut h, &[A, B]);
    click(&mut h, "Union selection");
    let sel = h.state().tabs[0].selection.unwrap();
    assert_eq!(
        h.state().tabs[0].doc.node(sel).unwrap().kind,
        "BOOLEAN_OPERATION"
    );
    h.state_mut().tabs[0].history.seal_coalesce();
    crate::editor::undo_tab(&mut h.state_mut().tabs[0]).unwrap();
    select(&mut h, &[A, B]);
    click(&mut h, "Mask with the bottom layer");
    let flag = |h: &Harness<'_, App>, f: &str| h.state().tabs[0].doc.node(A).unwrap().data.flag(f);
    assert_eq!(flag(&h, "mask"), Some(true), "the bottom layer masks");
    select(&mut h, &[A]);
    pick(&mut h, "Mask type", "Outline");
    assert_eq!(flag(&h, "maskIsOutline"), Some(true));
    click(&mut h, "Stop masking");
    assert_eq!(flag(&h, "mask"), Some(false));
}

#[test]
fn interactions_are_added_and_edited_from_the_panel() {
    let mut h = harness();
    let (A, _, F) = layout(&mut h);
    select(&mut h, &[A]);
    click(&mut h, "Add interaction");
    let read = |h: &Harness<'_, App>| reactions_of(&h.state().tabs[0].doc, A).unwrap();
    assert_eq!(read(&h).len(), 1);
    assert_eq!(read(&h)[0].trigger, Trigger::OnClick);
    assert_eq!(read(&h)[0].action, Action::Navigate { to: F });
    pick(&mut h, "Interaction 1 trigger", "While hovering");
    assert_eq!(read(&h)[0].trigger, Trigger::OnHover);
    pick(&mut h, "Interaction 1 animation", "Dissolve");
    assert_eq!(read(&h)[0].transition.as_deref(), Some("dissolve"));
    type_into(&mut h, "Interaction 1 duration", "500");
    assert!((read(&h)[0].duration.unwrap() - 0.5).abs() < 1e-6);
    pick(&mut h, "Interaction 1 action", "Back");
    assert_eq!(read(&h)[0].action, Action::Back);
    // A second one keeps the first.
    click(&mut h, "Add interaction");
    assert_eq!(read(&h).len(), 2);
    assert_eq!(read(&h)[0].action, Action::Back);
    click(&mut h, "Remove interaction 1");
    assert_eq!(read(&h).len(), 1);
    assert_eq!(read(&h)[0].action, Action::Navigate { to: F });
}

#[test]
fn a_frame_starts_a_flow_and_scrolls() {
    let mut h = harness();
    let (_, _, F) = layout(&mut h);
    select(&mut h, &[F]);
    click(&mut h, "Flow starting point");
    let data = |h: &Harness<'_, App>| h.state().tabs[0].doc.node(F).unwrap().data.clone();
    assert!(data(&h).get("prototypeStartingPoint").is_some());
    pick(&mut h, "Overflow scrolling", "Vertical");
    assert_eq!(data(&h).text("scrollDirection"), Some("VERTICAL"));
}

#[test]
fn the_selection_connects_to_its_destination_on_the_canvas() {
    let mut h = harness();
    let (A, _, F) = layout(&mut h);
    crate::editor::apply_tab_ops(
        &mut h.state_mut().tabs[0],
        &[figstudio_fig::edit::EditOp::SetReactions {
            target: A,
            reactions: vec![figstudio_fig::edit::ReactionSpec {
                trigger: Trigger::OnClick,
                action: Action::Navigate { to: F },
                transition: None,
                duration: None,
                easing: None,
            }],
        }],
        None,
        None,
    )
    .unwrap();
    select(&mut h, &[A]);
    let tab = &h.state().tabs[0];
    let origin = tab.canvas_rect.unwrap().min;
    let lines = crate::editor::proto_connections(tab, origin);
    assert_eq!(lines.len(), 1);
    let (a, z) = lines[0];
    let to_screen = |x: f64, y: f64| tab.view.to_screen(origin, Pos2::new(x as f32, y as f32));
    let (ba, bf) = (
        figstudio_render::node_bounds(&tab.doc, A).unwrap(),
        figstudio_render::node_bounds(&tab.doc, F).unwrap(),
    );
    assert_eq!(a, to_screen(ba.x1, ba.center().y));
    assert_eq!(z, to_screen(bf.x0, bf.center().y));
    // Nothing selected: nothing drawn.
    h.state_mut().tabs[0].select(None);
    let tab = &h.state().tabs[0];
    assert!(crate::editor::proto_connections(tab, origin).is_empty());
}

#[test]
fn interactions_the_panel_cannot_say_are_shown_and_kept() {
    let mut h = harness();
    let (A, _, F) = layout(&mut h);
    select(&mut h, &[A]);
    click(&mut h, "Add interaction");
    // Give the stored reaction a second action (Figma can; the panel cannot edit that).
    let tab = &mut h.state_mut().tabs[0];
    let mut r = tab
        .doc
        .node(A)
        .unwrap()
        .data
        .get("prototypeInteractions")
        .unwrap()
        .items()[0]
        .clone()
        .into_owned();
    let act = r.get("actions").unwrap().items()[0].clone().into_owned();
    r.set("actions", figstudio_fig::Val::Arr(vec![act.clone(), act]));
    let mut txn = figstudio_fig::edit::Txn::new("two actions");
    txn.set_field(
        A,
        "prototypeInteractions",
        figstudio_fig::Val::Arr(vec![r.clone()]),
    );
    let doc = std::sync::Arc::make_mut(&mut tab.doc);
    tab.history.execute(doc, txn).unwrap();
    h.run_steps(3);
    assert!(h.query_by_label("Add interaction").is_none(), "no editing");
    assert!(h.query_by_label_contains("cannot edit").is_some());
    let kept = h.state().tabs[0]
        .doc
        .node(A)
        .unwrap()
        .data
        .get("prototypeInteractions")
        .unwrap()
        .items()[0]
        .get("actions")
        .unwrap()
        .items()
        .len();
    assert_eq!(kept, 2);
    let _ = F;
}

#[test]
fn a_connection_to_a_frame_on_the_left_runs_between_the_facing_sides() {
    let mut h = harness();
    let (_, B, F) = layout(&mut h);
    crate::editor::apply_tab_ops(
        &mut h.state_mut().tabs[0],
        &[figstudio_fig::edit::EditOp::SetReactions {
            target: F,
            reactions: vec![figstudio_fig::edit::ReactionSpec {
                trigger: Trigger::OnClick,
                action: Action::Navigate { to: B },
                transition: None,
                duration: None,
                easing: None,
            }],
        }],
        None,
        None,
    )
    .unwrap();
    select(&mut h, &[F]);
    let tab = &h.state().tabs[0];
    let origin = tab.canvas_rect.unwrap().min;
    let (a, z) = crate::editor::proto_connections(tab, origin)[0];
    let to_screen = |x: f64| tab.view.to_screen(origin, Pos2::new(x as f32, 0.0)).x;
    let (bf, bb) = (
        figstudio_render::node_bounds(&tab.doc, F).unwrap(),
        figstudio_render::node_bounds(&tab.doc, B).unwrap(),
    );
    assert_eq!((a.x, z.x), (to_screen(bf.x0), to_screen(bb.x1)));
}

#[test]
fn a_layer_in_a_group_has_no_scroll_position_but_one_in_a_frame_does() {
    let mut h = harness();
    let (A, B, F) = layout(&mut h);
    let child = h.state().tabs[0].doc.children(F)[0];
    select(&mut h, &[child]);
    assert!(
        h.query_by_label("Position when scrolling").is_some(),
        "in a frame"
    );
    let group = crate::editor::apply_tab_ops(
        &mut h.state_mut().tabs[0],
        &[figstudio_fig::edit::EditOp::Group {
            targets: vec![A, B],
            name: None,
        }],
        None,
        None,
    )
    .unwrap()[0];
    assert!(h.state().tabs[0].doc.node(group).is_some());
    select(&mut h, &[A]);
    assert!(
        h.query_by_label("Position when scrolling").is_none(),
        "in a group"
    );
}

#[test]
fn a_group_offers_no_frame_settings() {
    let mut h = harness();
    let (A, B, F) = layout(&mut h);
    let group = crate::editor::apply_tab_ops(
        &mut h.state_mut().tabs[0],
        &[figstudio_fig::edit::EditOp::Group {
            targets: vec![A, B],
            name: None,
        }],
        None,
        None,
    )
    .unwrap()[0];
    select(&mut h, &[group]);
    for frame_only in ["Clip content", "Add auto layout", "Overflow scrolling"] {
        assert!(
            h.query_by_label(frame_only).is_none(),
            "{frame_only} on a group"
        );
    }
    // A frame has them.
    select(&mut h, &[F]);
    for frame_only in ["Clip content", "Add auto layout", "Overflow scrolling"] {
        assert!(
            h.query_by_label(frame_only).is_some(),
            "{frame_only} on a frame"
        );
    }
}
