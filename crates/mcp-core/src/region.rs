//! The nodes of a page that a rectangle in page coordinates touches.
//!
//! Shared by the app's Copy region and (later) the MCP region tools.

use std::collections::VecDeque;

use figstudio_fig::{Document, Guid};
use figstudio_render::kurbo::Rect;
use figstudio_render::node_bounds;

/// Default row cap for a region listing.
pub const DEFAULT_LIMIT: usize = 50;
/// Default cap on nodes visited, so a huge file cannot stall the caller.
pub const DEFAULT_BUDGET: usize = 200_000;

/// One node that touches the region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegionNode {
    pub id: Guid,
    pub name: String,
    pub kind: String,
}

/// The nodes found, top-level first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Region {
    /// At most `limit` nodes, in breadth-first order from the page's children.
    pub nodes: Vec<RegionNode>,
    /// Every touching node found. A lower bound when `exhausted`.
    pub total: usize,
    /// Fewer nodes are listed than touch the region, or the walk ran out of budget.
    pub truncated: bool,
    /// The walk stopped at the budget, so `total` may be too low.
    pub exhausted: bool,
}

/// Visible nodes of `page` whose bounds overlap `rect` (`[x, y, w, h]`), breadth-first, so
/// top-level nodes come first. Invisible nodes and their subtrees are skipped. At most `budget`
/// nodes are visited and `limit` returned; touching edges do not count as overlap. A missing
/// page or an empty rect gives an empty region.
pub fn nodes_in(doc: &Document, page: Guid, rect: [f64; 4], limit: usize, budget: usize) -> Region {
    let area = Rect::new(rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]);
    let mut out = Region::default();
    if !(rect.iter().all(|v| v.is_finite()) && rect[2] > 0.0 && rect[3] > 0.0) {
        return out;
    }
    let mut queue: VecDeque<Guid> = doc.children(page).iter().copied().collect();
    let mut left = budget;
    while let Some(id) = queue.pop_front() {
        if left == 0 {
            out.exhausted = true;
            break;
        }
        left -= 1;
        let Some(node) = doc.node(id) else { continue };
        if node.data.flag("visible") == Some(false) {
            continue;
        }
        queue.extend(doc.children(id).iter().copied());
        if node_bounds(doc, id).is_some_and(|b| overlaps(&b, &area)) {
            out.total += 1;
            if out.nodes.len() < limit {
                out.nodes.push(RegionNode {
                    id,
                    name: node.name.clone(),
                    kind: node.kind.clone(),
                });
            }
        }
    }
    out.truncated = out.exhausted || out.total > out.nodes.len();
    out
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    span(a.x0, a.x1, b.x0, b.x1) && span(a.y0, a.y1, b.y0, b.y1)
}

/// Open intervals overlap; a zero-width node (a line) counts when it lies inside the region.
fn span(a0: f64, a1: f64, b0: f64, b1: f64) -> bool {
    (a1 > b0 && a0 < b1) || (a0 == a1 && a0 >= b0 && a0 <= b1)
}
