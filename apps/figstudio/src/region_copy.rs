//! The region menu and the text it copies: the region link, the file and page, and the nodes
//! that touch the rectangle, top-level first.
//!
//! `region_text` is a pure function over the document (tests in `region_copy_tests.rs`).

use egui::Rect;
use figstudio_fig::Guid;
use figstudio_mcp_core::link::format_region_link;
use figstudio_mcp_core::region::{self, Region};

use crate::copy::{self, Item};
use crate::marquee::region_array;
use crate::region_png::Job;
use crate::tab::Tab;

/// The region menu body: Copy region (text) and Copy region as PNG.
pub fn menu(ui: &mut egui::Ui, tab: &Tab, rect: Rect, picked: &mut Option<Item>) {
    if ui.button("Copy region").clicked() {
        *picked = Some(text_item(tab, rect));
        ui.close();
    }
    if ui.button("Copy region as PNG").clicked() {
        let job = Job::Region {
            page: tab.page,
            rect: region_array(rect),
            zoom: tab.view.zoom * ui.pixels_per_point(),
        };
        *picked = Some(Item::png("Copy region as PNG", "region image", job));
        ui.close();
    }
}

/// The text item. The text is built here, once, when the entry is clicked, not while the menu is
/// open: walking a big page every frame would stall the canvas.
fn text_item(tab: &Tab, rect: Rect) -> Item {
    let text = region_text(tab, tab.page, region_array(rect)).unwrap_or_else(|e| format!("{e:#}"));
    Item::text("Copy region", "region", text)
}

/// The three-part text for `rect` (`[x, y, w, h]`) on `page`: link, `file: … · page: …`, then
/// `nodes (k of N, top-level first):` and one `1:42 "Name" (KIND)` row per node (at most 50).
///
/// # Errors
/// A message for the toast when the rectangle cannot be written as a link.
pub fn region_text(tab: &Tab, page: Guid, rect: [f64; 4]) -> anyhow::Result<String> {
    let link = format_region_link(&copy::file_key(tab), page, rect)
        .map_err(|_| anyhow::anyhow!("Region is too small to copy"))?;
    let found = region::nodes_in(
        &tab.doc,
        page,
        rect,
        region::DEFAULT_LIMIT,
        region::DEFAULT_BUDGET,
    );
    let page_name = tab.doc.node(page).map_or("", |p| p.name.as_str());
    Ok(format!(
        "{link}\nfile: {} · page: {}\n{}",
        tab.title(),
        flat(page_name),
        node_rows(&found)
    ))
}

/// A name or page title on one line: a line break would split its row.
fn flat(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn node_rows(found: &Region) -> String {
    let more = if found.exhausted { "+" } else { "" };
    let mut out = format!(
        "nodes ({} of {}{more}, top-level first):",
        found.nodes.len(),
        found.total
    );
    for n in &found.nodes {
        out.push_str(&format!("\n{} \"{}\" ({})", n.id, flat(&n.name), n.kind));
    }
    out
}

#[cfg(test)]
#[path = "region_copy_tests.rs"]
mod tests;
