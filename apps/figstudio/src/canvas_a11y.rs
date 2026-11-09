//! The canvas's layers in the AccessKit tree: the visible layers of the page, each a node under
//! the canvas with its name, kind, place on screen and selected state, and a press action that
//! selects it. Screen readers and agents (computer use) find a layer by name instead of by
//! pixels.
//!
//! Geometrically visible layers are listed, at most [`MAX_NODES`], walking at most
//! [`MAX_VISITS`] layers; counting omissions is bounded too. With AccessKit off (no screen
//! reader or agent attached) nothing is walked at all.

use egui::accesskit::{Action, Role};
use std::collections::{HashMap, HashSet};
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
/// The longest layer name or text value reported.
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
    omitted: usize,
    limited: bool,
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
    if !ui.is_enabled() || !ui.memory(|m| m.allows_interaction(ui.layer_id())) {
        return false;
    }
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

/// Work per UI frame, including both entering a node and finishing its subtree.
const REACH_STEPS: usize = 2048;
const MAX_REACH: usize = 1_000_000;

fn rect_of(transform: Affine, data: &figstudio_fig::Val) -> Option<kurbo::Rect> {
    let (w, h) = geom::size(data);
    let r = geom::transformed_bounds(transform, kurbo::Rect::new(0.0, 0.0, w, h));
    r.is_finite().then_some(r)
}

/// Owned by the tab, so closed documents cannot leave reach data for a new tab.
#[derive(Debug, Default)]
pub(crate) struct ReachCache {
    doc: std::sync::Weak<Document>,
    pages: HashMap<Guid, Reach>,
}

#[derive(Debug)]
struct ReachFrame {
    guid: Guid,
    transform: kurbo::Affine,
    next_child: usize,
    bounds: Option<kurbo::Rect>,
    clips: bool,
}

#[derive(Debug)]
struct Reach {
    bounds: Arc<HashMap<Guid, kurbo::Rect>>,
    seen: HashSet<Guid>,
    stack: Vec<ReachFrame>,
    limited: bool,
}

impl Reach {
    fn new(page: Guid) -> Self {
        Self {
            bounds: Arc::default(),
            seen: HashSet::from([page]),
            stack: vec![ReachFrame {
                guid: page,
                transform: kurbo::Affine::IDENTITY,
                next_child: 0,
                bounds: None,
                clips: false,
            }],
            limited: false,
        }
    }

    fn advance(&mut self, doc: &Document, steps: usize) {
        for _ in 0..steps {
            let Some(frame) = self.stack.last_mut() else {
                break;
            };
            if let Some(&guid) = doc.children(frame.guid).get(frame.next_child) {
                frame.next_child += 1;
                if self.seen.len() >= MAX_REACH {
                    self.limited = true;
                    self.stack.clear();
                    break;
                }
                if !self.seen.insert(guid) {
                    continue;
                }
                let Some(node) = doc
                    .node(guid)
                    .filter(|n| n.data.flag("visible") != Some(false))
                else {
                    continue;
                };
                let transform = frame.transform * geom::affine(node.data.get("transform"));
                self.stack.push(ReachFrame {
                    guid,
                    transform,
                    next_child: 0,
                    bounds: rect_of(transform, &node.data),
                    clips: geom::clips_content(&node.kind, &node.data),
                });
            } else if let Some(done) = self.stack.pop()
                && let Some(bounds) = done.bounds
            {
                Arc::make_mut(&mut self.bounds).insert(done.guid, bounds);
                if let Some(parent) = self.stack.last_mut().filter(|f| !f.clips) {
                    parent.bounds = Some(parent.bounds.map_or(bounds, |r| r.union(bounds)));
                }
            }
        }
        if self.stack.is_empty() {
            self.seen = HashSet::new();
            self.stack = Vec::new();
        }
    }
}

/// Incremental reach, retained for each page until its tab or document is replaced.
fn reach_of(tab: &mut Tab) -> Option<(Arc<HashMap<Guid, kurbo::Rect>>, bool)> {
    let cache = &mut tab.a11y_reach;
    let doc = Arc::downgrade(&tab.doc);
    if !cache.doc.ptr_eq(&doc) {
        cache.pages.clear();
        cache.doc = doc;
    }
    let reach = cache
        .pages
        .entry(tab.page)
        .or_insert_with(|| Reach::new(tab.page));
    reach.advance(&tab.doc, REACH_STEPS);
    reach
        .stack
        .is_empty()
        .then(|| (Arc::clone(&reach.bounds), reach.limited))
}

#[cfg(test)]
fn reach(doc: &Document, page: Guid) -> HashMap<Guid, kurbo::Rect> {
    let mut reach = Reach::new(page);
    while !reach.stack.is_empty() {
        reach.advance(doc, REACH_STEPS);
    }
    Arc::unwrap_or_clone(reach.bounds)
}

#[derive(Debug, Default)]
struct Walk {
    items: Vec<Item>,
    omitted: usize,
    limited: bool,
}

/// Breadth-first: show the page's screens before filling the cap with their contents.
fn walk(tab: &Tab, reach: &HashMap<Guid, kurbo::Rect>, origin: egui::Pos2, screen: Rect) -> Walk {
    let doc = &tab.doc;
    let on_screen = |b: kurbo::Rect| {
        let a = tab
            .view
            .to_screen(origin, egui::pos2(b.x0 as f32, b.y0 as f32));
        let z = tab
            .view
            .to_screen(origin, egui::pos2(b.x1 as f32, b.y1 as f32));
        Rect::from_two_pos(a, z)
    };
    let mut out = Walk::default();
    let roots = doc.children(tab.page);
    let mut queue: std::collections::VecDeque<_> = roots
        .iter()
        .take(MAX_VISITS)
        .map(|&g| (g, None, screen, Affine::IDENTITY))
        .collect();
    out.limited = roots.len() > MAX_VISITS;
    let mut seen = HashSet::from([tab.page]);
    let mut visits = 0;
    while let Some((guid, parent, clip, above)) = queue.pop_front() {
        visits += 1;
        if !seen.insert(guid) {
            continue;
        }
        let Some(node) = doc
            .node(guid)
            .filter(|n| n.data.flag("visible") != Some(false))
        else {
            continue;
        };
        if let Some(r) = reach.get(&guid)
            && !on_screen(*r).intersects(clip)
        {
            continue;
        }
        let transform = above * geom::affine(node.data.get("transform"));
        let rect = rect_of(transform, &node.data)
            .map(on_screen)
            .filter(|r| r.is_finite());
        let shown = rect.map(|r| r.intersect(clip));
        let mut child_parent = parent;
        if let Some(shown) = shown
            && shown.width() >= MIN_SIDE
            && shown.height() >= MIN_SIDE
        {
            if out.items.len() < MAX_NODES {
                child_parent = Some(out.items.len());
                out.items.push(Item {
                    guid,
                    parent,
                    shown,
                });
            } else {
                out.omitted += 1;
                // Count only one more batch, rather than walk a whole page for an exact total.
                if out.omitted >= MAX_NODES {
                    out.limited = true;
                    break;
                }
            }
        }
        let child_clip = match rect {
            Some(r) if geom::clips_content(&node.kind, &node.data) => r.intersect(clip),
            _ => clip,
        };
        if !child_clip.is_positive() {
            continue;
        }
        for &child in doc.children(guid) {
            if visits + queue.len() >= MAX_VISITS {
                out.limited = true;
                break;
            }
            queue.push_back((child, child_parent, child_clip, transform));
        }
    }
    out
}

fn status(ui: &mut Ui, canvas: &Response, label: &str, loading: bool) {
    let id = canvas.id.with("layer-list-status");
    let _ = ui.new_child(
        UiBuilder::new()
            .id(id)
            .max_rect(canvas.rect)
            .accessibility_parent(canvas.id),
    );
    ui.ctx().accesskit_node_builder(id, |n| {
        n.set_role(if loading {
            Role::ProgressIndicator
        } else {
            Role::Label
        });
        n.set_label(label);
        if !loading {
            n.set_value(label);
        }
        n.set_live(egui::accesskit::Live::Polite);
        n.set_author_id("canvas.layers.status");
    });
}

/// Adds geometrically visible layers under the canvas node; does not test occlusion/alpha.
pub fn layers(ui: &mut Ui, canvas: &Response, tab: &mut Tab, origin: egui::Pos2) {
    if ui.ctx().accesskit_node_builder(canvas.id, |_| ()).is_none() {
        return;
    }
    let screen = canvas.rect;
    let key = listing_key(tab, origin, screen);
    let cached = ui
        .ctx()
        .data(|d| d.get_temp::<Listing>(listing_id()))
        .filter(|l| l.key == key);
    let listing = match cached {
        Some(l) => l,
        None => {
            let Some((reach, reach_limited)) = reach_of(tab) else {
                ui.ctx().data_mut(|d| d.remove::<Listing>(listing_id()));
                status(ui, canvas, "Preparing canvas layers…", true);
                ui.ctx().request_repaint();
                return;
            };
            let walked = walk(tab, &reach, origin, screen);
            let listing = Listing {
                key,
                items: Arc::new(walked.items),
                omitted: walked.omitted,
                limited: walked.limited || reach_limited,
            };
            ui.ctx()
                .data_mut(|d| d.insert_temp(listing_id(), listing.clone()));
            listing
        }
    };
    if listing.omitted > 0 || listing.limited {
        let qualifier = if listing.limited { "at least " } else { "" };
        let limit = if listing.limited {
            " Listing limit reached."
        } else {
            ""
        };
        status(
            ui,
            canvas,
            &format!(
                "{} layers listed; {qualifier}{} more visible layers not listed.{limit} Zoom in to list more detail.",
                listing.items.len(),
                listing.omitted
            ),
            false,
        );
    }
    let items = &listing.items;
    let doc = &tab.doc;
    for item in items.iter() {
        let Some(node) = doc.node(item.guid) else {
            continue;
        };
        let id = node_id(tab.id, item.guid);
        let parent = item
            .parent
            .map_or(canvas.id, |i| node_id(tab.id, items[i].guid));
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
    let name: String = doc
        .node(guid)
        .map(display_name)
        .unwrap_or_default()
        .chars()
        .take(MAX_TEXT)
        .collect();
    let kind_name = kind.to_lowercase().replace('_', " ");
    let role = match kind {
        "TEXT" => Role::Label,
        "FRAME" | "GROUP" | "SECTION" | "SYMBOL" | "INSTANCE" | "COMPONENT_SET"
        | "BOOLEAN_OPERATION" => Role::Group,
        _ => Role::Image,
    };
    n.set_role(role);
    let kind_name = match kind {
        "SYMBOL" => "component".to_owned(),
        _ => kind_name,
    };
    // A layer named with spaces only still needs a name to be read and pressed by: its kind.
    n.set_label(if name.trim().is_empty() {
        kind_name.clone()
    } else {
        name.clone()
    });
    n.set_role_description(kind_name);
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
        let items = super::walk(&t, &super::reach(&t.doc, t.page), Pos2::ZERO, SCREEN).items;
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
        let items = super::walk(&t, &super::reach(&t.doc, t.page), Pos2::ZERO, SCREEN).items;
        let listed: Vec<Guid> = items.iter().map(|i| i.guid).collect();
        assert_eq!(listed, [Guid(9, 2), Guid(9, 3)]);
        let half = items[1].shown;
        assert_eq!((half.min.x, half.max.x), (200.0, 300.0), "{half:?}");
        assert_eq!(items[1].parent, Some(0));
    }

    /// The listing `super::layers` keeps in `ctx` after one frame of the canvas.
    fn listed_after_frame(ctx: &egui::Context, t: &mut Tab) -> std::sync::Arc<Vec<super::Item>> {
        for _ in 0..(super::MAX_REACH * 2 / super::REACH_STEPS + 2) {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let canvas = ui.allocate_rect(SCREEN, egui::Sense::click());
                super::layers(ui, &canvas, t, Pos2::ZERO);
            });
            out.textures_delta.clear();
            if let Some(listing) = ctx.data(|d| d.get_temp::<super::Listing>(super::listing_id())) {
                return listing.items;
            }
        }
        panic!("reach did not finish");
    }

    fn ready_reach(
        t: &mut Tab,
    ) -> std::sync::Arc<std::collections::HashMap<Guid, figstudio_render::kurbo::Rect>> {
        for _ in 0..(super::MAX_REACH * 2 / super::REACH_STEPS + 2) {
            if let Some((bounds, _)) = super::reach_of(t) {
                return bounds;
            }
        }
        panic!("reach did not finish");
    }

    #[test]
    fn the_walk_runs_again_only_when_the_view_changes() {
        let mut t = tab(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            node(2, 1, "FRAME", "Frame", 100.0, 200.0),
        ]);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let first = listed_after_frame(&ctx, &mut t);
        assert_eq!(first.len(), 1);
        let again = listed_after_frame(&ctx, &mut t);
        assert!(
            std::sync::Arc::ptr_eq(&first, &again),
            "walked again for the same view"
        );
        t.view.pan.x += 10.0;
        let moved = listed_after_frame(&ctx, &mut t);
        assert!(
            !std::sync::Arc::ptr_eq(&first, &moved),
            "kept a stale listing"
        );
        assert_eq!(moved[0].shown.min.x, first[0].shown.min.x + 10.0);
    }

    #[test]
    fn the_cap_does_not_hide_top_level_frames_behind_the_first_frames_children() {
        let mut nodes = vec![node(1, 0, "CANVAS", "page", 0.0, 0.0)];
        let mut frames = Vec::new();
        for i in 0..50 {
            let frame = 2 + i * 101;
            frames.push(Guid(9, frame));
            nodes.push(node(frame, 1, "FRAME", "Screen", 0.0, 200.0));
            for j in 1..=100 {
                nodes.push(node(frame + j, frame, "RECTANGLE", "Layer", 0.0, 200.0));
            }
        }
        let mut t = tab(nodes);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let items = listed_after_frame(&ctx, &mut t);
        assert_eq!(items.len(), super::MAX_NODES);
        let listing = ctx
            .data(|d| d.get_temp::<super::Listing>(super::listing_id()))
            .expect("listing");
        assert_eq!(listing.omitted, super::MAX_NODES);
        assert!(listing.limited);
        for frame in frames {
            assert!(items.iter().any(|i| i.guid == frame), "missing {frame:?}");
        }
    }

    #[test]
    fn reach_cache_is_not_shared_by_tabs_even_when_they_share_a_document() {
        let mut a = tab(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            node(2, 1, "FRAME", "Frame", 0.0, 200.0),
        ]);
        let mut b = tab(Vec::new());
        b.id = a.id + 1;
        b.doc = std::sync::Arc::clone(&a.doc);
        let first = ready_reach(&mut a);
        let second = ready_reach(&mut b);
        assert!(
            !std::sync::Arc::ptr_eq(&first, &second),
            "reach belongs to a tab, not an address"
        );
    }

    #[test]
    fn layer_names_are_bounded_like_text_values() {
        let name = "界".repeat(super::MAX_TEXT + 1);
        let t = tab(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            node(2, 1, "TEXT", &name, 0.0, 0.0),
        ]);
        let node = t.doc.node(Guid(9, 2)).expect("layer");
        let mut described = egui::accesskit::Node::new(egui::accesskit::Role::Label);
        super::describe(
            &mut described,
            &t.doc,
            node.id,
            &node.kind,
            &node.data,
            None,
        );
        assert_eq!(
            described.label().expect("name").chars().count(),
            super::MAX_TEXT
        );
    }

    #[test]
    fn a_layer_without_a_name_is_named_by_its_kind() {
        let t = tab(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            node(2, 1, "FRAME", " ", 0.0, 0.0),
        ]);
        let node = t.doc.node(Guid(9, 2)).expect("layer");
        let mut described = egui::accesskit::Node::new(egui::accesskit::Role::Group);
        super::describe(
            &mut described,
            &t.doc,
            node.id,
            &node.kind,
            &node.data,
            None,
        );
        assert_eq!(described.label(), Some("frame"));
    }

    #[test]
    fn reach_is_bounded_per_frame_and_resumes_until_ready() {
        let mut nodes = vec![node(1, 0, "CANVAS", "page", 0.0, 0.0)];
        for n in 2..(super::REACH_STEPS as u32 + 3) {
            nodes.push(node(n, 1, "RECTANGLE", "Layer", 100.0, 200.0));
        }
        let mut t = tab(nodes);
        assert!(
            super::reach_of(&mut t).is_none(),
            "a large page must not finish in one frame"
        );
        let pending = &t.a11y_reach.pages[&t.page];
        assert!(pending.seen.len() <= super::REACH_STEPS + 1);
        let ready = ready_reach(&mut t);
        let again = ready_reach(&mut t);
        assert!(std::sync::Arc::ptr_eq(&ready, &again));
    }

    #[test]
    fn listings_change_with_the_page_and_document_but_page_reach_is_retained() {
        let mut t = tab(vec![
            node(1, 0, "CANVAS", "First page", 0.0, 0.0),
            node(2, 1, "RECTANGLE", "First layer", 100.0, 200.0),
            node(3, 0, "CANVAS", "Second page", 0.0, 0.0),
            node(4, 3, "RECTANGLE", "Second layer", 300.0, 200.0),
        ]);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let first = listed_after_frame(&ctx, &mut t);
        let first_reach = ready_reach(&mut t);
        assert_eq!(first[0].guid, Guid(9, 2));
        t.page = Guid(9, 3);
        let second = listed_after_frame(&ctx, &mut t);
        assert_eq!(second[0].guid, Guid(9, 4));
        assert!(!std::sync::Arc::ptr_eq(&first, &second));
        t.page = Guid(9, 1);
        assert!(std::sync::Arc::ptr_eq(&first_reach, &ready_reach(&mut t)));
        let old_doc = std::sync::Arc::downgrade(&t.doc);
        t.doc = std::sync::Arc::new(document(vec![
            node(1, 0, "CANVAS", "Replacement page", 0.0, 0.0),
            node(2, 1, "RECTANGLE", "Replacement layer", 1200.0, 200.0),
        ]));
        t.view.pan.x = -800.0;
        let replacement = listed_after_frame(&ctx, &mut t);
        assert_eq!(replacement.len(), 1, "replacement uses its own reach");
        assert_eq!(replacement[0].guid, Guid(9, 2));
        assert_eq!(replacement[0].shown.min.x, 400.0);
        assert!(
            old_doc.upgrade().is_none(),
            "cache must not keep a closed document alive"
        );
        assert_eq!(t.a11y_reach.pages.len(), 1);
    }

    #[test]
    fn omission_counting_stops_after_one_additional_batch() {
        let mut nodes = vec![node(1, 0, "CANVAS", "page", 0.0, 0.0)];
        for n in 2..(super::MAX_VISITS as u32 + 3) {
            nodes.push(node(n, 1, "RECTANGLE", "Layer", 0.0, 200.0));
        }
        let t = tab(nodes);
        let listed = super::walk(&t, &std::collections::HashMap::new(), Pos2::ZERO, SCREEN);
        assert_eq!(listed.items.len(), super::MAX_NODES);
        assert_eq!(listed.omitted, super::MAX_NODES);
        assert!(listed.limited);
    }

    #[test]
    fn the_visit_limit_marks_unexamined_layers_even_if_none_were_listed() {
        let mut nodes = vec![node(1, 0, "CANVAS", "page", 0.0, 0.0)];
        for n in 2..(super::MAX_VISITS as u32 + 2) {
            nodes.push(node(n, 1, "RECTANGLE", "Off screen", -1000.0, 200.0));
        }
        let mut beyond = node(
            super::MAX_VISITS as u32 + 2,
            1,
            "RECTANGLE",
            "Beyond limit",
            0.0,
            200.0,
        );
        beyond.position = "zzzz".to_owned();
        nodes.push(beyond);
        let t = tab(nodes);
        let listed = super::walk(&t, &std::collections::HashMap::new(), Pos2::ZERO, SCREEN);
        assert!(listed.items.is_empty());
        assert_eq!(listed.omitted, 0);
        assert!(listed.limited);
    }

    #[test]
    fn the_tree_reports_omissions_and_pending_indexing() {
        fn frame_status(ctx: &egui::Context, t: &mut Tab) -> egui::accesskit::Node {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let canvas = ui.allocate_rect(SCREEN, egui::Sense::click());
                super::layers(ui, &canvas, t, Pos2::ZERO);
            });
            // Nothing draws here: drop the font atlas upload (debug builds assert otherwise).
            output.textures_delta.clear();
            output
                .platform_output
                .accesskit_update
                .expect("tree")
                .nodes
                .into_iter()
                .map(|(_, n)| n)
                .find(|n| n.author_id() == Some("canvas.layers.status"))
                .expect("listing status")
        }
        let nodes = |count: usize| {
            let mut nodes = vec![node(1, 0, "CANVAS", "page", 0.0, 0.0)];
            for n in 2..(count as u32 + 2) {
                nodes.push(node(n, 1, "RECTANGLE", "Layer", 0.0, 200.0));
            }
            nodes
        };
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut t = tab(nodes(super::MAX_NODES + 101));
        let omitted = frame_status(&ctx, &mut t);
        assert!(
            omitted
                .value()
                .expect("count")
                .contains("101 more visible layers not listed")
        );
        t.doc = std::sync::Arc::new(document(nodes(super::REACH_STEPS + 1)));
        let pending = frame_status(&ctx, &mut t);
        assert_eq!(pending.role(), egui::accesskit::Role::ProgressIndicator);
        assert_eq!(pending.label(), Some("Preparing canvas layers…"));
        assert!(
            ctx.data(|d| d.get_temp::<super::Listing>(super::listing_id()))
                .is_none(),
            "no stale listing while a replacement is indexing"
        );
    }

    #[test]
    fn without_accesskit_nothing_is_walked() {
        let mut t = tab(vec![
            node(1, 0, "CANVAS", "page", 0.0, 0.0),
            node(2, 1, "FRAME", "Frame", 100.0, 200.0),
        ]);
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let canvas = ui.allocate_rect(SCREEN, egui::Sense::click());
            super::layers(ui, &canvas, &mut t, Pos2::ZERO);
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
        let mut indexing = super::Reach::new(t.page);
        let mut chunks = Vec::new();
        while !indexing.stack.is_empty() {
            let chunk = std::time::Instant::now();
            indexing.advance(&t.doc, super::REACH_STEPS);
            chunks.push(chunk.elapsed());
        }
        let elapsed = started.elapsed();
        chunks.sort();
        let reach = indexing.bounds;
        println!(
            "reach of {} layers: {elapsed:?}, {} UI frames, {} steps/frame; median {:?}, max {:?}",
            reach.len(),
            chunks.len(),
            super::REACH_STEPS,
            chunks[chunks.len() / 2],
            chunks[chunks.len() - 1]
        );
        for (what, zoom) in [("fit", 1.0), ("4x fit", 4.0), ("40x fit", 40.0)] {
            t.view = fit;
            t.view.zoom = fit.zoom * zoom;
            let mut times = Vec::new();
            let mut listed = 0;
            for _ in 0..20 {
                let started = std::time::Instant::now();
                listed = super::walk(&t, &reach, Pos2::ZERO, screen).items.len();
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
        assert!(
            super::walk(&t, &super::reach(&t.doc, t.page), Pos2::ZERO, SCREEN)
                .items
                .is_empty()
        );
    }
}
