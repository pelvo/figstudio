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
