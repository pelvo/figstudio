//! Text for the right-click Copy menus: a node's name, text, ID, link and agent block.
//!
//! Pure functions over the document, so they are unit-tested without egui (`copy_tests.rs`).

use std::path::Path;

use figstudio_fig::{Guid, Node, Val};
use figstudio_mcp_core::link::format_node_link;
use figstudio_mcp_core::node_id::NodeId;

use crate::tab::Tab;

/// One menu entry: the button label, the noun the toast shows ("Copied link"), and the text.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: &'static str,
    pub what: &'static str,
    pub text: String,
    /// Set for an image item: `text` is empty and `region_png::drive` renders and copies it.
    pub png: Option<crate::region_png::Job>,
}

impl Item {
    pub fn text(label: &'static str, what: &'static str, text: String) -> Item {
        Item {
            label,
            what,
            text,
            png: None,
        }
    }

    pub fn png(label: &'static str, what: &'static str, job: crate::region_png::Job) -> Item {
        Item {
            png: Some(job),
            ..Item::text(label, what, String::new())
        }
    }
}

/// The menu for node `id`, in display order. Empty when the node is not in the document.
/// "Copy name" and "Copy text" are left out when there is nothing to copy.
pub fn items(tab: &Tab, id: Guid) -> Vec<Item> {
    let Some(node) = tab.doc.node(id) else {
        return Vec::new();
    };
    let text = if tab.selection == Some(id) {
        tab.selected_text()
    } else {
        text(node).filter(|t| !t.is_empty()).map(str::to_owned)
    };
    items_with_text(tab, node, text)
}

/// Properties copy the full preview, including resolved inner-layer overrides.
pub fn layer_items(tab: &Tab, node: &Node) -> Vec<Item> {
    items_with_text(
        tab,
        node,
        text(node).filter(|t| !t.is_empty()).map(str::to_owned),
    )
}

fn items_with_text(tab: &Tab, node: &Node, text: Option<String>) -> Vec<Item> {
    let id = node.id;
    let mut out = Vec::new();
    if !node.name.is_empty() {
        out.push(item("Copy name", "name", node.name.clone()));
    }
    if let Some(text) = text {
        out.push(item("Copy text", "text", text));
    }
    out.push(item("Copy node ID", "node ID", id.to_string()));
    out.push(item("Copy link", "link", link(&file_key(tab), id)));
    out.push(item(
        "Copy for agent",
        "agent block",
        agent_block(tab, node),
    ));
    out
}

fn item(label: &'static str, what: &'static str, text: String) -> Item {
    Item::text(label, what, text)
}

/// The whole `textData.characters` of a TEXT node, not the 400-character Properties preview.
pub fn text(node: &Node) -> Option<&str> {
    if node.kind != "TEXT" {
        return None;
    }
    node.data
        .path(&["textData", "characters"])
        .and_then(Val::as_str)
}

/// `figstudio://<file key>?node-id=1-42`, which `parse_link` reads back.
pub fn link(file_key: &str, id: Guid) -> String {
    format_node_link(file_key, &NodeId::Node(id))
}

/// The file's stem (`structure` for `structure.fig`), the name the MCP server uses for files.
pub fn file_key(tab: &Tab) -> String {
    stem(&tab.path)
        .or_else(|| {
            tab.doc
                .file_name
                .as_deref()
                .and_then(|n| stem(Path::new(n)))
        })
        .unwrap_or_else(|| "untitled".into())
}

fn stem(path: &Path) -> Option<String> {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
}

/// The three-line block for an agent: the link, the file and page, then the node and its box.
/// The box is left out when the node has no bounds.
pub fn agent_block(tab: &Tab, node: &Node) -> String {
    let doc = &tab.doc;
    let page = doc
        .page_of(node.id)
        .and_then(|p| doc.node(p))
        .map_or("", |p| p.name.as_str());
    let bounds = figstudio_render::node_bounds(doc, node.id).map_or(String::new(), |b| {
        format!(
            " x={} y={} w={} h={}",
            num(b.x0),
            num(b.y0),
            num(b.width()),
            num(b.height())
        )
    });
    // A name with a line break would split the block's third line.
    let name = node.name.replace(['\n', '\r'], " ");
    format!(
        "{}\nfile: {} · page: {page}\nnode: {} \"{name}\" ({}){bounds}",
        link(&file_key(tab), node.id),
        tab.title(),
        node.id,
        node.kind,
    )
}

/// Two decimals, without trailing zeros: `120`, `33.5`.
fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    format!("{r}")
}

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;
