//! Accessible bidirectional Properties Inspector (`Edit Layer` collapsible section & history bar).

use egui::Ui;
use figstudio_fig::Val;
use figstudio_fig::edit::EditOp;

use crate::tab::Tab;
use crate::theme;

use super::{apply_tab_ops, redo_tab, undo_tab};

/// Renders Undo / Redo buttons when the tab has edit history.
pub fn properties_history_bar(ui: &mut Ui, tab: &mut Tab) {
    let can_u = tab.history.can_undo();
    let can_r = tab.history.can_redo();
    if !can_u && !can_r && !tab.dirty_unsaved {
        return;
    }
    let p = theme::of(ui);
    ui.horizontal(|ui| {
        let u_btn = ui.add_enabled(can_u, egui::Button::new("↶ Undo"));
        crate::a11y::author(&u_btn, "editor.undo");
        if u_btn.clicked() {
            let _ = undo_tab(tab);
        }
        let r_btn = ui.add_enabled(can_r, egui::Button::new("↷ Redo"));
        crate::a11y::author(&r_btn, "editor.redo");
        if r_btn.clicked() {
            let _ = redo_tab(tab);
        }
        if tab.dirty_unsaved {
            ui.colored_label(p.text_dim, format!("• rev {}", tab.doc.revision));
        }
    });
}

/// The history bar, the session seal for coalesced edits, and the text override of an inner
/// instance layer. The Design sections are `editor::design`.
pub fn properties_edit_controls(ui: &mut Ui, tab: &mut Tab) {
    // One undo step per edit session: when the focused field or the dragged control changes
    // (to another one or to none), or the selection does, the last coalesced entry is sealed.
    let session = ui
        .ctx()
        .memory(|m| m.focused())
        .or_else(|| ui.ctx().dragged_id())
        .map(|id| id.with(tab.selection));
    // A canvas gesture seals its own step on release (an Alt-drag selects its copies midway).
    if tab.edit_drag.is_none() && tab.edit_session != session {
        tab.history.seal_coalesce();
        tab.edit_session = session;
    }
    properties_history_bar(ui, tab);

    // Inner-instance text override quick editor when an inner node inside an INSTANCE is selected!
    if let (Some(inst_id), Some(inner)) = (tab.selection, tab.inner.clone())
        && let Ok(resolved) = figstudio_fig::resolve::resolve_path(&tab.doc, &inner)
        && let Some(chars) = resolved
            .data
            .path(&["textData", "characters"])
            .and_then(Val::as_str)
    {
        let mut text_buf = chars.to_string();
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Instance Text Override")
                .strong()
                .size(11.5),
        );
        let resp = ui.text_edit_multiline(&mut text_buf);
        crate::a11y::name_field(&resp, "Instance text override");
        crate::a11y::author(&resp, "editor.instance-text");
        if resp.changed() {
            let ov_path = inner.keys().to_vec();
            let _ = apply_tab_ops(
                tab,
                &[EditOp::SetInstanceOverride {
                    instance: inst_id,
                    override_path: ov_path,
                    characters: Some(text_buf),
                    visible: None,
                    fill_hex: None,
                    opacity: None,
                    fields: None,
                }],
                Some("Edit instance text"),
                Some("gui:inst:text"),
            );
        }
    }
}

#[cfg(test)]
#[path = "inspector_tests.rs"]
mod tests;
