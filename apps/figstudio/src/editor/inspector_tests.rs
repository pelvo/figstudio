//! Inspector edits: a new edit session starts a new undo step.

use std::sync::Arc;
use std::time::Duration;

use egui_kittest::Harness;
use figstudio_fig::{Document, Guid};

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
