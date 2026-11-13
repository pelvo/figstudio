//! MCP tools for non-destructive `.fig` editing, Undo/Redo, image import, two-way Figma
//! clipboard exchange, and streaming `save_copy`.

use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use figstudio_fig::edit::{EditOp, execute_edit_ops, execute_import_image, execute_paste_html};
use figstudio_mcp_core::editing;
use figstudio_mcp_core::node_id::{NodeId, parse_node_id};
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::FigstudioServer;
use crate::error::McpError;
use crate::files::Files;
use crate::live::LiveMutation;
use crate::tools::{to_call, tool_error};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditableInput {
    /// Node ID (`1:2`), instance path (`I1:2;3:4`), or link; defaults to selection in live mode.
    node_id: Option<String>,
    /// File key when multiple files are registered or open.
    file_key: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApplyEditsInput {
    /// Ordered list of semantic `EditOp` objects (call `describe_edit_ops` for full specification).
    ops: Vec<Value>,
    /// File key when multiple files are registered or open.
    file_key: Option<String>,
    /// Human-readable Undo label for this batch.
    label: Option<String>,
    /// Validate against Kiwi schema and compute Auto-Layout without mutating the document.
    dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileOnlyInput {
    /// File key when multiple files are registered or open.
    file_key: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImportImageInput {
    /// Base64-encoded PNG, JPEG, WebP, or GIF image bytes.
    data_base64: String,
    /// File key when multiple files are registered or open.
    file_key: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyForFigmaInput {
    /// Node IDs to export; defaults to selection in live mode.
    node_ids: Option<Vec<String>>,
    /// File key when multiple files are registered or open.
    file_key: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PasteFromFigmaInput {
    /// Figma clipboard HTML (`<!--(figmeta)...-->` + `<!--(figma)...-->`).
    html: String,
    /// Target `CANVAS`, `FRAME`, `SECTION`, or `GROUP` GUID; defaults to first page.
    parent_id: Option<String>,
    /// Horizontal offset applied to pasted root nodes.
    dx: Option<f64>,
    /// Vertical offset applied to pasted root nodes.
    dy: Option<f64>,
    /// File key when multiple files are registered or open.
    file_key: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveCopyInput {
    /// Destination `.fig` file path to write atomically (never overwrites original unless asked).
    out_path: String,
    /// File key when multiple files are registered or open.
    file_key: Option<String>,
}

#[tool_router(router = editing_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "List every semantic EditOp variant accepted by apply_edits and its JSON shape."
    )]
    fn describe_edit_ops(&self) -> CallToolResult {
        to_call(editing::describe_ops())
    }

    #[tool(
        description = "Inspect all editable layout, paint, text, component, style and variable properties of a node (defaults to selection)."
    )]
    async fn get_editable(&self, Parameters(input): Parameters<EditableInput>) -> CallToolResult {
        let key = match self.file_for(input.file_key.as_deref(), input.node_id.as_deref()) {
            Ok(k) => k,
            Err(e) => return tool_error(e),
        };
        let (key, target) = match self.target(key, input.node_id) {
            Ok(pair) => pair,
            Err(e) => return tool_error(e),
        };
        let doc = match self.files().open(key.as_deref()).await {
            Ok(doc) => doc,
            Err(e) => return tool_error(e),
        };
        to_call(editing::editable(&doc, &target))
    }

    #[tool(
        description = "Execute an atomic, Kiwi-schema-validated batch of EditOp mutations with automatic Auto-Layout solving and Undo support."
    )]
    async fn apply_edits(&self, Parameters(input): Parameters<ApplyEditsInput>) -> CallToolResult {
        let mut parsed_ops = Vec::with_capacity(input.ops.len());
        for (i, raw) in input.ops.into_iter().enumerate() {
            match serde_json::from_value::<EditOp>(raw) {
                Ok(op) => parsed_ops.push(op),
                Err(e) => {
                    return tool_error(McpError::app(format!("ops[{i}] invalid EditOp: {e}")));
                }
            }
        }
        let dry_run = input.dry_run.unwrap_or(false);
        let label = input.label;

        match self.files() {
            Files::Registry(reg) => {
                let res = reg
                    .mutate_doc(input.file_key.as_deref(), move |doc, history, _| {
                        let (report, _) = execute_edit_ops(
                            doc,
                            history,
                            &parsed_ops,
                            label.as_deref(),
                            None,
                            dry_run,
                        )
                        .map_err(McpError::open)?;
                        Ok(serde_json::to_value(report).unwrap_or_else(|_| json!({})))
                    })
                    .await;
                match res {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
            Files::Live(live) => {
                let picked = match live.pick(input.file_key.as_deref()) {
                    Ok(p) => p,
                    Err(e) => return tool_error(e),
                };
                match live
                    .mutate(
                        picked.tab().id,
                        LiveMutation::ApplyEdits {
                            ops: parsed_ops,
                            label,
                            dry_run,
                        },
                    )
                    .await
                {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
        }
    }

    #[tool(description = "Undo the most recent edit batch on the target document.")]
    async fn undo(&self, Parameters(input): Parameters<FileOnlyInput>) -> CallToolResult {
        match self.files() {
            Files::Registry(reg) => {
                let res = reg
                    .mutate_doc(input.file_key.as_deref(), |doc, history, _| {
                        let undone = history.undo(doc).map_err(McpError::open)?;
                        Ok(json!({
                            "undone": undone.as_ref().map(|(l, _)| l.as_str()),
                            "revision": doc.revision,
                            "undoDepth": history.undo_len(),
                            "redoDepth": history.redo_len(),
                        }))
                    })
                    .await;
                match res {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
            Files::Live(live) => {
                let picked = match live.pick(input.file_key.as_deref()) {
                    Ok(p) => p,
                    Err(e) => return tool_error(e),
                };
                match live.mutate(picked.tab().id, LiveMutation::Undo).await {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
        }
    }

    #[tool(description = "Redo the most recently undone edit batch on the target document.")]
    async fn redo(&self, Parameters(input): Parameters<FileOnlyInput>) -> CallToolResult {
        match self.files() {
            Files::Registry(reg) => {
                let res = reg
                    .mutate_doc(input.file_key.as_deref(), |doc, history, _| {
                        let redone = history.redo(doc).map_err(McpError::open)?;
                        Ok(json!({
                            "redone": redone.as_ref().map(|(l, _)| l.as_str()),
                            "revision": doc.revision,
                            "undoDepth": history.undo_len(),
                            "redoDepth": history.redo_len(),
                        }))
                    })
                    .await;
                match res {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
            Files::Live(live) => {
                let picked = match live.pick(input.file_key.as_deref()) {
                    Ok(p) => p,
                    Err(e) => return tool_error(e),
                };
                match live.mutate(picked.tab().id, LiveMutation::Redo).await {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
        }
    }

    #[tool(description = "Return the document edit revision and current Undo/Redo stack labels.")]
    async fn get_history(&self, Parameters(input): Parameters<FileOnlyInput>) -> CallToolResult {
        match self.files() {
            Files::Registry(reg) => {
                let res = reg
                    .mutate_doc(input.file_key.as_deref(), |doc, history, _| {
                        Ok(json!({
                            "revision": doc.revision,
                            "undo": history.undo_labels(),
                            "redo": history.redo_labels(),
                        }))
                    })
                    .await;
                match res {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
            Files::Live(live) => {
                let picked = match live.pick(input.file_key.as_deref()) {
                    Ok(p) => p,
                    Err(e) => return tool_error(e),
                };
                match live.mutate(picked.tab().id, LiveMutation::GetHistory).await {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
        }
    }

    #[tool(
        description = "Import a PNG, JPEG, WebP or GIF image (base64) into the document image store and return its 40-char SHA-1 hash."
    )]
    async fn import_image(
        &self,
        Parameters(input): Parameters<ImportImageInput>,
    ) -> CallToolResult {
        let bytes = match B64.decode(input.data_base64.trim()) {
            Ok(b) => b,
            Err(e) => return tool_error(McpError::app(format!("invalid base64 image data: {e}"))),
        };
        match self.files() {
            Files::Registry(reg) => {
                let res = reg
                    .mutate_doc(input.file_key.as_deref(), move |doc, history, _| {
                        let (hash, _) =
                            execute_import_image(doc, history, bytes).map_err(McpError::open)?;
                        Ok(json!({ "sha1Hex": hash, "revision": doc.revision }))
                    })
                    .await;
                match res {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
            Files::Live(live) => {
                let picked = match live.pick(input.file_key.as_deref()) {
                    Ok(p) => p,
                    Err(e) => return tool_error(e),
                };
                match live
                    .mutate(picked.tab().id, LiveMutation::ImportImage { bytes })
                    .await
                {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
        }
    }

    #[tool(
        description = "Export one or more nodes (defaults to selection) with transitive local SYMBOL masters as Figma clipboard HTML."
    )]
    async fn copy_for_figma(
        &self,
        Parameters(input): Parameters<CopyForFigmaInput>,
    ) -> CallToolResult {
        let first_hint = input
            .node_ids
            .as_ref()
            .and_then(|v| v.first())
            .map(String::as_str);
        let key = match self.file_for(input.file_key.as_deref(), first_hint) {
            Ok(k) => k,
            Err(e) => return tool_error(e),
        };
        let ids = match input.node_ids {
            Some(v) if !v.is_empty() => v,
            _ => match self.target(key.clone(), None) {
                Ok((_, sel)) => vec![sel],
                Err(e) => return tool_error(e),
            },
        };
        let (doc, fkey, _) = match self.files().open_picked(key.as_deref()).await {
            Ok(t) => t,
            Err(e) => return tool_error(e),
        };
        to_call(editing::copy_for_figma(&doc, &ids, &fkey))
    }

    #[tool(
        description = "Paste Figma clipboard HTML into a page or container frame, remapping GUIDs and deduplicating local components."
    )]
    async fn paste_from_figma(
        &self,
        Parameters(input): Parameters<PasteFromFigmaInput>,
    ) -> CallToolResult {
        let parent_guid = match input.parent_id.as_deref() {
            Some(raw) => match parse_node_id(raw) {
                Ok(NodeId::Node(g)) => Some(g),
                _ => return tool_error(McpError::app("parentId must be a valid node GUID")),
            },
            None => None,
        };
        let offset = (input.dx.unwrap_or(0.0), input.dy.unwrap_or(0.0));
        let html = input.html;

        match self.files() {
            Files::Registry(reg) => {
                let res = reg
                    .mutate_doc(input.file_key.as_deref(), move |doc, history, _| {
                        let target_parent = parent_guid
                            .or_else(|| doc.pages.first().copied())
                            .ok_or_else(|| McpError::app("document has no pages"))?;
                        let (roots, _) =
                            execute_paste_html(doc, history, &html, target_parent, offset)
                                .map_err(McpError::open)?;
                        Ok(json!({
                            "pastedRootIds": roots.into_iter().map(|g| g.to_string()).collect::<Vec<_>>(),
                            "revision": doc.revision,
                        }))
                    })
                    .await;
                match res {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
            Files::Live(live) => {
                let picked = match live.pick(input.file_key.as_deref()) {
                    Ok(p) => p,
                    Err(e) => return tool_error(e),
                };
                match live
                    .mutate(
                        picked.tab().id,
                        LiveMutation::PasteHtml {
                            html,
                            parent: parent_guid,
                            offset,
                        },
                    )
                    .await
                {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
        }
    }

    #[tool(
        description = "Atomically save the edited document to outPath (streaming unread images when opened lazily)."
    )]
    async fn save_copy(&self, Parameters(input): Parameters<SaveCopyInput>) -> CallToolResult {
        let out_path = PathBuf::from(&input.out_path);
        match self.files() {
            Files::Registry(reg) => {
                let out_clone = out_path.clone();
                let res = reg
                    .mutate_doc(input.file_key.as_deref(), move |doc, _, src_path| {
                        doc.save_with_lazy_source(Some(src_path), &out_clone)
                            .map_err(McpError::open)?;
                        Ok(json!({
                            "savedPath": out_clone,
                            "revision": doc.revision,
                            "nodes": doc.nodes.len(),
                        }))
                    })
                    .await;
                match res {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
            Files::Live(live) => {
                let picked = match live.pick(input.file_key.as_deref()) {
                    Ok(p) => p,
                    Err(e) => return tool_error(e),
                };
                match live
                    .mutate(picked.tab().id, LiveMutation::SaveCopy { out_path })
                    .await
                {
                    Ok(val) => CallToolResult::structured(val),
                    Err(e) => tool_error(e),
                }
            }
        }
    }
}
