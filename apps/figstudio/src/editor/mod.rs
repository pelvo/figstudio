//! Interactive GUI editor: canvas gizmos, shape/text creation tools, bidirectional Properties
//! Inspector, two-way Figma HTML clipboard + plain-text paste fallback, and unified Live / Wasm
//! Relay MCP mutation dispatcher.

mod canvas_gizmos;
mod clipboard_io;
mod inspector;
mod inspector_sections;
mod mcp_dispatch;

use std::sync::Arc;

use figstudio_fig::edit::{EditOp, execute_edit_ops};
use figstudio_fig::{Guid, Val};

use crate::tab::Tab;

pub use canvas_gizmos::{CanvasDrag, canvas_gizmos_and_input};
pub use clipboard_io::{copy_selection_for_figma, paste_from_clipboard_or_text};
pub use inspector::{properties_edit_controls, properties_history_bar};
#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
pub use mcp_dispatch::handle_live_mutation;
#[cfg(any(test, target_arch = "wasm32"))]
pub use mcp_dispatch::handle_wasm_mutation;

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
        if let Some(sel) = tab.selection
            && tab.doc.node(sel).is_none()
        {
            tab.selection = None;
            tab.inner = None;
        }
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
        Ok(Some(label))
    } else {
        Ok(None)
    }
}

/// Duplicates the currently selected layer on `tab` with a `(24, 24)` offset.
pub fn duplicate_selection(tab: &mut Tab) -> Result<String, String> {
    let sel = tab
        .selection
        .ok_or_else(|| "Select a layer first to duplicate".to_string())?;
    let ids = apply_tab_ops(
        tab,
        &[EditOp::DuplicateNode {
            target: sel,
            dx: 24.0,
            dy: 24.0,
        }],
        Some("Duplicate layer"),
        None,
    )?;
    if let Some(&first) = ids.first() {
        tab.select(Some(first));
    }
    Ok("Duplicated layer".into())
}

/// Deletes the currently selected layer on `tab`.
pub fn delete_selection(tab: &mut Tab) -> Result<String, String> {
    let sel = tab
        .selection
        .ok_or_else(|| "Select a layer first to delete".to_string())?;
    if sel == tab.page {
        return Err("Cannot delete the current page root".into());
    }
    let name = tab
        .doc
        .node(sel)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    apply_tab_ops(
        tab,
        &[EditOp::DeleteNode { target: sel }],
        Some("Delete layer"),
        None,
    )?;
    tab.select(None);
    Ok(format!("Deleted “{name}”"))
}

pub(super) fn local_xywh(data: &Val) -> (f64, f64, f64, f64) {
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
