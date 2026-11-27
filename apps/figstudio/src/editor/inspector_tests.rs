//! Inspector edits: a new edit session starts a new undo step.

use std::sync::Arc;
use std::time::Duration;

use egui_kittest::Harness;
use figstudio_fig::{Document, Guid};

use crate::editor::apply_tab_ops;

use super::*;

fn rename(tab: &mut Tab, name: &str) {
    let sel = tab.selection.unwrap();
    apply_tab_ops(
        tab,
        &[EditOp::Rename {
            target: sel,
            name: name.into(),
        }],
        Some("Rename layer"),
        Some(&format!("gui:rename:{sel}")),
    )
    .unwrap();
}

#[test]
fn a_changed_edit_session_seals_the_coalesced_undo_step() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(Document::open(&path).unwrap());
    let mut tab = Tab::new(1, path, doc, None, Duration::ZERO);
    tab.select(Some(Guid(1, 25)));
    rename(&mut tab, "a");
    rename(&mut tab, "ab");
    assert_eq!(tab.history.undo_len(), 1, "typing in one field coalesces");

    // The field that was focused lost focus: the next edit is a separate step.
    tab.edit_session = Some(egui::Id::new("a field"));
    let mut h = Harness::new_ui_state(properties_edit_controls, tab);
    h.run();
    rename(h.state_mut(), "abc");
    assert_eq!(h.state().history.undo_len(), 2);
}

#[test]
fn a_partial_fill_hex_is_kept_while_its_field_has_focus() {
    use egui_kittest::kittest::Queryable;
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(Document::open(&path).unwrap());
    let target = doc
        .nodes
        .iter()
        .find(|n| n.kind == "RECTANGLE" && first_solid_srgba(&n.data).is_some())
        .or_else(|| {
            doc.nodes
                .iter()
                .find(|n| first_solid_srgba(&n.data).is_some())
        })
        .map(|n| n.id)
        .expect("the fixture has a layer with a solid fill");
    let before = first_solid_srgba(&doc.node(target).unwrap().data).unwrap();
    let mut tab = Tab::new(1, path, doc, None, Duration::ZERO);
    tab.select(Some(target));
    let mut h = Harness::new_ui_state(properties_edit_controls, tab);
    h.run();
    h.get_by_label("Edit Layer").click();
    h.run_steps(3);
    let hex = format!("#{:02X}{:02X}{:02X}", before[0], before[1], before[2]);
    let value = |h: &Harness<'_, Tab>| h.get_by_label("Fill colour hex").value();
    assert_eq!(value(&h).as_deref(), Some(hex.as_str()));

    // Delete four digits: the field keeps the partial text and the fill does not change.
    h.get_by_label("Fill colour hex").click();
    h.run();
    h.key_press(egui::Key::End);
    for _ in 0..4 {
        h.key_press(egui::Key::Backspace);
    }
    h.run_steps(3);
    assert_eq!(value(&h).as_deref(), Some(&hex[..3]));
    let fill = |h: &Harness<'_, Tab>| first_solid_srgba(&h.state().doc.node(target).unwrap().data);
    assert_eq!(fill(&h), Some(before));

    // Completing it commits the new colour.
    h.get_by_label("Fill colour hex").type_text("1234");
    h.run_steps(3);
    let after = fill(&h).unwrap();
    assert_eq!((after[0], after[1], after[2]), (before[0], 0x12, 0x34));
}

#[test]
fn autolayout_gap_and_direction_preserve_asymmetric_padding_and_fixed_sizing() {
    use egui_kittest::kittest::Queryable;
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(Document::open(&path).unwrap());
    let mut tab = Tab::new(1, path, doc, None, Duration::ZERO);
    let page = tab.page;
    let frame = apply_tab_ops(
        &mut tab,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: None,
            x: 10.0,
            y: 10.0,
            width: 240.0,
            height: 120.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
    )
    .unwrap()[0];
    apply_tab_ops(
        &mut tab,
        &[EditOp::SetAutoLayout {
            target: frame,
            mode: "HORIZONTAL".into(),
            spacing: Some(8.0),
            counter_spacing: None,
            padding: Some([4.0, 20.0, 16.0, 8.0]),
            primary_sizing: Some("FIXED".into()),
            counter_sizing: Some("FIXED".into()),
            primary_align: None,
            counter_align: None,
            wrap: None,
        }],
        None,
        None,
    )
    .unwrap();
    tab.select(Some(frame));

    let mut h = Harness::new_ui_state(properties_edit_controls, tab);
    h.run();
    h.get_by_label("Edit Layer").click();
    h.run_steps(3);

    // Edit Gap only: asymmetric right/bottom padding and FIXED sizing must survive.
    let gap_rect = h.get_by_label("Auto-layout gap").rect();
    let start = gap_rect.center();
    h.event(egui::Event::PointerMoved(start));
    h.step();
    h.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.event(egui::Event::PointerMoved(start + egui::vec2(10.0, 0.0)));
    h.step();
    h.event(egui::Event::PointerButton {
        pos: start + egui::vec2(10.0, 0.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);

    let data = &h.state().doc.node(frame).unwrap().data;
    assert_ne!(
        data.num("stackSpacing"),
        Some(8.0),
        "gap should have changed"
    );
    assert_eq!(data.num("stackVerticalPadding"), Some(4.0));
    assert_eq!(data.num("stackPaddingRight"), Some(20.0));
    assert_eq!(data.num("stackPaddingBottom"), Some(16.0));
    assert_eq!(data.num("stackHorizontalPadding"), Some(8.0));
    assert_eq!(data.text("stackPrimarySizing"), Some("FIXED"));
    assert_eq!(data.text("stackCounterSizing"), Some("FIXED"));

    // Switching Horiz -> Vert must also preserve FIXED sizing and asymmetric padding.
    h.get_by_label("Vert").click();
    h.run_steps(2);
    let data = &h.state().doc.node(frame).unwrap().data;
    assert_eq!(data.text("stackMode"), Some("VERTICAL"));
    assert_eq!(data.num("stackVerticalPadding"), Some(4.0));
    assert_eq!(data.num("stackPaddingRight"), Some(20.0));
    assert_eq!(data.num("stackPaddingBottom"), Some(16.0));
    assert_eq!(data.num("stackHorizontalPadding"), Some(8.0));
    assert_eq!(data.text("stackPrimarySizing"), Some("FIXED"));
    assert_eq!(data.text("stackCounterSizing"), Some("FIXED"));
}

#[test]
fn fill_hex_draft_is_isolated_across_tabs_and_document_revisions() {
    use egui_kittest::kittest::Queryable;
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(Document::open(&path).unwrap());
    let target = Guid(1, 34);

    let mut tab1 = Tab::new(1, path.clone(), doc.clone(), None, Duration::ZERO);
    apply_tab_ops(
        &mut tab1,
        &[EditOp::SetSolidFill {
            target,
            hex: Some("#112233".into()),
        }],
        None,
        None,
    )
    .unwrap();
    tab1.select(Some(target));

    let mut tab2 = Tab::new(2, path, doc, None, Duration::ZERO);
    apply_tab_ops(
        &mut tab2,
        &[EditOp::SetSolidFill {
            target,
            hex: Some("#AA5500".into()),
        }],
        None,
        None,
    )
    .unwrap();
    tab2.select(Some(target));

    struct TwoTabs {
        active: usize,
        tabs: [Tab; 2],
    }

    let mut h = Harness::new_ui_state(
        |ui, state: &mut TwoTabs| {
            properties_edit_controls(ui, &mut state.tabs[state.active]);
        },
        TwoTabs {
            active: 0,
            tabs: [tab1, tab2],
        },
    );
    h.run();
    h.get_by_label("Edit Layer").click();
    h.run_steps(3);
    assert_eq!(
        h.get_by_label("Fill colour hex").value().as_deref(),
        Some("#112233")
    );

    // Leave a partial draft in tab 1's Fill hex field.
    h.get_by_label("Fill colour hex").click();
    h.run();
    h.key_press(egui::Key::End);
    for _ in 0..4 {
        h.key_press(egui::Key::Backspace);
    }
    h.run_steps(3);
    assert_eq!(
        h.get_by_label("Fill colour hex").value().as_deref(),
        Some("#11")
    );

    // Switch to tab 2 (same node Guid): must show tab 2's fill, not tab 1's draft.
    h.state_mut().active = 1;
    h.run_steps(2);
    assert_eq!(
        h.get_by_label("Fill colour hex").value().as_deref(),
        Some("#AA5500"),
        "tab 2 must not inherit tab 1's partial fill hex draft"
    );

    // Return to tab 1, focus a partial draft, then apply an external fill edit (new revision).
    h.state_mut().active = 0;
    h.run_steps(2);
    h.get_by_label("Fill colour hex").click();
    h.run();
    h.key_press(egui::Key::End);
    for _ in 0..4 {
        h.key_press(egui::Key::Backspace);
    }
    h.run_steps(3);
    assert_eq!(
        h.get_by_label("Fill colour hex").value().as_deref(),
        Some("#11")
    );

    apply_tab_ops(
        &mut h.state_mut().tabs[0],
        &[EditOp::SetSolidFill {
            target,
            hex: Some("#44CC88".into()),
        }],
        None,
        None,
    )
    .unwrap();
    h.run_steps(2);
    assert_eq!(
        h.get_by_label("Fill colour hex").value().as_deref(),
        Some("#44CC88"),
        "a document revision change must supersede a stale fill hex draft"
    );
}

#[test]
fn switching_inner_instance_text_starts_a_new_undo_step() {
    use egui_kittest::kittest::Queryable;
    use figstudio_fig::resolve::InstancePath;

    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(Document::open(&path).unwrap());
    let mut tab = Tab::new(1, path, doc, None, Duration::ZERO);
    // Duplicate the TEXT node (1:42) inside the SYMBOL so instance 1:38 has two inner texts.
    let second_text = apply_tab_ops(
        &mut tab,
        &[EditOp::DuplicateNode {
            target: Guid(1, 42),
            dx: 0.0,
            dy: 20.0,
        }],
        None,
        None,
    )
    .unwrap()[0];
    tab.history = figstudio_fig::edit::History::new(20);

    let instance = Guid(1, 38);
    let path_a = InstancePath::new(vec![instance, Guid(1, 42)]).unwrap();
    let path_b = InstancePath::new(vec![instance, second_text]).unwrap();
    tab.select(Some(instance));
    tab.set_inner(Some(path_a));

    let mut h = Harness::new_ui_state(properties_edit_controls, tab);
    h.run();
    h.get_by_label("Instance text override").click();
    h.run();
    h.get_by_label("Instance text override").type_text("1");
    h.run_steps(2);
    assert_eq!(h.state().history.undo_len(), 1);

    // Switch inner target inside the SAME instance and type without changing outer selection.
    h.state_mut().set_inner(Some(path_b));
    h.run_steps(2);
    h.get_by_label("Instance text override").click();
    h.run();
    h.get_by_label("Instance text override").type_text("2");
    h.run_steps(2);
    assert_eq!(
        h.state().history.undo_len(),
        2,
        "editing a different inner text node in the same instance must not coalesce with the previous inner text"
    );
}

fn drag_widget(h: &mut Harness<'_, Tab>, label: &str, dx: f32) {
    use egui_kittest::kittest::Queryable;
    let start = h.get_by_label(label).rect().center();
    h.event(egui::Event::PointerMoved(start));
    h.step();
    h.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    h.step();
    h.event(egui::Event::PointerMoved(start + egui::vec2(dx, 0.0)));
    h.step();
    h.event(egui::Event::PointerButton {
        pos: start + egui::vec2(dx, 0.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(2);
}

#[test]
fn dragging_numeric_fields_while_text_field_has_focus_keeps_separate_undo_steps() {
    use egui_kittest::kittest::Queryable;
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(Document::open(&path).unwrap());
    let mut tab = Tab::new(1, path, doc, None, Duration::ZERO);
    let page = tab.page;
    let frame = apply_tab_ops(
        &mut tab,
        &[EditOp::CreateShape {
            id: None,
            kind: "FRAME".into(),
            parent: page,
            name: None,
            x: 100.0,
            y: 120.0,
            width: 240.0,
            height: 120.0,
            corner_radius: 0.0,
            fill_hex: None,
            count: None,
            star_inner_scale: None,
        }],
        None,
        None,
    )
    .unwrap()[0];
    apply_tab_ops(
        &mut tab,
        &[EditOp::SetAutoLayout {
            target: frame,
            mode: "HORIZONTAL".into(),
            spacing: Some(8.0),
            counter_spacing: None,
            padding: Some([12.0, 12.0, 12.0, 12.0]),
            primary_sizing: None,
            counter_sizing: None,
            primary_align: None,
            counter_align: None,
            wrap: None,
        }],
        None,
        None,
    )
    .unwrap();
    tab.history = figstudio_fig::edit::History::new(20);
    tab.select(Some(frame));

    let mut h = Harness::new_ui_state(properties_edit_controls, tab);
    h.run();
    h.get_by_label("Edit Layer").click();
    h.run_steps(3);

    // Focus Layer name so ctx.memory().focused() stays Some(layer_name) across DragValue drags.
    h.get_by_label("Layer name").click();
    h.run_steps(2);

    drag_widget(&mut h, "Position X", 12.0);
    assert_eq!(h.state().history.undo_len(), 1);
    drag_widget(&mut h, "Position Y", 12.0);
    assert_eq!(
        h.state().history.undo_len(),
        2,
        "dragging Position Y after Position X while Layer name has focus must be a separate undo step"
    );
    drag_widget(&mut h, "Position X", 10.0);
    assert_eq!(
        h.state().history.undo_len(),
        3,
        "a second drag of Position X while Layer name has focus must be a separate undo step"
    );
    drag_widget(&mut h, "Auto-layout gap", 10.0);
    assert_eq!(h.state().history.undo_len(), 4);
    let gap_after = h.state().doc.node(frame).unwrap().data.num("stackSpacing");
    drag_widget(&mut h, "Auto-layout horizontal padding", 10.0);
    assert_eq!(
        h.state().history.undo_len(),
        5,
        "dragging PadX after Gap while Layer name has focus must be a separate undo step"
    );

    // Undoing once restores only PadX and preserves the Gap edit.
    undo_tab(h.state_mut()).unwrap();
    let data = &h.state().doc.node(frame).unwrap().data;
    assert_eq!(data.num("stackHorizontalPadding"), Some(12.0));
    assert_eq!(data.num("stackSpacing"), gap_after);
}

#[test]
fn switching_tabs_isolates_in_progress_drag_value_and_name_edits() {
    use egui_kittest::kittest::Queryable;
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/structure.fig");
    let doc = Arc::new(Document::open(&path).unwrap());
    let target = Guid(1, 34);

    let mut tab1 = Tab::new(1, path.clone(), doc.clone(), None, Duration::ZERO);
    apply_tab_ops(
        &mut tab1,
        &[EditOp::Move {
            target,
            x: 100.0,
            y: 50.0,
            relative: false,
        }],
        None,
        None,
    )
    .unwrap();
    tab1.select(Some(target));

    let mut tab2 = Tab::new(2, path, doc, None, Duration::ZERO);
    apply_tab_ops(
        &mut tab2,
        &[EditOp::Move {
            target,
            x: 250.0,
            y: 50.0,
            relative: false,
        }],
        None,
        None,
    )
    .unwrap();
    tab2.select(Some(target));

    struct TwoTabs {
        active: usize,
        tabs: [Tab; 2],
    }

    let mut h = Harness::new_ui_state(
        |ui, state: &mut TwoTabs| {
            properties_edit_controls(ui, &mut state.tabs[state.active]);
        },
        TwoTabs {
            active: 0,
            tabs: [tab1, tab2],
        },
    );
    h.run();
    h.get_by_label("Edit Layer").click();
    h.run_steps(3);

    // Click Position X on Tab 1 and type an uncommitted draft ("-") without pressing Enter.
    h.get_by_label("Position X").click();
    h.run_steps(2);
    h.event(egui::Event::Text("-".into()));
    h.run_steps(2);
    assert!(
        h.get_by_label("Position X").is_focused(),
        "Tab 1's Position X should have keyboard focus while editing"
    );

    // Switch to Tab 2 (as Ctrl+Tab does, without clicking away from the inspector).
    h.state_mut().active = 1;
    h.run_steps(2);
    assert!(
        !h.get_by_label("Position X").is_focused(),
        "switching tabs must not transfer Tab 1's Position X focus onto Tab 2"
    );
    // Typing after switching tabs must not edit Tab 2's X coordinate through leaked focus.
    h.event(egui::Event::Text("999".into()));
    h.run_steps(2);
    let (tab2_x, ..) = local_xywh(&h.state().tabs[1].doc.node(target).unwrap().data);
    assert!(
        (tab2_x - 250.0).abs() < 1e-3,
        "Tab 2's X coordinate must remain 250, got {tab2_x}"
    );
}
