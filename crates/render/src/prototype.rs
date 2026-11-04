//! Prototype flows and links read from a document, and a player that follows them. Shared by
//! the app's Preview, the CLI `prototype` command and the MCP `get_prototype` tool.
//!
//! Played: "On click" links that navigate to a frame or go back. Every other interaction
//! (hover, after delay, overlays, swap state, scroll to, URLs, variables) is listed with
//! [`Link::step`] `None`, so each surface can count it instead of hiding it.
use std::collections::{HashSet, VecDeque};
use std::fmt;

use figstudio_fig::resolve::{self, InstancePath};
use figstudio_fig::{Document, Guid, Node, Val};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Affine, Point, Rect};
use vello_cpu::{RenderContext, RenderSettings};

use crate::geom::size;
use crate::{
    Ctx, Inspect, MAX_RENDER_WORK, Renderer, absolute_transform, page_background, paint,
    transformed_bounds,
};

/// Most links listed for one screen; beyond it [`Screen::truncated`] is set.
pub const MAX_LINKS: usize = 5_000;

/// A flow starting point: a named entry into the prototype.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    pub name: String,
    pub description: String,
    pub page: Guid,
    /// The frame the flow starts on.
    pub start: Guid,
}

/// The flows of the visible pages, in page order and then in Figma's flow order.
pub fn flows(doc: &Document) -> Vec<Flow> {
    let mut found: Vec<(usize, &str, Flow)> = doc
        .nodes
        .iter()
        .filter_map(|node| {
            let start = node.data.get("prototypeStartingPoint")?;
            let page = doc.page_of(node.id)?;
            let index = doc.pages.iter().position(|p| *p == page)?;
            Some((
                index,
                start.text("position").unwrap_or(""),
                Flow {
                    name: start.text("name").unwrap_or("").to_owned(),
                    description: start.text("description").unwrap_or("").to_owned(),
                    page,
                    start: node.id,
                },
            ))
        })
        .collect();
    found.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    found.into_iter().map(|(_, _, flow)| flow).collect()
}

/// Where Preview starts on `page`: the screen holding `selection`, else the page's first flow,
/// else the first flow of any page, else the page's first frame.
pub fn start(doc: &Document, page: Guid, selection: Option<Guid>) -> Option<Guid> {
    let flows = flows(doc);
    selection
        .and_then(|id| screen_of(doc, id))
        .or_else(|| flows.iter().find(|f| f.page == page).map(|f| f.start))
        .or_else(|| flows.first().map(|f| f.start))
        .or_else(|| {
            doc.children(page)
                .iter()
                .copied()
                .find(|id| doc.node(*id).is_some_and(|n| n.kind == "FRAME"))
        })
}

/// The top-level frame (a child of the page or of a section) that holds `id`.
pub fn screen_of(doc: &Document, id: Guid) -> Option<Guid> {
    let mut seen = HashSet::new();
    let mut cur = doc.node(id)?;
    loop {
        if !seen.insert(cur.id) {
            return None;
        }
        let parent = doc.node(cur.parent?)?;
        if matches!(parent.kind.as_str(), "CANVAS" | "SECTION") {
            return matches!(cur.kind.as_str(), "FRAME" | "INSTANCE" | "SYMBOL").then_some(cur.id);
        }
        cur = parent;
    }
}

/// The colour behind a screen: the page's prototype background, else its canvas colour.
pub fn background(doc: &Document, page: Guid) -> AlphaColor<Srgb> {
    doc.node(page)
        .and_then(|p| paint::color(p.data.get("prototypeBackgroundColor"), 1.0))
        .unwrap_or_else(|| page_background(doc, page))
}

/// A layer: a document node or a node inside an instance.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    Node(Guid),
    Path(InstancePath),
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Layer::Node(id) => id.fmt(f),
            Layer::Path(path) => path.fmt(f),
        }
    }
}

/// What a played link does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Navigate(Guid),
    Back,
}

/// One action of one interaction on a layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub layer: Layer,
    pub name: String,
    /// The layer's page-space box.
    pub bounds: Rect,
    /// Figma's interaction type, such as `ON_CLICK`, `ON_HOVER` or `AFTER_TIMEOUT`.
    pub trigger: &'static str,
    /// For a link to a layer, Figma's navigation type (`NAVIGATE`, `OVERLAY`, `SWAP_STATE`,
    /// `SCROLL_TO`, …); otherwise its connection type (`BACK`, `CLOSE`, `URL`, …).
    pub action: &'static str,
    /// The layer the link goes to, as stored (it may be missing from this file).
    pub destination: Option<Guid>,
    pub url: Option<String>,
    /// Figma's transition type, such as `INSTANT_TRANSITION` or `SMART_ANIMATE`.
    pub transition: &'static str,
    /// What the player does on it; `None` when the link is not played.
    pub step: Option<Step>,
}

/// One frame of the prototype and its links in paint order (back to front).
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub frame: Guid,
    pub links: Vec<Link>,
    /// More links or instance nodes than the limits; the list is incomplete.
    pub truncated: bool,
}

impl Screen {
    /// The links of `frame`, inside instances too, found in one walk the way the renderer
    /// draws it (overrides applied, hidden layers left out).
    pub fn read(renderer: &mut Renderer, doc: &Document, frame: Guid) -> Screen {
        let mut canvas = RenderContext::new_with(1, 1, RenderSettings::default());
        let mut ctx = renderer.context(doc, &mut canvas, Rect::ZERO, MAX_RENDER_WORK, 0);
        ctx.inspect = Some(Inspect::Links);
        if let Some(node) = doc.node(frame) {
            let parent = node
                .parent
                .map_or(Affine::IDENTITY, |p| absolute_transform(doc, p));
            if ctx.charge_work() {
                ctx.draw(node, parent, None);
            }
        }
        let mut screen = Screen {
            frame,
            links: Vec::new(),
            truncated: ctx.exhausted || ctx.stats.depth_skipped > 0,
        };
        for found in std::mem::take(&mut ctx.found) {
            if screen.links.len() >= MAX_LINKS {
                screen.truncated = true;
                break;
            }
            screen.push(doc, found.layer, &found.data, found.bounds);
        }
        screen.links.truncate(MAX_LINKS);
        screen
    }

    fn push(&mut self, doc: &Document, layer: Layer, data: &Val, bounds: Rect) {
        let name = data.text("name").unwrap_or("");
        for interaction in interactions(doc, data) {
            if interaction.flag("isDeleted") == Some(true) {
                continue;
            }
            let trigger =
                name_of(interaction.path(&["event", "interactionType"])).unwrap_or("NONE");
            for action in interaction.get("actions").map_or(&[][..], Val::items) {
                let destination = action.get("transitionNodeID").and_then(Val::as_guid);
                let connection =
                    name_of(action.get("connectionType")).unwrap_or(if destination.is_some() {
                        "INTERNAL_NODE"
                    } else {
                        "NONE"
                    });
                let kind = if connection == "INTERNAL_NODE" {
                    name_of(action.get("navigationType")).unwrap_or("NAVIGATE")
                } else {
                    connection
                };
                if trigger == "NONE" || kind == "NONE" {
                    continue;
                }
                let step = match (trigger, kind) {
                    ("ON_CLICK", "NAVIGATE") => destination
                        .filter(|d| doc.node(*d).is_some())
                        .map(Step::Navigate),
                    ("ON_CLICK", "BACK") => Some(Step::Back),
                    _ => None,
                };
                self.links.push(Link {
                    layer: layer.clone(),
                    name: name.to_owned(),
                    bounds,
                    trigger,
                    action: kind,
                    destination,
                    url: action.text("connectionURL").map(str::to_owned),
                    transition: name_of(action.get("transitionType"))
                        .unwrap_or("INSTANT_TRANSITION"),
                    step,
                });
            }
        }
    }

    /// The front-most "On click" link under page point `p`, played or not.
    pub fn hit(&self, p: Point) -> Option<&Link> {
        self.links
            .iter()
            .rev()
            .find(|l| l.trigger == "ON_CLICK" && l.bounds.contains(p))
    }

    /// Links the player does not play.
    pub fn skipped(&self) -> usize {
        self.links.iter().filter(|l| l.step.is_none()).count()
    }
}

/// The screens reachable from `starts` by played links, breadth first, at most `max`.
/// Returns them and whether `max` cut the list short.
pub fn reachable(
    renderer: &mut Renderer,
    doc: &Document,
    starts: &[Guid],
    max: usize,
) -> (Vec<Screen>, bool) {
    let mut seen: HashSet<Guid> = HashSet::new();
    let mut queue: VecDeque<Guid> = starts.iter().copied().filter(|s| seen.insert(*s)).collect();
    let mut screens = Vec::new();
    while let Some(frame) = queue.pop_front() {
        if screens.len() >= max {
            return (screens, true);
        }
        let screen = Screen::read(renderer, doc, frame);
        for link in &screen.links {
            if let Some(Step::Navigate(to)) = link.step
                && seen.insert(to)
            {
                queue.push_back(to);
            }
        }
        screens.push(screen);
    }
    (screens, false)
}

/// Follows played links: the screens visited, newest last; never empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    history: Vec<Guid>,
}

impl Player {
    pub fn new(start: Guid) -> Player {
        Player {
            history: vec![start],
        }
    }

    /// The screen shown.
    pub fn screen(&self) -> Guid {
        self.history[self.history.len() - 1]
    }

    /// Whether [`Step::Back`] has somewhere to go.
    pub fn can_go_back(&self) -> bool {
        self.history.len() > 1
    }

    /// Follows `step`; returns whether the screen changed.
    pub fn go(&mut self, step: Step) -> bool {
        match step {
            Step::Navigate(to) if to != self.screen() => {
                self.history.push(to);
                true
            }
            Step::Back if self.can_go_back() => {
                self.history.pop();
                true
            }
            _ => false,
        }
    }

    /// Back to the first screen; returns whether the screen changed.
    pub fn restart(&mut self) -> bool {
        let moved = self.can_go_back();
        self.history.truncate(1);
        moved
    }
}

/// A layer with interactions, as an [`Inspect::Links`] walk found it.
pub(crate) struct Found {
    layer: Layer,
    bounds: Rect,
    data: Val,
}

impl Ctx<'_> {
    /// Records `node` when it has interactions; `path` is its override path inside
    /// `self.outer`, `None` for a document node.
    pub(crate) fn found_link(
        &mut self,
        node: &Node,
        path: Option<&[Guid]>,
        world: Affine,
        data: &Val,
    ) {
        if interactions(self.doc, data).is_empty() {
            return;
        }
        let layer = match (path, self.outer) {
            (None, _) => Layer::Node(node.id),
            (Some(keys), Some(outer)) => {
                match InstancePath::new(
                    std::iter::once(outer).chain(keys.iter().copied()).collect(),
                ) {
                    Some(path) => Layer::Path(path),
                    None => return,
                }
            }
            (Some(_), None) => return,
        };
        let (w, h) = size(data);
        self.found.push(Found {
            layer,
            bounds: transformed_bounds(world, Rect::new(0.0, 0.0, w, h)),
            data: data.clone(),
        });
    }
}

/// The interactions of a layer; an instance without its own uses its component's.
fn interactions<'a>(doc: &'a Document, data: &'a Val) -> &'a [Val] {
    if let Some(own) = data.get("prototypeInteractions") {
        return own.items();
    }
    if data.text("type") == Some("INSTANCE")
        && let Some(component) = resolve::symbol_id(data).and_then(|id| doc.node(id))
    {
        return component
            .data
            .get("prototypeInteractions")
            .map_or(&[], Val::items);
    }
    &[]
}

/// A stored enum value's name.
fn name_of(value: Option<&Val>) -> Option<&'static str> {
    match value? {
        Val::Enum(name) => Some(name),
        _ => None,
    }
}
