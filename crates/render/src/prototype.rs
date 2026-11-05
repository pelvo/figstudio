//! Prototype flows and links read from a document, and a player that follows them. Shared by
//! the app's Preview, the CLI `prototype` command and the MCP `get_prototype` tool.
//!
//! Played: "On click" links that navigate to a frame or go back. Every other interaction
//! (hover, after delay, overlays, swap state, scroll to, URLs, variables) is listed with
//! [`Link::step`] `None`, so each surface can count it instead of hiding it.
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;

use figstudio_fig::resolve::{self, InstancePath};
use figstudio_fig::{Document, Guid, Node, Val};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Affine, Point, Rect};
use vello_cpu::{RenderContext, RenderSettings};

use crate::geom::size;
use crate::{
    Ctx, Inspect, MAX_RENDER_WORK, Renderer, absolute_transform, node_bounds, paint,
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

/// The frames Present steps through on `page`, as Figma does: visible frames that are not
/// component sets or groups, at the top level or inside sections. Figma sorts each level
/// (a section is one item with its own box) by "left first when they overlap vertically,
/// else top first", with a stable insertion sort from layer order. That comparison is not
/// transitive, so the sort itself is part of the order; matched against Figma on real files.
pub fn frames(doc: &Document, page: Guid) -> Vec<Guid> {
    let mut found = Vec::new();
    in_order(doc, page, &mut found, 0);
    found
}

/// Appends the frames under `parent` in Figma's Present order (see [`frames`]).
fn in_order(doc: &Document, parent: Guid, found: &mut Vec<Guid>, depth: usize) {
    if depth > 64 {
        return;
    }
    let mut items: Vec<(Rect, Guid, bool)> = doc
        .children(parent)
        .iter()
        .filter_map(|id| {
            let node = doc.node(*id)?;
            if node.data.flag("visible") == Some(false) {
                return None;
            }
            let section = match node.kind.as_str() {
                "SECTION" => true,
                // A group is stored as a frame that resizes to fit; component sets are frames too.
                "FRAME"
                    if node.data.flag("isStateGroup") != Some(true)
                        && node.data.flag("resizeToFit") != Some(true) =>
                {
                    false
                }
                _ => return None,
            };
            Some((node_bounds(doc, *id)?, *id, section))
        })
        .collect();
    let before = |a: &Rect, b: &Rect| {
        // Boxes that only touch count as overlapping too.
        let (major, minor) = if a.y0 <= b.y1 && b.y0 <= a.y1 {
            (a.x0.total_cmp(&b.x0), a.y0.total_cmp(&b.y0))
        } else {
            (a.y0.total_cmp(&b.y0), a.x0.total_cmp(&b.x0))
        };
        major.then(minor).is_lt()
    };
    for i in 1..items.len() {
        let mut j = i;
        while j > 0 && before(&items[j].0, &items[j - 1].0) {
            items.swap(j, j - 1);
            j -= 1;
        }
    }
    for (_, id, section) in items {
        if section {
            in_order(doc, id, found, depth + 1);
        } else {
            found.push(id);
        }
    }
}

/// Whether `id` and every ancestor up to its page are visible.
fn shown(doc: &Document, id: Guid) -> bool {
    let mut seen = HashSet::new();
    let mut cur = doc.node(id);
    while let Some(node) = cur {
        if !seen.insert(node.id) || node.data.flag("visible") == Some(false) {
            return false;
        }
        if node.kind == "CANVAS" {
            return true;
        }
        cur = node.parent.and_then(|p| doc.node(p));
    }
    false
}

/// Where Preview starts on `page`, as in Figma: the screen holding `selection`, else the
/// page's first flow, else the page's first frame in [`frames`] order.
pub fn start(doc: &Document, page: Guid, selection: Option<Guid>) -> Option<Guid> {
    let flows = flows(doc);
    selection
        .and_then(|id| screen_of(doc, id))
        .or_else(|| {
            flows
                .iter()
                .find(|f| f.page == page && shown(doc, f.start))
                .map(|f| f.start)
        })
        .or_else(|| frames(doc, page).first().copied())
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

/// The colour behind a screen: the page's prototype background, else black, as in Figma.
pub fn background(doc: &Document, page: Guid) -> AlphaColor<Srgb> {
    doc.node(page)
        .and_then(|p| paint::color(p.data.get("prototypeBackgroundColor"), 1.0))
        // Figma's Present shows black where the page sets no prototype background.
        .unwrap_or(AlphaColor::new([0.0, 0.0, 0.0, 1.0]))
}

/// A layer: a document node or a node inside an instance.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    Node(Guid),
    Path(InstancePath),
}

impl Layer {
    /// The layer as a guid path: the document node, or the instance and its override keys.
    pub fn key(&self) -> Vec<Guid> {
        match self {
            Layer::Node(id) => vec![*id],
            Layer::Path(path) => std::iter::once(path.instance())
                .chain(path.keys().iter().copied())
                .collect(),
        }
    }
}

/// What a running prototype has changed that the drawing must show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// Instances showing another variant, by [`Layer::key`]: the component they show instead.
    pub swaps: HashMap<Vec<Guid>, Guid>,
}

/// Fields of an instance that are not its look: never taken from a swapped-in variant.
const KEEP_ON_SWAP: &[&str] = &[
    "guid",
    "parentIndex",
    "name",
    "type",
    "transform",
    "symbolData",
    "overriddenSymbolID",
    "overrideKey",
    "componentPropAssignments",
    "componentPropDefs",
    "isStateGroup",
    "isPublishable",
    "variantPropSpecs",
    "phase",
    "editInfo",
    "key",
    "version",
    "userFacingVersion",
    "publishedVersion",
    "sharedSymbolReference",
    "visible",
    "locked",
];

impl Ctx<'_> {
    /// `data` of instance `node` showing the variant the prototype swapped it to, if any. As
    /// in Figma, the new variant's own look (fills, size, layout…) replaces the old variant's
    /// wherever the instance did not override it; overrides stay.
    pub(crate) fn swapped_variant(
        &self,
        node: &Node,
        path: Option<&[Guid]>,
        data: &Val,
    ) -> Option<Val> {
        if node.kind != "INSTANCE" || self.prototype.swaps.is_empty() {
            return None;
        }
        let key: Vec<Guid> = match path {
            None => vec![node.id],
            Some(keys) => std::iter::once(self.outer?)
                .chain(keys.iter().copied())
                .collect(),
        };
        let to = *self.prototype.swaps.get(&key)?;
        let new = self.doc.node(to)?;
        let old = resolve::symbol_id(data).and_then(|id| self.doc.node(id));
        let mut out = data.clone();
        if let (Some(old), Val::Obj(new_fields), Val::Obj(fields)) = (old, &new.data, &mut out) {
            let inherited = |k: &str| data.get(k) == old.data.get(k);
            for (k, v) in new_fields.iter() {
                if !KEEP_ON_SWAP.contains(&k) && inherited(k) {
                    fields.insert(k, v.clone());
                }
            }
            let gone: Vec<&'static str> = fields
                .keys()
                .filter(|k| !KEEP_ON_SWAP.contains(k) && new.data.get(k).is_none() && inherited(k))
                .collect();
            for k in gone {
                fields.remove(k);
            }
        }
        out.set("overriddenSymbolID", to.to_val());
        Some(out)
    }
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

/// A layer's box placed on the page: `transform` maps `0..w × 0..h` to page space, so a
/// rotated or skewed layer keeps its shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub transform: Affine,
    pub width: f64,
    pub height: f64,
}

impl Area {
    pub fn contains(&self, p: Point) -> bool {
        if self.transform.determinant().abs() < 1e-12 {
            return false;
        }
        let local = self.transform.inverse() * p;
        (0.0..=self.width).contains(&local.x) && (0.0..=self.height).contains(&local.y)
    }

    /// The four corners in page space, clockwise from the layer's top left.
    pub fn corners(&self) -> [Point; 4] {
        let (w, h) = (self.width, self.height);
        [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| self.transform * Point::new(x, y))
    }

    fn bounds(&self) -> Rect {
        transformed_bounds(self.transform, Rect::new(0.0, 0.0, self.width, self.height))
    }
}

/// One action of one interaction on a layer.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub layer: Layer,
    pub name: String,
    /// The layer's page-space bounding box.
    pub bounds: Rect,
    /// The layer's shape, for hit tests and outlines.
    pub area: Area,
    /// The clipping frames around the layer: a point outside any of them does not hit it.
    pub clips: Vec<Area>,
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
            screen.push(doc, found);
            if screen.truncated {
                break;
            }
        }
        screen
    }

    fn push(&mut self, doc: &Document, found: Found) {
        let Found {
            layer,
            area,
            clips,
            data,
        } = found;
        let data = &data;
        let bounds = area.bounds();
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
                if self.links.len() >= MAX_LINKS {
                    self.truncated = true;
                    return;
                }
                let step = match (trigger, kind) {
                    ("ON_CLICK", "NAVIGATE") => {
                        destination.filter(|d| shown(doc, *d)).map(Step::Navigate)
                    }
                    ("ON_CLICK", "BACK") => Some(Step::Back),
                    _ => None,
                };
                self.links.push(Link {
                    layer: layer.clone(),
                    name: name.to_owned(),
                    bounds,
                    area,
                    clips: clips.clone(),
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

    /// What a click at page point `p` does: on the front-most layer with an "On click" link
    /// there, its first played action, else its first action (not played).
    pub fn hit(&self, p: Point) -> Option<&Link> {
        let clicks = |l: &&Link| l.trigger == "ON_CLICK";
        let front = self
            .links
            .iter()
            .rev()
            .filter(clicks)
            .find(|l| l.area.contains(p) && l.clips.iter().all(|c| c.contains(p)))?;
        let mut on_layer = self
            .links
            .iter()
            .filter(clicks)
            .filter(|l| l.layer == front.layer);
        on_layer
            .clone()
            .find(|l| l.step.is_some())
            .or_else(|| on_layer.next())
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
    area: Area,
    clips: Vec<Area>,
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
        let (width, height) = size(data);
        let area = Area {
            transform: world,
            width,
            height,
        };
        // A layer wholly outside a clipping frame is not drawn, so it has no link to click.
        let bounds = area.bounds();
        if self
            .clips
            .iter()
            .any(|c| c.bounds().intersect(bounds).area() <= 0.0 && !bounds.is_zero_area())
        {
            return;
        }
        self.found.push(Found {
            layer,
            area,
            clips: self.clips.clone(),
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
