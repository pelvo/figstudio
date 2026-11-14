//! Unified tab mutation dispatcher shared by the desktop Live MCP host and the browser Wasm relay.

#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
use std::path::PathBuf;
use std::sync::Arc;

use figstudio_fig::Guid;
use figstudio_fig::edit::{EditOp, execute_edit_ops, execute_paste_html};
use serde_json::{Value, json};

use crate::tab::Tab;

use super::{redo_tab, undo_tab};

/// Shared document mutation command executed identically across Desktop Live and Browser Relay.
#[cfg_attr(
    not(any(test, target_arch = "wasm32", feature = "mcp")),
    allow(dead_code)
)]
pub(crate) enum TabMutation {
    ApplyEdits {
        ops: Vec<EditOp>,
        label: Option<String>,
        dry_run: bool,
    },
    Undo,
    Redo,
    GetHistory,
    ImportImage {
        bytes: Vec<u8>,
    },
    PasteHtml {
        html: String,
        parent: Option<Guid>,
        offset: (f64, f64),
    },
    #[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
    SaveCopy {
        out_path: PathBuf,
        overwrite: bool,
    },
}

#[cfg_attr(
    not(any(test, target_arch = "wasm32", feature = "mcp")),
    allow(dead_code)
)]
fn execute_tab_mutation(tab: &mut Tab, cmd: TabMutation) -> Result<Value, String> {
    match cmd {
        TabMutation::ApplyEdits {
            ops,
            label,
            dry_run,
        } => {
            let doc_mut = Arc::make_mut(&mut tab.doc);
            let (report, _) = execute_edit_ops(
                doc_mut,
                &mut tab.history,
                &ops,
                label.as_deref(),
                None,
                dry_run,
            )
            .map_err(|e| e.to_string())?;
            if report.applied {
                tab.sync_unsaved();
                tab.motion_preview = None;
            }
            let mut val = serde_json::to_value(report).map_err(|e| e.to_string())?;
            if let Some(obj) = val.as_object_mut() {
                obj.insert("unsaved".into(), json!(tab.dirty_unsaved));
            }
            Ok(val)
        }
        TabMutation::Undo => {
            let undone = undo_tab(tab)?;
            Ok(json!({
                "undone": undone,
                "revision": tab.doc.revision,
                "unsaved": tab.dirty_unsaved,
                "undoDepth": tab.history.undo_len(),
                "redoDepth": tab.history.redo_len(),
            }))
        }
        TabMutation::Redo => {
            let redone = redo_tab(tab)?;
            Ok(json!({
                "redone": redone,
                "revision": tab.doc.revision,
                "unsaved": tab.dirty_unsaved,
                "undoDepth": tab.history.undo_len(),
                "redoDepth": tab.history.redo_len(),
            }))
        }
        TabMutation::GetHistory => Ok(json!({
            "revision": tab.doc.revision,
            "unsaved": tab.dirty_unsaved,
            "undo": tab.history.undo_labels(),
            "redo": tab.history.redo_labels(),
        })),
        TabMutation::ImportImage { bytes } => {
            let doc_mut = Arc::make_mut(&mut tab.doc);
            let (hash, _) =
                figstudio_fig::edit::execute_import_image(doc_mut, &mut tab.history, bytes)
                    .map_err(|e| e.to_string())?;
            tab.sync_unsaved();
            Ok(json!({
                "sha1Hex": hash,
                "revision": tab.doc.revision,
                "unsaved": tab.dirty_unsaved,
            }))
        }
        TabMutation::PasteHtml {
            html,
            parent,
            offset,
        } => {
            let target_parent = parent.unwrap_or(tab.page);
            let doc_mut = Arc::make_mut(&mut tab.doc);
            let (roots, _) =
                execute_paste_html(doc_mut, &mut tab.history, &html, target_parent, offset)
                    .map_err(|e| e.to_string())?;
            tab.sync_unsaved();
            tab.motion_preview = None;
            if let Some(&first) = roots.first() {
                tab.select(Some(first));
            }
            Ok(json!({
                "pastedRootIds": roots.into_iter().map(|g| g.to_string()).collect::<Vec<_>>(),
                "revision": tab.doc.revision,
                "unsaved": tab.dirty_unsaved,
            }))
        }
        #[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
        TabMutation::SaveCopy {
            out_path,
            overwrite,
        } => {
            let lazy_src = tab.real_path.exists().then_some(tab.real_path.as_path());
            let saved_to = tab
                .doc
                .save_copy_checked(lazy_src, &out_path, overwrite, true)
                .map_err(|e| e.to_string())?;
            tab.mark_saved();
            Ok(json!({
                "savedPath": saved_to,
                "revision": tab.doc.revision,
                "unsaved": tab.dirty_unsaved,
                "nodes": tab.doc.nodes.len(),
            }))
        }
    }
}

/// Browser Wasm relay entry point: parses JSON tool arguments into [`TabMutation`] and runs
/// [`execute_tab_mutation`].
#[cfg(any(test, target_arch = "wasm32"))]
pub fn handle_wasm_mutation(
    tabs: &mut [Tab],
    tab_id: u64,
    tool: &str,
    args: &Value,
) -> Result<figstudio_mcp_core::output::Output, String> {
    use base64::Engine as _;
    use figstudio_mcp_core::node_id::{NodeId, parse_node_id};
    use figstudio_mcp_core::output::Output;

    let tab = tabs
        .iter_mut()
        .find(|t| t.id == tab_id)
        .ok_or_else(|| "tab closed".to_string())?;

    let cmd = match tool {
        "apply_edits" => {
            let raw_ops = args
                .get("ops")
                .cloned()
                .ok_or_else(|| "ops array is required".to_string())?;
            let ops: Vec<EditOp> =
                serde_json::from_value(raw_ops).map_err(|e| format!("invalid ops: {e}"))?;
            let label = args.get("label").and_then(Value::as_str).map(str::to_owned);
            let dry_run = args.get("dryRun").and_then(Value::as_bool).unwrap_or(false);
            TabMutation::ApplyEdits {
                ops,
                label,
                dry_run,
            }
        }
        "undo" => TabMutation::Undo,
        "redo" => TabMutation::Redo,
        "get_history" => TabMutation::GetHistory,
        "import_image" => {
            let b64 = args
                .get("dataBase64")
                .and_then(Value::as_str)
                .ok_or_else(|| "dataBase64 is required".to_string())?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| format!("invalid base64 image data: {e}"))?;
            TabMutation::ImportImage { bytes }
        }
        "paste_from_figma" => {
            let html = args
                .get("html")
                .and_then(Value::as_str)
                .ok_or_else(|| "html is required".to_string())?
                .to_owned();
            let parent = match args.get("parentId").and_then(Value::as_str) {
                Some(raw) => match parse_node_id(raw).map_err(|e| e.to_string())? {
                    NodeId::Node(g) => Some(g),
                    _ => return Err("parentId must be a valid node GUID".into()),
                },
                None => None,
            };
            let dx = args.get("dx").and_then(Value::as_f64).unwrap_or(24.0);
            let dy = args.get("dy").and_then(Value::as_f64).unwrap_or(24.0);
            TabMutation::PasteHtml {
                html,
                parent,
                offset: (dx, dy),
            }
        }
        other => return Err(format!("Unsupported mutation tool: {other}")),
    };

    execute_tab_mutation(tab, cmd).map(Output::structured)
}

/// Desktop hosted Live MCP entry point: maps [`figstudio_mcp::live::LiveMutation`] to
/// [`TabMutation`] and runs [`execute_tab_mutation`].
#[cfg(all(feature = "mcp", not(target_arch = "wasm32")))]
pub fn handle_live_mutation(
    tabs: &mut [Tab],
    tab_id: u64,
    mutation: figstudio_mcp::live::LiveMutation,
) -> anyhow::Result<Value> {
    use figstudio_mcp::live::LiveMutation as M;

    let tab = tabs
        .iter_mut()
        .find(|t| t.id == tab_id)
        .ok_or_else(|| anyhow::anyhow!("tab closed"))?;

    let cmd = match mutation {
        M::ApplyEdits {
            ops,
            label,
            dry_run,
        } => TabMutation::ApplyEdits {
            ops,
            label,
            dry_run,
        },
        M::Undo => TabMutation::Undo,
        M::Redo => TabMutation::Redo,
        M::GetHistory => TabMutation::GetHistory,
        M::ImportImage { bytes } => TabMutation::ImportImage { bytes },
        M::PasteHtml {
            html,
            parent,
            offset,
        } => TabMutation::PasteHtml {
            html,
            parent,
            offset,
        },
        M::SaveCopy {
            out_path,
            overwrite,
        } => TabMutation::SaveCopy {
            out_path,
            overwrite,
        },
    };

    execute_tab_mutation(tab, cmd).map_err(anyhow::Error::msg)
}
