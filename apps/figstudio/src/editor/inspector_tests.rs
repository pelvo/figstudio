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
    // The whole app, so the panel has its fonts and layout.
    let mut h = crate::ui_tests::harness();
    let target = h.state().tabs[0]
        .doc
        .nodes
        .iter()
        .find(|n| n.kind == "ROUNDED_RECTANGLE" && first_solid_srgba(&n.data).is_some())
        .map(|n| n.id)
        .expect("the fixture has a rectangle with a solid fill");
    let before = first_solid_srgba(&h.state().tabs[0].doc.node(target).unwrap().data).unwrap();
    h.state_mut().tabs[0].select(Some(target));
    h.run_steps(3);
    let hex = format!("#{:02X}{:02X}{:02X}", before[0], before[1], before[2]);
    let value =
        |h: &egui_kittest::Harness<'_, crate::App>| h.get_by_label("Fill colour hex").value();
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
    let fill = |h: &egui_kittest::Harness<'_, crate::App>| {
        first_solid_srgba(&h.state().tabs[0].doc.node(target).unwrap().data)
    };
    assert_eq!(fill(&h), Some(before));

    // Completing it commits the new colour.
    h.get_by_label("Fill colour hex").type_text("1234");
    h.run_steps(3);
    let after = fill(&h).unwrap();
    assert_eq!((after[0], after[1], after[2]), (before[0], 0x12, 0x34));
}

/// The first visible solid fill of `data` as sRGB bytes.
fn first_solid_srgba(data: &Val) -> Option<[u8; 4]> {
    let p = data.get("fillPaints")?.items().first()?;
    if p.text("type") != Some("SOLID") {
        return None;
    }
    let c = p.get("color")?;
    let op = p.num("opacity").unwrap_or(1.0);
    Some([
        (c.num("r")? * 255.0).round() as u8,
        (c.num("g")? * 255.0).round() as u8,
        (c.num("b")? * 255.0).round() as u8,
        (c.num("a").unwrap_or(1.0) * op * 255.0).round() as u8,
    ])
}
