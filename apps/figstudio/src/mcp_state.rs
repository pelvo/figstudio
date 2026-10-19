//! View state and reveal semantics shared by desktop and browser MCP transports.
use crate::tab::Tab;
use figstudio_fig::Guid;
use std::sync::Arc;

type TabKey = (u64, usize, Guid, Option<Guid>, Option<[i64; 4]>);
#[derive(Debug, PartialEq)]
pub(crate) struct Key {
    tabs: Vec<TabKey>,
    active: Option<u64>,
}

pub(crate) fn viewport(tab: &Tab) -> Option<[f64; 4]> {
    let rect = tab.canvas_rect?;
    let min = tab.view.to_page(rect.min, rect.min);
    let max = tab.view.to_page(rect.min, rect.max);
    Some([min.x, min.y, max.x - min.x, max.y - min.y].map(f64::from))
}

pub(crate) fn key(tabs: &[Tab], active: usize) -> Key {
    Key {
        tabs: tabs
            .iter()
            .map(|t| {
                let view = viewport(t).map(|v| v.map(|x| x.round() as i64));
                (
                    t.id,
                    Arc::as_ptr(&t.doc) as usize,
                    t.page,
                    t.selection,
                    view,
                )
            })
            .collect(),
        active: tabs.get(active).map(|t| t.id),
    }
}

pub(crate) fn reveal(
    tabs: &mut [Tab],
    active: &mut usize,
    tab: u64,
    node: Guid,
) -> Result<(), String> {
    match tabs.iter().position(|t| t.id == tab) {
        Some(i) => {
            *active = i;
            tabs[i].show_node(node);
            Ok(())
        }
        None => Err("tab closed".to_owned()),
    }
}

#[cfg(test)]
#[path = "mcp_state_tests.rs"]
mod tests;
