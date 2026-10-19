//! Live-mode tools: the app's open tabs, its selection, and revealing a node in the app.
use crate::{
    FigstudioServer,
    error::McpError,
    files::Files,
    live::{LiveFiles, TabInfo},
    tools::tool_error,
};
use figstudio_fig::{Document, Guid};
use figstudio_mcp_core::{
    link::{FileHint, parse_link},
    node_id::{NodeId, parse_node_id},
};
use rmcp::{handler::server::wrapper::Parameters, model::CallToolResult, tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Inputs for `get_selection`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelectionInput {
    /// File key of an open tab (from list_open_tabs); default the active tab.
    file_key: Option<String>,
}

/// Inputs for `reveal_node`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RevealInput {
    /// Node ID, instance path (I1:2;3:4), figstudio:// or Figma URL with node-id.
    node_id: String,
    /// File key of an open tab; default the file a link names, else the active tab.
    file_key: Option<String>,
}

#[tool_router(router = live_router, vis = "pub(crate)")]
impl FigstudioServer {
    #[tool(
        description = "List the tabs open in the figstudio app: tabId, fileKey (use it in other tools), path, title, the page shown {id, name}, the selected node ID (or null) and whether the tab is active."
    )]
    fn list_open_tabs(&self) -> CallToolResult {
        let Some(live) = self.live_files() else {
            return tool_error(McpError::app("live mode only"));
        };
        let snapshot = live.snapshot();
        let tabs: Vec<Value> = live
            .keys()
            .into_iter()
            .zip(&snapshot.tabs)
            .map(|(key, tab)| {
                json!({
                    "tabId": tab.id,
                    "fileKey": key,
                    "path": tab.path,
                    "title": tab.path.file_name().map(|name| name.to_string_lossy()),
                    "page": page(tab),
                    "selection": tab.selection.map(|id| id.to_string()),
                    "active": snapshot.active == Some(tab.id),
                })
            })
            .collect();
        CallToolResult::structured(json!({ "tabs": tabs }))
    }

    #[tool(
        description = "Describe the node selected in an open tab of the figstudio app (default the active tab): {fileKey, tabId, page: {id, name}, node: {nodeId, name, kind, bounds: {x, y, width, height}}}; node is null when nothing is selected. Bounds are in page units."
    )]
    fn get_selection(&self, Parameters(input): Parameters<SelectionInput>) -> CallToolResult {
        let Some(live) = self.live_files() else {
            return tool_error(McpError::app("live mode only"));
        };
        let picked = match live.pick(input.file_key.as_deref()) {
            Ok(picked) => picked,
            Err(error) => return tool_error(error),
        };
        let tab = picked.tab();
        let node = tab.selection.and_then(|id| describe(&tab.doc, id));
        CallToolResult::structured(json!({
            "fileKey": picked.key,
            "tabId": tab.id,
            "page": page(tab),
            "node": node,
        }))
    }

    #[tool(
        description = "Select a node in the figstudio app and show it: the tab becomes active and the view fits the node. An instance path (I1:2;3:4) reveals its outer instance. A figma.com link is shown only in the file it names: the open tab, or else the local copy `<file name>.fig` from the app's link folders (default ~/Downloads), which is opened first; other tools can read it after that. When several files match equally, it fails and asks for a pick in the app. Fails when the app does not answer within 5 s (60 s while opening a file)."
    )]
    async fn reveal_node(&self, Parameters(input): Parameters<RevealInput>) -> CallToolResult {
        let Some(live) = self.live_files() else {
            return tool_error(McpError::app("live mode only"));
        };
        // The node first: a link without a valid node-id fails the same way whether or not
        // its file is open, and never makes the app open a file.
        let (node, note) = match parse_node_id(&input.node_id) {
            Ok(NodeId::Node(id)) => (id, ""),
            Ok(NodeId::InstancePath(path)) => (
                path.instance(),
                " (an instance path: its outer instance is shown)",
            ),
            Err(error) => return tool_error(error.into()),
        };
        // A figma.com link names a file. It is shown in that file or not at all: falling back
        // to the active tab could reveal the same node ID in another document.
        let figma = input.file_key.is_none() && names_figma_file(&input.node_id);
        if figma && live.opens_links() {
            // The app resolves it against its tabs and local copies (a tie is an error).
            return match live.open_link(&input.node_id).await {
                Ok(done) => CallToolResult::success(vec![rmcp::model::ContentBlock::text(
                    format!("{done}{note}"),
                )]),
                Err(error) => tool_error(error),
            };
        }
        let key = if figma {
            // The browser bridge has no folders: the link must name exactly one open tab.
            let mut keys = self.link_files(&input.node_id);
            match keys.len() {
                1 => keys.pop(),
                0 => {
                    return tool_error(McpError::app("no open tab shows the file this link names"));
                }
                _ => {
                    return tool_error(McpError::app(
                        "several open tabs match this link; pass fileKey (see list_open_tabs)",
                    ));
                }
            }
        } else {
            self.file_for(input.file_key.as_deref(), Some(&input.node_id))
        };
        let picked = match live.pick(key.as_deref()) {
            Ok(picked) => picked,
            Err(error) => return tool_error(error),
        };
        let tab = picked.tab();
        if tab.doc.node(node).is_none() {
            return tool_error(McpError::app(format!("node {node} is not in this file")));
        }
        match live.reveal(tab.id, node).await {
            Ok(()) => CallToolResult::success(vec![rmcp::model::ContentBlock::text(format!(
                "revealed {node} in tab {}{note}",
                tab.id
            ))]),
            Err(error) => tool_error(error),
        }
    }
}

impl FigstudioServer {
    fn live_files(&self) -> Option<&LiveFiles> {
        match self.files() {
            Files::Live(live) => Some(live),
            Files::Registry(_) => None,
        }
    }
}

/// Whether `link` is a figma.com link with a file part.
fn names_figma_file(link: &str) -> bool {
    matches!(
        parse_link(link).ok().and_then(|link| link.file),
        Some(FileHint::Figma { .. })
    )
}

fn page(tab: &TabInfo) -> Value {
    let name = tab.doc.node(tab.page).map(|node| node.name.as_str());
    json!({"id": tab.page.to_string(), "name": name})
}

fn describe(doc: &Document, id: Guid) -> Option<Value> {
    let node = doc.node(id)?;
    let bounds = figstudio_render::node_bounds(doc, id).map(
        |rect| json!({"x": rect.x0, "y": rect.y0, "width": rect.width(), "height": rect.height()}),
    );
    Some(json!({
        "nodeId": id.to_string(),
        "name": node.name,
        "kind": node.kind,
        "bounds": bounds,
    }))
}
