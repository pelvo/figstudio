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
    for item in copy::items(tab, id) {
        if ui.button(item.label).clicked() {
            *picked = Some(item);
            ui.close();
        }
    }
    if ui.button("Copy as PNG").clicked() {
        *picked = Some(Item::png("Copy as PNG", "image", Job::Node(id)));
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
    on_context(&label, tab, node.id, picked);
    if ui.small_button("Copy").clicked() {
        *picked = copy::items(tab, node.id)
            .into_iter()
            .find(|i| i.label == "Copy text");
    }
}
