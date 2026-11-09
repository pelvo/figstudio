//! Prototype flows and links read from a document, and a player that follows them. Shared by
//! the app's Preview, the CLI `prototype` command and the MCP `get_prototype` tool.
//!
//! Played: "On click" links that navigate to a frame or go back. Every other interaction
//! (hover, after delay, overlays, swap state, scroll to, URLs, variables) is listed with
//! [`Link::step`] `None`, so each surface can count it instead of hiding it.
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::Arc;

use figstudio_fig::resolve::{self, InstancePath};
use figstudio_fig::{Document, Guid, Node, Val};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::kurbo::{Affine, Point, Rect, Vec2};
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
    /// Scrolled frames, by [`Layer::key`]: how far their content is scrolled, in whole
    /// units of the frame (whole units keep states comparable and their tiles reusable).
    pub scrolls: HashMap<Vec<Guid>, (i32, i32)>,
}

/// A frame that scrolls its content in Preview: Figma's overflow scrolling on a frame that
/// clips its content.
#[derive(Clone, Debug, PartialEq)]
pub struct Scroller {
    pub layer: Layer,
    /// The frame's box: the viewport the content scrolls in.
    pub area: Area,
    /// The clipping frames around it.
    pub clips: Vec<Area>,
    /// How far the content reaches, in the frame's units from its top left.
    pub content: (f64, f64),
    /// Scrolls across, down.
    pub axes: (bool, bool),
}

impl Scroller {
    /// The largest offset the content allows on each axis.
    pub fn max(&self) -> (i32, i32) {
        let reach = |on: bool, content: f64, view: f64| {
            if on {
                (content - view).max(0.0).floor() as i32
            } else {
                0
            }
        };
        (
            reach(self.axes.0, self.content.0, self.area.width),
            reach(self.axes.1, self.content.1, self.area.height),
        )
    }
}

/// The transform scrolling adds to a child of a frame scrolled by `offset`: by the offset,
/// none for a child fixed in place, and for a sticky child only until it reaches the frame's
/// top or left edge, where it stays (Figma's scroll behaviours).
pub(crate) fn scrolled(child: &Val, (dx, dy): (f64, f64)) -> Affine {
    match child.text("scrollBehavior") {
        Some("FIXED_WHEN_CHILD_OF_SCROLLING_FRAME") => Affine::IDENTITY,
        Some("STICKY_SCROLLS") => {
            let (x, y) = (
                child
                    .path(&["transform", "m02"])
                    .and_then(Val::as_f32)
                    .map_or(0.0, f64::from),
                child
                    .path(&["transform", "m12"])
                    .and_then(Val::as_f32)
                    .map_or(0.0, f64::from),
            );
            Affine::translate((-dx.min(x.max(0.0)), -dy.min(y.max(0.0))))
        }
        _ => Affine::translate((-dx, -dy)),
    }
}

/// Which axes a frame scrolls, from Figma's `scrollDirection`; `None` if it does not, or
/// does not clip its content (Figma scrolls only clipped frames).
fn scroll_axes(kind: &str, data: &Val) -> Option<(bool, bool)> {
    if !crate::geom::clips_content(kind, data) {
        return None;
    }
    match data.text("scrollDirection")? {
        "HORIZONTAL" => Some((true, false)),
        "VERTICAL" => Some((false, true)),
        "BOTH" => Some((true, true)),
        _ => None,
    }
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
    /// The layer key of `node` drawn at override `path`: the node, or the outer instance and
    /// the path's keys.
    fn layer_key(&self, node: Guid, path: Option<&[Guid]>) -> Option<Vec<Guid>> {
        match path {
            None => Some(vec![node]),
            Some(keys) => Some(
                std::iter::once(self.outer?)
                    .chain(keys.iter().copied())
                    .collect(),
            ),
        }
    }

    /// How far the prototype state scrolls the content of frame `node`, if at all.
    pub(crate) fn scroll_of(
        &self,
        node: Guid,
        path: Option<&[Guid]>,
        data: &Val,
    ) -> Option<(f64, f64)> {
        if self.prototype.scrolls.is_empty() {
            return None;
        }
        let kind = data.text("type").unwrap_or("");
        scroll_axes(kind, data)?;
        let (x, y) = *self.prototype.scrolls.get(&self.layer_key(node, path)?)?;
        Some((f64::from(x), f64::from(y)))
    }

    /// Records `node` when it scrolls its content (Links walks).
    pub(crate) fn found_scroller(
        &mut self,
        node: &Node,
        path: Option<&[Guid]>,
        world: Affine,
        data: &Val,
    ) {
        let Some(axes) = scroll_axes(&node.kind, data) else {
            return;
        };
        let Some(key) = self.layer_key(node.id, path) else {
            return;
        };
        let layer = match path {
            None => Layer::Node(node.id),
            Some(_) => match InstancePath::new(key) {
                Some(p) => Layer::Path(p),
                None => return,
            },
        };
        // The content's reach, from the children as stored (in the frame's units); for an
        // instance, its component's children.
        let parent = if node.kind == "INSTANCE" {
            resolve::symbol_id(data).unwrap_or(node.id)
        } else {
            node.id
        };
        let mut content = (0.0f64, 0.0f64);
        for child in self
            .doc
            .children(parent)
            .iter()
            .filter_map(|c| self.doc.node(*c))
        {
            if child.data.flag("visible") == Some(false) {
                continue;
            }
            let (w, h) = size(&child.data);
            let b = transformed_bounds(
                crate::geom::affine(child.data.get("transform")),
                Rect::new(0.0, 0.0, w, h),
            );
            content = (content.0.max(b.x1), content.1.max(b.y1));
        }
        let (width, height) = size(data);
        self.scrollers.push(Scroller {
            layer,
            area: Area {
                transform: world,
                width,
                height,
            },
            clips: self.clips.clone(),
            content,
            axes,
        });
    }

    /// `data` of instance `node` showing the variant the prototype swapped it to, if any. As
    /// in Figma, the new variant's own look (fills, size, layout…) replaces the old variant's
    /// wherever the instance did not override it; overrides stay.
    pub(crate) fn swapped_variant(
        &self,
        node: &Node,
        path: Option<&[Guid]>,
        data: &Val,
    ) -> Option<(Val, HashMap<Guid, Guid>)> {
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
        let mut remap = HashMap::new();
        if let Some(old) = old {
            match_layers(self.doc, new.id, old.id, &mut remap, 0);
        }
        Some((out, remap))
    }
}

/// Maps the layer keys under variant `new` to those of the matching layers under `old`, as
/// Figma keeps overrides across a variant swap: by name and type, in order. Nested instances
/// are matched but not entered (their layers belong to their own component).
fn match_layers(doc: &Document, new: Guid, old: Guid, map: &mut HashMap<Guid, Guid>, depth: usize) {
    if depth > 32 {
        return;
    }
    let mut free: Vec<&Node> = doc
        .children(old)
        .iter()
        .filter_map(|id| doc.node(*id))
        .collect();
    for child in doc.children(new).iter().filter_map(|id| doc.node(*id)) {
        let Some(at) = free
            .iter()
            .position(|o| o.name == child.name && o.kind == child.kind)
        else {
            continue;
        };
        let other = free.remove(at);
        map.insert(child.override_key(), other.override_key());
        if child.kind != "INSTANCE" {
            match_layers(doc, child.id, other.id, map, depth + 1);
        }
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Navigate(Guid),
    Back,
    /// Show variant `to` in place of instance `target` (Figma's "change to").
    Swap {
        target: Layer,
        to: Guid,
    },
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
    /// Seconds before an `AFTER_TIMEOUT` link fires.
    pub delay: f64,
}

/// One frame of the prototype and its links in paint order (back to front).
#[derive(Clone, Debug, PartialEq)]
pub struct Screen {
    pub frame: Guid,
    pub links: Vec<Link>,
    /// More links or instance nodes than the limits; the list is incomplete.
    pub truncated: bool,
    /// The frames that scroll their content, back to front.
    pub scrollers: Vec<Scroller>,
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
            scrollers: std::mem::take(&mut ctx.scrollers),
        };
        let mut variants: HashMap<Vec<Guid>, Option<Guid>> = std::mem::take(&mut ctx.variants)
            .into_iter()
            .map(|(k, v)| (k, Some(v)))
            .collect();
        for found in std::mem::take(&mut ctx.found) {
            screen.push(doc, &ctx.prototype, &ctx.definitions, &mut variants, found);
            if screen.truncated {
                break;
            }
        }
        screen
    }

    fn push(
        &mut self,
        doc: &Document,
        state: &State,
        definitions: &Arc<resolve::Definitions>,
        variants: &mut HashMap<Vec<Guid>, Option<Guid>>,
        found: Found,
    ) {
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
                let step = if !TRIGGERS.contains(&trigger) {
                    None
                } else {
                    match kind {
                        "NAVIGATE" => destination.filter(|d| shown(doc, *d)).map(Step::Navigate),
                        "BACK" => Some(Step::Back),
                        "SWAP_STATE" => destination.and_then(|to| {
                            let (target, to) =
                                swap_target(doc, state, definitions, variants, &layer, to)?;
                            Some(Step::Swap { target, to })
                        }),
                        _ => None,
                    }
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
                    delay: interaction
                        .path(&["event", "transitionTimeout"])
                        .and_then(Val::as_f32)
                        .map_or(0.8, f64::from),
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

/// A frame a scroll goes to: the frame, its offset now and its largest offset.
type ScrollTarget<'s> = (&'s Scroller, (i32, i32), (i32, i32));

/// A pointer event in page space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointer {
    Move(Point),
    Down(Point),
    Up(Point),
    /// The pointer left the screen.
    Leave,
    /// A scroll by `delta` page units (positive: towards the content's end) over a point.
    Scroll(Point, Vec2),
}

/// Runs a prototype: the screens visited (newest last, never empty), the variants instances
/// show, and what the pointer is over or pressing. Time is in seconds, from the caller.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    history: Vec<Guid>,
    state: State,
    /// Layers with hover links under the pointer, with their shape and clips when entered:
    /// "while hovering" lasts until the pointer leaves that shape, whatever the swapped-in
    /// variant's own links are.
    hovered: Vec<(Layer, Area, Vec<Area>)>,
    /// The layer pressed, if the press started on one with click or press links.
    pressed: Option<Layer>,
    /// Temporary swaps ("while hovering", "while pressing"), by target key and trigger: the
    /// variant to go back to (`None`: the instance's own).
    reverts: Vec<(Vec<Guid>, &'static str, Option<Guid>)>,
    /// The unscrolled fraction of a unit left by the last scroll, and the frame it was for.
    scroll_rest: Option<(Vec<Guid>, Vec2)>,
    /// When the screen or the state last changed: "after delay" counts from here.
    since: f64,
    now: f64,
}

const HOVER: &[&str] = &[
    "ON_HOVER",
    "MOUSE_ENTER",
    "MOUSE_LEAVE",
    "MOUSE_IN",
    "MOUSE_OUT",
];
const PRESS: &[&str] = &["ON_CLICK", "ON_PRESS", "MOUSE_DOWN", "MOUSE_UP", "DRAG"];

impl Player {
    pub fn new(start: Guid) -> Player {
        Player {
            history: vec![start],
            state: State::default(),
            hovered: Vec::new(),
            pressed: None,
            reverts: Vec::new(),
            scroll_rest: None,
            since: 0.0,
            now: 0.0,
        }
    }

    /// The screen shown.
    pub fn screen(&self) -> Guid {
        self.history[self.history.len() - 1]
    }

    /// The variants instances show now.
    pub fn state(&self) -> &State {
        &self.state
    }

    /// Whether [`Step::Back`] has somewhere to go.
    pub fn can_go_back(&self) -> bool {
        self.history.len() > 1
    }

    /// Follows `step`; returns whether the screen or the state changed.
    pub fn go(&mut self, step: Step) -> bool {
        let changed = match step {
            Step::Navigate(to) if to != self.screen() => {
                self.history.push(to);
                self.left_screen();
                true
            }
            Step::Back if self.can_go_back() => {
                self.history.pop();
                self.left_screen();
                true
            }
            Step::Swap { target, to } => self.state.swaps.insert(target.key(), to) != Some(to),
            _ => false,
        };
        if changed {
            self.since = self.now;
        }
        changed
    }

    /// Back to the first screen and the document's own variants; returns whether anything
    /// changed.
    pub fn restart(&mut self) -> bool {
        let moved = self.can_go_back() || !self.state.swaps.is_empty();
        self.history.truncate(1);
        self.state = State::default();
        self.left_screen();
        self.since = self.now;
        moved
    }

    fn left_screen(&mut self) {
        // A new screen starts scrolled to the top, as Figma's do by default.
        self.state.scrolls.clear();
        self.hovered.clear();
        self.pressed = None;
        self.reverts.clear();
    }

    /// Acts on a pointer event over `screen` (its links read with this player's state);
    /// returns whether the screen or the state changed.
    pub fn pointer(&mut self, screen: &Screen, event: Pointer, now: f64) -> bool {
        self.now = now;
        let inside = |area: &Area, clips: &[Area], p: Point| {
            area.contains(p) && clips.iter().all(|c| c.contains(p))
        };
        let under_links = |p: Point, triggers: &[&str]| -> Vec<&Link> {
            let mut found: Vec<&Link> = Vec::new();
            for link in screen.links.iter().rev() {
                if triggers.contains(&link.trigger)
                    && inside(&link.area, &link.clips, p)
                    && !found.iter().any(|f| f.layer == link.layer)
                {
                    found.push(link);
                }
            }
            found
        };
        let under = |p: Point, triggers: &[&str]| -> Vec<Layer> {
            under_links(p, triggers)
                .into_iter()
                .map(|l| l.layer.clone())
                .collect()
        };
        let mut changed = false;
        if let Pointer::Scroll(p, delta) = event {
            return self.scroll(screen, p, delta);
        }
        match event {
            Pointer::Move(_) | Pointer::Leave => {
                let at = match event {
                    Pointer::Move(p) => Some(p),
                    _ => None,
                };
                let (stay, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.hovered)
                    .into_iter()
                    .partition(|(_, area, clips)| at.is_some_and(|p| inside(area, clips, p)));
                let left: Vec<Layer> = gone.into_iter().map(|(l, _, _)| l).collect();
                let entered: Vec<(Layer, Area, Vec<Area>)> = at
                    .map(|p| under_links(p, HOVER))
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|l| !stay.iter().any(|(s, _, _)| *s == l.layer))
                    .map(|l| (l.layer.clone(), l.area, l.clips.clone()))
                    .collect();
                self.hovered = stay;
                self.hovered.extend(entered.iter().cloned());
                let entered: Vec<Layer> = entered.into_iter().map(|(l, _, _)| l).collect();
                for layer in &left {
                    changed |= self.revert(layer, "ON_HOVER", screen);
                    changed |= self.run(screen, layer, "MOUSE_LEAVE", false);
                    changed |= self.run(screen, layer, "MOUSE_OUT", false);
                }
                for layer in &entered {
                    changed |= self.run(screen, layer, "MOUSE_ENTER", false);
                    changed |= self.run(screen, layer, "MOUSE_IN", false);
                    changed |= self.run(screen, layer, "ON_HOVER", true);
                }
            }
            Pointer::Down(p) => {
                self.pressed = under(p, PRESS).into_iter().next();
                if let Some(layer) = self.pressed.clone() {
                    changed |= self.run(screen, &layer, "MOUSE_DOWN", false);
                    changed |= self.run(screen, &layer, "ON_PRESS", true);
                }
            }
            Pointer::Scroll(..) => {}
            Pointer::Up(p) => {
                let pressed = self.pressed.take();
                if let Some(layer) = &pressed {
                    changed |= self.revert(layer, "ON_PRESS", screen);
                }
                if let Some(layer) = under(p, PRESS).into_iter().next() {
                    changed |= self.run(screen, &layer, "MOUSE_UP", false);
                    if pressed.as_ref() == Some(&layer) {
                        changed |= self.run(screen, &layer, "ON_CLICK", false);
                        changed |= self.run(screen, &layer, "DRAG", false);
                    }
                }
            }
        }
        changed
    }

    /// The front-most frame under `p` that can still move by `delta` (an inner frame at its
    /// end lets an outer one scroll), with its offset now and its largest offset.
    fn scroller_for<'s>(
        &self,
        screen: &'s Screen,
        p: Point,
        delta: Vec2,
    ) -> Option<ScrollTarget<'s>> {
        screen.scrollers.iter().rev().find_map(|scroller| {
            if !(scroller.area.contains(p) && scroller.clips.iter().all(|c| c.contains(p))) {
                return None;
            }
            let at = self
                .state
                .scrolls
                .get(&scroller.layer.key())
                .copied()
                .unwrap_or((0, 0));
            let max = scroller.max();
            let can = |d: f64, a: i32, m: i32| (d > 0.0 && a < m) || (d < 0.0 && a > 0);
            (can(delta.x, at.0, max.0) || can(delta.y, at.1, max.1)).then_some((scroller, at, max))
        })
    }

    /// Whether a scroll by `delta` over `p` goes to a frame of `screen` (else the view pans).
    pub fn takes_scroll(&self, screen: &Screen, p: Point, delta: Vec2) -> bool {
        self.scroller_for(screen, p, delta).is_some()
    }

    /// Scrolls the frame [`Player::scroller_for`] finds; fractions of a unit add up over
    /// events (smooth scrolling sends small steps). Returns whether the offset changed.
    fn scroll(&mut self, screen: &Screen, p: Point, delta: Vec2) -> bool {
        let Some((scroller, (x, y), (mx, my))) = self.scroller_for(screen, p, delta) else {
            return false;
        };
        let key = scroller.layer.key();
        let rest = match &self.scroll_rest {
            Some((k, rest)) if *k == key => *rest,
            _ => Vec2::ZERO,
        };
        let want = Vec2::new(f64::from(x), f64::from(y)) + rest + delta;
        let next = (
            want.x.round().clamp(0.0, f64::from(mx)) as i32,
            want.y.round().clamp(0.0, f64::from(my)) as i32,
        );
        let left = want - Vec2::new(f64::from(next.0), f64::from(next.1));
        self.scroll_rest = Some((
            key.clone(),
            Vec2::new(left.x.clamp(-0.5, 0.5), left.y.clamp(-0.5, 0.5)),
        ));
        if next == (x, y) {
            return false;
        }
        if next == (0, 0) {
            self.state.scrolls.remove(&key);
        } else {
            self.state.scrolls.insert(key, next);
        }
        true
    }

    /// Fires the first "after delay" link of `screen` that is due at `now`; returns whether
    /// the screen or the state changed.
    pub fn tick(&mut self, screen: &Screen, now: f64) -> bool {
        self.now = now;
        let due = screen
            .links
            .iter()
            .filter(|l| l.trigger == "AFTER_TIMEOUT" && l.step.is_some())
            .find(|l| now - self.since >= l.delay)
            .and_then(|l| l.step.clone());
        due.is_some_and(|step| self.go(step))
    }

    /// When the next "after delay" link of `screen` is due, if it has any.
    pub fn next_due(&self, screen: &Screen) -> Option<f64> {
        screen
            .links
            .iter()
            .filter(|l| l.trigger == "AFTER_TIMEOUT" && l.step.is_some())
            .map(|l| self.since + l.delay)
            .reduce(f64::min)
    }

    /// Runs the played links of `layer` for `trigger`; `temporary` swaps are undone by
    /// [`Player::revert`].
    fn run(
        &mut self,
        screen: &Screen,
        layer: &Layer,
        trigger: &'static str,
        temporary: bool,
    ) -> bool {
        let steps: Vec<Step> = screen
            .links
            .iter()
            .filter(|l| l.trigger == trigger && &l.layer == layer)
            .filter_map(|l| l.step.clone())
            .collect();
        let mut changed = false;
        for step in steps {
            if temporary && let Step::Swap { target, .. } = &step {
                let key = target.key();
                if !self
                    .reverts
                    .iter()
                    .any(|(k, t, _)| *k == key && *t == trigger)
                {
                    let before = self.state.swaps.get(&key).copied();
                    self.reverts.push((key, trigger, before));
                }
            }
            changed |= self.go(step);
        }
        changed
    }

    /// Undoes the temporary swaps `layer`'s `trigger` links made.
    fn revert(&mut self, layer: &Layer, trigger: &'static str, screen: &Screen) -> bool {
        let targets: Vec<Vec<Guid>> = screen
            .links
            .iter()
            .filter(|l| &l.layer == layer)
            .filter_map(|l| match &l.step {
                Some(Step::Swap { target, .. }) => Some(target.key()),
                _ => None,
            })
            .chain(std::iter::once(layer.key()))
            .collect();
        let mut changed = false;
        self.reverts.retain(|(key, t, before)| {
            if *t != trigger || !targets.contains(key) {
                return true;
            }
            match before {
                Some(v) => changed |= self.state.swaps.insert(key.clone(), *v) != Some(*v),
                None => changed |= self.state.swaps.remove(key).is_some(),
            }
            false
        });
        if changed {
            self.since = self.now;
        }
        changed
    }
}

/// The triggers the player acts on.
pub const TRIGGERS: &[&str] = &[
    "ON_CLICK",
    "ON_HOVER",
    "ON_PRESS",
    "MOUSE_ENTER",
    "MOUSE_LEAVE",
    // Older names of mouse enter and leave.
    "MOUSE_IN",
    "MOUSE_OUT",
    "MOUSE_DOWN",
    "MOUSE_UP",
    "AFTER_TIMEOUT",
    "DRAG",
];

/// The instance a "change to `to`" on `layer` swaps, as in Figma: the layer or the nearest
/// instance around it whose current variant is in the same component set as `to`.
/// Returns the instance and the variant it is to show.
fn swap_target(
    doc: &Document,
    state: &State,
    definitions: &Arc<resolve::Definitions>,
    variants: &mut HashMap<Vec<Guid>, Option<Guid>>,
    layer: &Layer,
    to: Guid,
) -> Option<(Layer, Guid)> {
    let set = doc.node(to)?.parent?;
    // Many links share the instances around them: each one's variant is resolved once a read.
    let mut variant = |candidate: &Layer| -> Option<Guid> {
        let key = candidate.key();
        if let Some(swapped) = state.swaps.get(&key) {
            return Some(*swapped);
        }
        *variants.entry(key).or_insert_with(|| match candidate {
            Layer::Node(id) => {
                let node = doc.node(*id)?;
                (node.kind == "INSTANCE").then(|| resolve::symbol_id(&node.data))?
            }
            Layer::Path(path) => {
                let resolved = resolve::resolve_path_with(doc, definitions, path).ok()?;
                (resolved.data.text("type") == Some("INSTANCE"))
                    .then(|| resolve::symbol_id(&resolved.data))?
            }
        })
    };
    let mut candidates = Vec::new();
    let mut outer = match layer {
        Layer::Node(id) => Some(*id),
        Layer::Path(path) => {
            let keys = path.keys();
            for n in (1..=keys.len()).rev() {
                let mut guids = vec![path.instance()];
                guids.extend(&keys[..n]);
                candidates.extend(InstancePath::new(guids).map(Layer::Path));
            }
            Some(path.instance())
        }
    };
    let mut seen = HashSet::new();
    while let Some(id) = outer.filter(|id| seen.insert(*id)) {
        candidates.push(Layer::Node(id));
        outer = doc.node(id).and_then(|n| n.parent);
    }
    let mut set_of = |c: &Layer| variant(c).and_then(|v| doc.node(v)?.parent);
    if let Some(found) = candidates.iter().find(|c| set_of(c) == Some(set)) {
        return Some((found.clone(), to));
    }
    // The link may name a variant of another copy of the set (a library republished as a
    // new set): as Figma does, take the variant with the same properties in the candidate's
    // own set.
    let wanted = variant_props(&doc.node(to)?.name);
    candidates.into_iter().find_map(|c| {
        let own = set_of(&c)?;
        let same = doc.children(own).iter().find(|v| {
            doc.node(**v)
                .is_some_and(|n| variant_props(&n.name) == wanted)
        })?;
        Some((c, *same))
    })
}

/// A variant's name as its property assignments ("Size=L, State=Hover"), order-free.
fn variant_props(name: &str) -> Vec<(String, String)> {
    let mut props: Vec<(String, String)> = name
        .split(',')
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_owned(), v.trim().to_owned()))
        })
        .collect();
    props.sort();
    props
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
