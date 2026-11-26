//! Interactive GUI editor: canvas gizmos, shape/text creation tools, bidirectional Properties
//! Inspector, two-way Figma HTML clipboard + plain-text paste fallback, and unified Live / Wasm
//! Relay MCP mutation dispatcher.

mod canvas_gizmos;
mod canvas_select;
mod clipboard_io;
pub(crate) mod commands;
mod design;
mod design_component;
mod design_more;
mod design_proto;
mod inspector;
mod mcp_dispatch;
mod vector_edit;
mod widgets;

use std::sync::Arc;

use figstudio_fig::edit::{EditOp, execute_edit_ops};
use figstudio_fig::{Guid, Val};

use crate::tab::Tab;

pub use canvas_gizmos::{CanvasDrag, canvas_gizmos_and_input};
pub(crate) use canvas_select::nudge;
pub use clipboard_io::{copy_selection_for_figma, paste_from_clipboard_or_text};
pub use design::design_sections;
pub(crate) use design_component::local_components;
#[cfg(test)]
pub(crate) use design_proto::connections as proto_connections;
pub(crate) use design_proto::noodles as proto_noodles;
pub use inspector::{properties_edit_controls, properties_history_bar};
#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
pub use mcp_dispatch::handle_live_mutation;
#[cfg(any(test, target_arch = "wasm32"))]
pub use mcp_dispatch::handle_wasm_mutation;
#[cfg(test)]
pub(crate) use vector_edit::finish_pen as vector_edit_finish_pen;
pub(crate) use vector_edit::keys as vector_keys;
pub(crate) use vector_edit::{PenPath, VectorEditing};
pub(crate) use widgets::{button, num_field, rgba_of, text_field};

/// Applies `ops` onto `tab.doc` via `tab.history`, invalidating derived caches and syncing
/// `dirty_unsaved` against `tab.saved_revision`.
pub fn apply_tab_ops(
    tab: &mut Tab,
    ops: &[EditOp],
    label: Option<&str>,
    coalesce_key: Option<&str>,
) -> Result<Vec<Guid>, String> {
    let doc_mut = Arc::make_mut(&mut tab.doc);
    let (report, _) = execute_edit_ops(doc_mut, &mut tab.history, ops, label, coalesce_key, false)
        .map_err(|e| e.to_string())?;
    if report.applied {
        tab.sync_unsaved();
        tab.motion_preview = None;
        tab.prune_selection();
    }
    Ok(report.created_ids)
}

/// Undoes the last edit on `tab` and updates `dirty_unsaved` against `tab.saved_revision`.
pub fn undo_tab(tab: &mut Tab) -> Result<Option<String>, String> {
    let doc_mut = Arc::make_mut(&mut tab.doc);
    let res = tab.history.undo(doc_mut).map_err(|e| e.to_string())?;
    if let Some((label, _)) = res {
        tab.sync_unsaved();
        tab.motion_preview = None;
        tab.prune_selection();
        Ok(Some(label))
    } else {
        Ok(None)
    }
}

/// Redoes the last undone edit on `tab` and updates `dirty_unsaved` against `tab.saved_revision`.
pub fn redo_tab(tab: &mut Tab) -> Result<Option<String>, String> {
    let doc_mut = Arc::make_mut(&mut tab.doc);
    let res = tab.history.redo(doc_mut).map_err(|e| e.to_string())?;
    if let Some((label, _)) = res {
        tab.sync_unsaved();
        tab.motion_preview = None;
        tab.prune_selection();
        Ok(Some(label))
    } else {
        Ok(None)
    }
}

/// The selected layers, or an error naming what needs a selection.
fn selection_for(tab: &Tab, what: &str) -> Result<Vec<Guid>, String> {
    let ids: Vec<Guid> = tab
        .selected()
        .into_iter()
        .filter(|&id| id != tab.page)
        .collect();
    if ids.is_empty() {
        return Err(format!("Select a layer first to {what}"));
    }
    Ok(ids)
}

/// `"“name”"` for one layer, `"n layers"` for several.
fn describe(tab: &Tab, ids: &[Guid]) -> String {
    match ids {
        [one] => format!(
            "“{}”",
            tab.doc.node(*one).map(|n| n.name.as_str()).unwrap_or("")
        ),
        many => format!("{} layers", many.len()),
    }
}

/// Duplicates the selected layers on `tab` with a `(24, 24)` offset and selects the copies.
pub fn duplicate_selection(tab: &mut Tab) -> Result<String, String> {
    let ids = selection_for(tab, "duplicate")?;
    let ops: Vec<EditOp> = ids
        .iter()
        .map(|&target| EditOp::DuplicateNode {
            target,
            dx: 24.0,
            dy: 24.0,
        })
        .collect();
    let label = if ids.len() == 1 {
        "Duplicate layer"
    } else {
        "Duplicate layers"
    };
    let copies = apply_tab_ops(tab, &ops, Some(label), None)?;
    tab.select_all_of(&copies);
    Ok(format!("Duplicated {}", describe(tab, &ids)))
}

/// Deletes the selected layers on `tab`.
pub fn delete_selection(tab: &mut Tab) -> Result<String, String> {
    let ids = selection_for(tab, "delete")?;
    let what = describe(tab, &ids);
    // A layer inside another selected one goes with it; deleting it again would fail.
    let ops: Vec<EditOp> = ids
        .iter()
        .filter(|&&id| !has_selected_ancestor(tab, id, &ids))
        .map(|&target| EditOp::DeleteNode { target })
        .collect();
    let label = if ids.len() == 1 {
        "Delete layer"
    } else {
        "Delete layers"
    };
    apply_tab_ops(tab, &ops, Some(label), None)?;
    tab.select(None);
    Ok(format!("Deleted {what}"))
}

fn has_selected_ancestor(tab: &Tab, id: Guid, ids: &[Guid]) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut cur = tab.doc.node(id).and_then(|n| n.parent);
    while let Some(p) = cur {
        if !seen.insert(p) {
            return false;
        }
        if ids.contains(&p) {
            return true;
        }
        cur = tab.doc.node(p).and_then(|n| n.parent);
    }
    false
}

pub(crate) fn local_xywh(data: &Val) -> (f64, f64, f64, f64) {
    let x = data
        .path(&["transform", "m02"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    let y = data
        .path(&["transform", "m12"])
        .and_then(Val::as_f32)
        .map_or(0.0, f64::from);
    let (w, h) = figstudio_render::geom::size(data);
    (x, y, w, h)
}
