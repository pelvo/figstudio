//! The canvas's layers in the AccessKit tree: the visible layers of the page, each a node under
//! the canvas with its name, kind, place on screen and selected state, and a press action that
//! selects it. Screen readers and agents (computer use) find a layer by name instead of by
//! pixels.
//!
//! Only what is on screen is listed, at most [`MAX_NODES`], walking at most [`MAX_VISITS`]
//! layers: a big page must not cost a frame. With AccessKit off (no screen reader or agent
//! attached) nothing is walked at all.

use egui::accesskit::{Action, Role};
use egui::{Event, Id, Rect, Response, Ui, UiBuilder};
use figstudio_fig::{Document, Guid};

use crate::panels::display_name;
use crate::tab::Tab;

/// Most layer nodes listed in one frame.
pub const MAX_NODES: usize = 400;
/// Most layers looked at in one frame (hidden and off-screen ones included).
pub const MAX_VISITS: usize = 20_000;
/// The longest text content a text layer reports.
const MAX_TEXT: usize = 300;
/// A layer smaller than this on screen (points, either side) is not listed.
const MIN_SIDE: f32 = 2.0;

/// The node id of layer `guid` on the canvas of tab `tab_id`.
pub fn node_id(tab_id: u64, guid: Guid) -> Id {
    Id::new(("canvas-layer", tab_id, guid))
}

/// The stable automation id of layer `guid` (`AXIdentifier` on macOS).
pub fn author_id(guid: Guid) -> String {
    format!("canvas.layer.{guid}")
}

/// Selects the layer a screen reader or agent pressed (an AccessKit Click on its node) since
/// the last frame. Returns true when it did.
pub fn take_presses(ui: &Ui, tab: &mut Tab) -> bool {
    let pressed: Vec<egui::accesskit::NodeId> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                Event::AccessKitActionRequest(r) if r.action == Action::Click => {
                    Some(r.target_node)
                }
                _ => None,
            })
            .collect()
    });
    if pressed.is_empty() {
        return false;
    }
    let mut picked = None;
    walk(&tab.doc, tab.page, &mut |guid| {
        if pressed.contains(&node_id(tab.id, guid).accesskit_id()) {
            picked = Some(guid);
        }
    });
    if let Some(guid) = picked {
        tab.select(Some(guid));
    }
    picked.is_some()
}

/// Every layer of `page`, depth first, at most [`MAX_VISITS`].
fn walk(doc: &Document, page: Guid, f: &mut impl FnMut(Guid)) {
    let mut stack: Vec<Guid> = doc.children(page).iter().rev().copied().collect();
    let mut visits = 0;
    while let Some(id) = stack.pop() {
        visits += 1;
        if visits > MAX_VISITS {
            return;
        }
        f(id);
        stack.extend(doc.children(id).iter().rev().copied());
    }
}

/// Adds the page's visible layers under the canvas node (`canvas` is its response).
pub fn layers(ui: &mut Ui, canvas: &Response, tab: &Tab, origin: egui::Pos2) {
    // Without a screen reader or agent egui keeps no tree: walk nothing.
    if ui.ctx().accesskit_node_builder(canvas.id, |_| ()).is_none() {
        return;
    }
    let screen = canvas.rect;
    let doc = &tab.doc;
    let rect_of = |id: Guid| {
        let b = figstudio_render::node_bounds(doc, id)?;
        let a = tab
            .view
            .to_screen(origin, egui::pos2(b.x0 as f32, b.y0 as f32));
        let z = tab
            .view
            .to_screen(origin, egui::pos2(b.x1 as f32, b.y1 as f32));
        Some(Rect::from_two_pos(a, z))
    };
    // (layer, its accessibility parent): top-level layers hang off the canvas.
    let mut stack: Vec<(Guid, Id)> = doc
        .children(tab.page)
        .iter()
        .rev()
        .map(|&g| (g, canvas.id))
        .collect();
    let (mut listed, mut visits) = (0, 0);
    while let Some((guid, parent)) = stack.pop() {
        visits += 1;
        if visits > MAX_VISITS || listed >= MAX_NODES {
            break;
        }
        let Some(node) = doc.node(guid) else { continue };
        if node.data.flag("visible") == Some(false) {
            continue;
        }
        let Some(rect) = rect_of(guid) else { continue };
        // Children usually lie inside their parent: an off-screen layer's subtree is skipped.
        if !rect.intersects(screen) {
            continue;
        }
        let shown = rect.intersect(screen);
        let id = node_id(tab.id, guid);
        let mut child_parent = parent;
        if shown.width() >= MIN_SIDE && shown.height() >= MIN_SIDE {
            // A child Ui gives the node its place in the tree (under `parent`); it allocates
            // nothing, so it takes no pointer input from the canvas.
            let _ = ui.new_child(
                UiBuilder::new()
                    .id(id)
                    .max_rect(shown)
                    .sense(egui::Sense::hover())
                    .accessibility_parent(parent),
            );
            ui.ctx().accesskit_node_builder(id, |n| {
                describe(n, doc, guid, &node.kind, &node.data, tab.selection);
                n.set_bounds(egui::accesskit::Rect {
                    x0: shown.min.x.into(),
                    y0: shown.min.y.into(),
                    x1: shown.max.x.into(),
                    y1: shown.max.y.into(),
                });
            });
            listed += 1;
            child_parent = id;
        }
        stack.extend(doc.children(guid).iter().rev().map(|&g| (g, child_parent)));
    }
}

fn describe(
    n: &mut egui::accesskit::Node,
    doc: &Document,
    guid: Guid,
    kind: &str,
    data: &figstudio_fig::Val,
    selection: Option<Guid>,
) {
    let name = doc.node(guid).map(display_name).unwrap_or_default();
    let kind_name = kind.to_lowercase().replace('_', " ");
    let role = match kind {
        "TEXT" => Role::Label,
        "FRAME" | "GROUP" | "SECTION" | "SYMBOL" | "INSTANCE" | "COMPONENT_SET"
        | "BOOLEAN_OPERATION" => Role::Group,
        _ => Role::Image,
    };
    n.set_role(role);
    n.set_role_description(match kind {
        "SYMBOL" => "component".to_owned(),
        _ => kind_name,
    });
    n.set_label(name.clone());
    if kind == "TEXT" {
        // A text layer reads as what it says (a label's name is its value).
        let text = crate::text_select::characters(data);
        let text: String = text.chars().take(MAX_TEXT).collect();
        n.set_value(if text.trim().is_empty() { name } else { text });
    }
    let (w, h) = figstudio_render::geom::size(data);
    n.set_description(format!("{} × {}", round(w), round(h)));
    n.set_selected(selection == Some(guid));
    n.add_action(Action::Click);
    n.set_author_id(author_id(guid));
}

fn round(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 {
        format!("{r:.0}")
    } else {
        format!("{r}")
    }
}
