//! The right-click Copy menu for one node, drawn with egui's context menu.
//!
//! A picked item is written to `picked`, a slot the caller owns for the frame; the caller
//! stores it on the tab and `main.rs` copies it.

use figstudio_fig::{Guid, Node};

use crate::copy::{self, Item};
use crate::region_png::Job;
use crate::tab::Tab;
use crate::theme;

/// Shows the Copy items for node `id` inside an open context menu.
pub fn node_menu(ui: &mut egui::Ui, tab: &Tab, id: Guid, picked: &mut Option<Item>) {
    menu_items(ui, tab, copy::items(tab, id), id, picked);
}

fn menu_items(ui: &mut egui::Ui, tab: &Tab, items: Vec<Item>, id: Guid, picked: &mut Option<Item>) {
    for item in items {
        if ui.button(item.label).clicked() {
            *picked = Some(item);
            ui.close();
        }
    }
    if ui.button("Copy as PNG").clicked() {
        *picked = Some(Item::png("Copy as PNG", "image", Job::Node(id)));
        ui.close();
    }
    if ui.button("Copy as WebP").clicked() {
        *picked = Some(Item::png("Copy as WebP", "WebP image", Job::WebpNode(id)));
        ui.close();
    }
    if ui.button("Copy as SVG").clicked() {
        *picked = Some(Item::png("Copy as SVG", "SVG", Job::SvgNode(id)));
        ui.close();
    }
    // Checked every frame the menu is open, so only the node's own bindings are looked at.
    if tab
        .doc
        .node(id)
        .is_some_and(|n| figstudio_mcp_core::motion::has_motion(&n.data))
        && ui.button("Copy CSS animation").clicked()
    {
        if let Some(css) = figstudio_mcp_core::motion::node_css(&tab.doc, &id.to_string()) {
            *picked = Some(Item::text("Copy CSS animation", "CSS animation", css));
        }
        ui.close();
    }
}

/// Right-click on `response` opens the node menu. Returns the item picked this frame.
pub fn on_context(response: &egui::Response, tab: &Tab, id: Guid, picked: &mut Option<Item>) {
    crate::chrome::context_menu(response, |ui| node_menu(ui, tab, id, picked));
}

/// Copies a picked item to the clipboard. Returns the toast text.
pub fn copy(ctx: &egui::Context, item: Item) -> String {
    let toast = format!("Copied {}", item.what);
    ctx.copy_text(item.text);
    toast
}

/// The Text section's characters: a 400-character preview, a Copy button and the node menu.
pub fn text_block(ui: &mut egui::Ui, tab: &Tab, node: &Node, picked: &mut Option<Item>) {
    let Some(full) = copy::text(node) else { return };
    let preview: String = full.chars().take(400).collect();
    let label = ui.add(egui::Label::new(
        egui::RichText::new(preview).color(theme::of(ui).text_dim),
    ));
    crate::chrome::context_menu(&label, |ui| {
        menu_items(ui, tab, copy::layer_items(tab, node), node.id, picked);
    });
    if ui.small_button("Copy").clicked() && !full.is_empty() {
        *picked = Some(Item::text("Copy text", "text", full.to_owned()));
    }
}
