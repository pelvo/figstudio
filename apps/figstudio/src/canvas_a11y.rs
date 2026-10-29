//! The canvas's layers in the AccessKit tree: the visible layers of the page, each a node under
//! the canvas with its name, kind, place on screen and selected state, and a press action that
//! selects it. Screen readers and agents (computer use) find a layer by name instead of by
//! pixels.
//!
//! Only what is on screen is listed, at most [`MAX_NODES`], walking at most [`MAX_VISITS`]
//! layers: a big page must not cost a frame. With AccessKit off (no screen reader or agent
//! attached) nothing is walked at all.

use egui::accesskit::{Action, Role};
use std::collections::HashMap;
use std::sync::Arc;

use egui::{Event, Id, Rect, Response, Ui, UiBuilder};
use figstudio_fig::{Document, Guid};
use figstudio_render::geom;
use figstudio_render::kurbo::{self, Affine};

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

/// One listed layer: its place on screen (inside the screen and every clipping frame above
/// it) and the listed layer it hangs under (`None`: the canvas).
#[derive(Clone, Copy, Debug)]
struct Item {
    guid: Guid,
    parent: Option<usize>,
    shown: Rect,
}

/// The layers listed for one view, kept until the view, page, file or canvas place changes:
/// the walk is the costly part, and the tree is sent again every frame.
#[derive(Clone, Debug, Default)]
struct Listing {
    key: u64,
    items: Arc<Vec<Item>>,
}

fn listing_id() -> Id {
    Id::new("canvas-a11y-listing")
}

/// What the listing depends on: the tab and page, the file (a reload swaps the document), the
/// view and where the canvas is.
fn listing_key(tab: &Tab, origin: egui::Pos2, screen: Rect) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::hash::DefaultHasher::new();
    (tab.id, tab.page, Arc::as_ptr(&tab.doc) as usize).hash(&mut h);
    for v in [
        tab.view.zoom,
        tab.view.pan.x,
        tab.view.pan.y,
        origin.x,
        origin.y,
        screen.min.x,
        screen.min.y,
        screen.max.x,
        screen.max.y,
    ] {
        v.to_bits().hash(&mut h);
    }
    h.finish()
}

/// Selects the layer a screen reader or agent pressed (an AccessKit Click on its node) since
/// the last frame: one of the layers listed last. Returns true when it did.
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
    let listed = ui
        .ctx()
        .data(|d| d.get_temp::<Listing>(listing_id()))
        .unwrap_or_default();
    let picked = listed
        .items
        .iter()
        .find(|item| pressed.contains(&node_id(tab.id, item.guid).accesskit_id()))
        .map(|item| item.guid);
    if let Some(guid) = picked {
        tab.select(Some(guid));
    }
    picked.is_some()
}

/// Most layers looked at to work out the reach of a page's layers ([`reach`]); a page past it
/// keeps the rest unknown, and the walk looks into those.
const MAX_REACH: usize = 1_000_000;

/// How far each visible layer of `page` and everything inside it reaches, in page space: its
/// own bounds, joined with its children's unless it clips them. It does not depend on the view,
/// so it is worked out once per file and page ([`reach_of`]) and lets the walk skip a subtree
/// that does not reach the screen.
fn reach(doc: &Document, page: Guid) -> HashMap<Guid, kurbo::Rect> {
    let mut out = HashMap::new();
    // (layer, its page transform once known, whether its children are done).
    let mut stack: Vec<(Guid, Affine, bool)> = doc
        .children(page)
        .iter()
        .map(|&g| (g, Affine::IDENTITY, false))
        .collect();
    let mut visits = 0;
    while let Some((guid, transform, done)) = stack.pop() {
        let Some(node) = doc.node(guid) else { continue };
        if done {
            let (w, h) = geom::size(&node.data);
            let mut r = geom::transformed_bounds(transform, kurbo::Rect::new(0.0, 0.0, w, h));
            if !geom::clips_content(&node.kind, &node.data) {
                for c in doc.children(guid) {
                    if let Some(c) = out.get(c) {
                        r = r.union(*c);
                    }
                }
            }
            out.insert(guid, r);
            continue;
        }
        visits += 1;
        if visits > MAX_REACH {
            break;
        }
        if node.data.flag("visible") == Some(false) {
            continue;
        }
        let transform = transform * geom::affine(node.data.get("transform"));
        stack.push((guid, transform, true));
        stack.extend(doc.children(guid).iter().map(|&g| (g, transform, false)));
    }
    out
}

/// [`reach`] for the tab's file and page, kept in `ctx` while they stay.
fn reach_of(ctx: &egui::Context, tab: &Tab) -> Arc<HashMap<Guid, kurbo::Rect>> {
    type Kept = (usize, Guid, Arc<HashMap<Guid, kurbo::Rect>>);
    let id = Id::new("canvas-a11y-reach");
    let file = Arc::as_ptr(&tab.doc) as usize;
    if let Some((f, p, r)) = ctx.data(|d| d.get_temp::<Kept>(id))
        && (f, p) == (file, tab.page)
    {
        return r;
    }
    let r = Arc::new(reach(&tab.doc, tab.page));
    ctx.data_mut(|d| d.insert_temp::<Kept>(id, (file, tab.page, Arc::clone(&r))));
    r
}

/// The visible layers of the page shown in `screen`, depth first. A layer is listed where it
/// shows: inside the screen and every clipping frame above it. A layer that does not clip its
/// content is looked into even when it is off screen (its children may lie outside it); a
/// clipping one that is off screen is not. At most [`MAX_NODES`] listed, [`MAX_VISITS`] visited.
fn walk(
    tab: &Tab,
    reach: &HashMap<Guid, kurbo::Rect>,
    origin: egui::Pos2,
    screen: Rect,
) -> Vec<Item> {
    let doc = &tab.doc;
    // A layer's page bounds as `figstudio_render::node_bounds` gives them, from the transform
    // carried down the walk (its own transform after its parent's) instead of climbing to the
    // page again for every layer.
    let on_screen = |b: kurbo::Rect| {
        let a = tab
            .view
            .to_screen(origin, egui::pos2(b.x0 as f32, b.y0 as f32));
        let z = tab
            .view
            .to_screen(origin, egui::pos2(b.x1 as f32, b.y1 as f32));
        Rect::from_two_pos(a, z)
    };
    let rect_of = |transform: Affine, data: &figstudio_fig::Val| {
        let (w, h) = geom::size(data);
        let b = geom::transformed_bounds(transform, kurbo::Rect::new(0.0, 0.0, w, h));
        let a = tab
            .view
            .to_screen(origin, egui::pos2(b.x0 as f32, b.y0 as f32));
        let z = tab
            .view
            .to_screen(origin, egui::pos2(b.x1 as f32, b.y1 as f32));
        let r = Rect::from_two_pos(a, z);
        r.is_finite().then_some(r)
    };
    // (layer, the listed layer above it, the clip from the screen and clipping frames, its
    // parent's page transform).
    let mut stack: Vec<(Guid, Option<usize>, Rect, Affine)> = doc
        .children(tab.page)
        .iter()
        .rev()
        .map(|&g| (g, None, screen, Affine::IDENTITY))
        .collect();
    let mut items = Vec::new();
    let mut visits = 0;
    while let Some((guid, parent, clip, above)) = stack.pop() {
        visits += 1;
        if visits > MAX_VISITS || items.len() >= MAX_NODES {
            break;
        }
        let Some(node) = doc.node(guid) else { continue };
        if node.data.flag("visible") == Some(false) {
            continue;
        }
        // Nothing of it or inside it reaches what shows: skip it all.
        if let Some(r) = reach.get(&guid)
            && !on_screen(*r).intersects(clip)
        {
            continue;
        }
        let transform = above * geom::affine(node.data.get("transform"));
        let rect = rect_of(transform, &node.data);
        let shown = rect.map(|r| r.intersect(clip));
        let mut child_parent = parent;
        if let Some(shown) = shown
            && shown.width() >= MIN_SIDE
            && shown.height() >= MIN_SIDE
        {
            child_parent = Some(items.len());
            items.push(Item {
                guid,
                parent,
                shown,
            });
        }
        let child_clip = match rect {
            Some(r) if geom::clips_content(&node.kind, &node.data) => r.intersect(clip),
            _ => clip,
        };
        if !child_clip.is_positive() {
            continue;
        }
        stack.extend(
            doc.children(guid)
                .iter()
                .rev()
                .map(|&g| (g, child_parent, child_clip, transform)),
        );
    }
    items
}

/// Adds the page's visible layers under the canvas node (`canvas` is its response).
pub fn layers(ui: &mut Ui, canvas: &Response, tab: &Tab, origin: egui::Pos2) {
    // Without a screen reader or agent egui keeps no tree: walk nothing.
    if ui.ctx().accesskit_node_builder(canvas.id, |_| ()).is_none() {
        return;
    }
    let screen = canvas.rect;
    let key = listing_key(tab, origin, screen);
    let cached = ui
        .ctx()
        .data(|d| d.get_temp::<Listing>(listing_id()))
        .filter(|l| l.key == key);
    let items = match cached {
        Some(l) => l.items,
        None => {
            let reach = reach_of(ui.ctx(), tab);
            let items = Arc::new(walk(tab, &reach, origin, screen));
            let listing = Listing {
                key,
                items: Arc::clone(&items),
            };
            ui.ctx().data_mut(|d| d.insert_temp(listing_id(), listing));
            items
        }
    };
    let doc = &tab.doc;
    for item in items.iter() {
        let Some(node) = doc.node(item.guid) else {
            continue;
        };
        let id = node_id(tab.id, item.guid);
        let parent = item
            .parent
            .map_or(canvas.id, |i| node_id(tab.id, items[i].guid));
        // A child Ui gives the node its place in the tree (under `parent`); it allocates
        // nothing, so it takes no pointer input from the canvas.
        let _ = ui.new_child(
            UiBuilder::new()
                .id(id)
                .max_rect(item.shown)
                .sense(egui::Sense::hover())
                .accessibility_parent(parent),
        );
        ui.ctx().accesskit_node_builder(id, |n| {
            describe(n, doc, item.guid, &node.kind, &node.data, tab.selection);
            let r = item.shown;
            n.set_bounds(egui::accesskit::Rect {
                x0: r.min.x.into(),
                y0: r.min.y.into(),
                x1: r.max.x.into(),
                y1: r.max.y.into(),
            });
        });
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

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect, Vec2};
    use figstudio_fig::{Guid, Val};

    use crate::labels::tests::{document, node};
    use crate::tab::{Tab, View};

    fn tab(nodes: Vec<figstudio_fig::Node>) -> Tab {
        let mut tab = Tab::new(
            1,
            std::path::PathBuf::from("synthetic.fig"),
            std::sync::Arc::new(document(nodes)),
            None,
            std::time::Duration::ZERO,
        );
        tab.page = Guid(9, 1);
        tab.view = View {
            pan: Vec2::ZERO,
            zoom: 1.0,
        };
        tab
    }

    fn no_clip(mut n: figstudio_fig::Node) -> figstudio_fig::Node {
        n.data.set("frameMaskDisabled", Val::Bool(true));
        n
    }

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1000.0, 1000.0));

    #[test]
    fn a_layer_outside_its_non_clipping_frame_is_listed_even_when_the_frame_is_off_screen() {
        // The frame sits left of the screen; its child is placed back on screen.
        let t = tab(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            no_clip(node(2, 1, "FRAME", "Off", -500.0, 100.0)),
            node(3, 2, "RECTANGLE", "Back on screen", 700.0, 100.0),
        ]);
        let items = super::walk(&t, &super::reach(&t.doc, t.page), Pos2::ZERO, SCREEN);
        let listed: Vec<Guid> = items.iter().map(|i| i.guid).collect();
        assert_eq!(listed, [Guid(9, 3)], "only the child shows");
        assert_eq!(
            items[0].parent, None,
            "its frame is not listed: it hangs off the canvas"
        );
    }

    #[test]
    fn a_layer_clipped_by_its_frame_is_listed_only_where_it_shows() {
        let t = tab(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            node(2, 1, "FRAME", "Clips", 100.0, 200.0),
            // Half inside the frame: x 200..400 in a frame at 100..300.
            node(3, 2, "RECTANGLE", "Half out", 100.0, 200.0),
            // All outside it: clipped away.
            node(4, 2, "RECTANGLE", "Gone", 400.0, 100.0),
        ]);
        let items = super::walk(&t, &super::reach(&t.doc, t.page), Pos2::ZERO, SCREEN);
        let listed: Vec<Guid> = items.iter().map(|i| i.guid).collect();
        assert_eq!(listed, [Guid(9, 2), Guid(9, 3)]);
        let half = items[1].shown;
        assert_eq!((half.min.x, half.max.x), (200.0, 300.0), "{half:?}");
        assert_eq!(items[1].parent, Some(0));
    }

    /// The listing `super::layers` keeps in `ctx` after one frame of the canvas.
    fn listed_after_frame(ctx: &egui::Context, t: &Tab) -> std::sync::Arc<Vec<super::Item>> {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let canvas = ui.allocate_rect(SCREEN, egui::Sense::click());
            super::layers(ui, &canvas, t, Pos2::ZERO);
        });
        out.textures_delta.clear();
        ctx.data(|d| d.get_temp::<super::Listing>(super::listing_id()))
            .expect("a listing")
            .items
    }

    #[test]
    fn the_walk_runs_again_only_when_the_view_changes() {
        let mut t = tab(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            node(2, 1, "FRAME", "Frame", 100.0, 200.0),
        ]);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let first = listed_after_frame(&ctx, &t);
        assert_eq!(first.len(), 1);
        let again = listed_after_frame(&ctx, &t);
        assert!(
            std::sync::Arc::ptr_eq(&first, &again),
            "walked again for the same view"
        );
        t.view.pan.x += 10.0;
        let moved = listed_after_frame(&ctx, &t);
        assert!(
            !std::sync::Arc::ptr_eq(&first, &moved),
            "kept a stale listing"
        );
        assert_eq!(moved[0].shown.min.x, first[0].shown.min.x + 10.0);
    }

    #[test]
    fn without_accesskit_nothing_is_walked() {
        let t = tab(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            node(2, 1, "FRAME", "Frame", 100.0, 200.0),
        ]);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let canvas = ui.allocate_rect(SCREEN, egui::Sense::click());
            super::layers(ui, &canvas, &t, Pos2::ZERO);
        });
        out.textures_delta.clear();
        assert!(
            ctx.data(|d| d.get_temp::<super::Listing>(super::listing_id()))
                .is_none()
        );
    }

    /// Manual timing on a big page: FIGSTUDIO_A11Y_BENCH=<file.fig> cargo test --profile fast
    /// -p figstudio canvas_a11y::tests::timing -- --ignored --nocapture
    /// (`figstudio-cli synth tests/fixtures/structure.fig -o big.fig --copies 4000`).
    #[test]
    #[ignore = "manual timing; needs FIGSTUDIO_A11Y_BENCH"]
    fn timing() {
        let Some(path) = std::env::var_os("FIGSTUDIO_A11Y_BENCH") else {
            return;
        };
        let doc = figstudio_fig::Document::open(std::path::Path::new(&path)).expect("the file");
        let page = doc.pages[0];
        let mut t = Tab::new(
            1,
            std::path::PathBuf::from("bench.fig"),
            std::sync::Arc::new(doc),
            None,
            std::time::Duration::ZERO,
        );
        t.page = page;
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 880.0));
        crate::canvas::fit(&mut t, screen);
        let fit = t.view;
        let started = std::time::Instant::now();
        let reach = super::reach(&t.doc, t.page);
        println!(
            "reach of {} layers: {:?} (once per file and page)",
            reach.len(),
            started.elapsed()
        );
        for (what, zoom) in [("fit", 1.0), ("4x fit", 4.0), ("40x fit", 40.0)] {
            t.view = fit;
            t.view.zoom = fit.zoom * zoom;
            let mut times = Vec::new();
            let mut listed = 0;
            for _ in 0..20 {
                let started = std::time::Instant::now();
                listed = super::walk(&t, &reach, Pos2::ZERO, screen).len();
                times.push(started.elapsed());
            }
            times.sort();
            println!(
                "{what}: {listed} listed, walk median {:?}, max {:?}",
                times[times.len() / 2],
                times[times.len() - 1]
            );
        }
    }

    #[test]
    fn a_clipping_frame_off_screen_is_not_looked_into() {
        let t = tab(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            node(2, 1, "FRAME", "Off", -500.0, 100.0),
            node(3, 2, "RECTANGLE", "Would be on screen", 700.0, 100.0),
        ]);
        assert!(super::walk(&t, &super::reach(&t.doc, t.page), Pos2::ZERO, SCREEN).is_empty());
    }
}
