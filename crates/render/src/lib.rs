//! Renders decoded `.fig` documents with Vello CPU.
//!
//! The same code path serves the desktop app, the headless CLI and the browser build.

pub mod geom;
mod paint;
mod text;

use std::borrow::Cow;
use std::fmt;

use figview_fig::{Document, Guid, Node, Val};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::filter_effects::{EdgeMode, Filter, FilterPrimitive};
use vello_cpu::kurbo::{Affine, BezPath, Point, Rect, Shape, Stroke};
use vello_cpu::peniko::Fill;
use vello_cpu::{Pixmap, RenderContext, RenderSettings, Resources};

use crate::geom::{affine, base_shape, clips_content, geometry, size, transformed_bounds};
use crate::paint::ImageCache;

pub use vello_cpu::{color, kurbo};

/// Instances nested deeper than this are not expanded (guards against cycles).
const MAX_INSTANCE_DEPTH: usize = 24;
/// Largest output side in pixels (Vello CPU uses 16-bit dimensions).
pub const MAX_SIDE: u32 = 16_384;

/// What was drawn, approximated or skipped. Shown in the app and the CLI so gaps are visible.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub nodes: usize,
    pub instances: usize,
    pub text_stored_glyphs: usize,
    pub text_system_fonts: usize,
    pub text_missing: usize,
    pub missing_images: usize,
    pub missing_symbols: usize,
    pub masks: usize,
    pub shadows: usize,
    pub blurs: usize,
    pub skipped_effects: usize,
    pub unsupported_paints: usize,
    pub approximated_paints: usize,
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} nodes, {} instances", self.nodes, self.instances)?;
        let mut note = |n: usize, what: &str| if n > 0 { write!(f, ", {n} {what}") } else { Ok(()) };
        note(self.text_stored_glyphs, "text (stored outlines)")?;
        note(self.text_system_fonts, "text (system fonts)")?;
        note(self.text_missing, "text not drawn")?;
        note(self.masks, "masks")?;
        note(self.shadows, "drop shadows")?;
        note(self.blurs, "blurs")?;
        note(self.skipped_effects, "effects skipped")?;
        note(self.missing_images, "images missing")?;
        note(self.missing_symbols, "components missing")?;
        note(self.unsupported_paints, "paints skipped")?;
        note(self.approximated_paints, "paints approximated")
    }
}

/// Where to draw: output size and the document-to-pixel transform.
#[derive(Clone, Copy, Debug)]
pub struct Viewport {
    pub width: u16,
    pub height: u16,
    pub transform: Affine,
    /// Fill behind everything; `None` keeps the output transparent.
    pub background: Option<AlphaColor<Srgb>>,
}

pub struct Frame {
    pub pixmap: Pixmap,
    pub stats: Stats,
}

/// Holds caches (decoded images, system fonts) between renders of the same document.
pub struct Renderer {
    images: ImageCache,
    fonts: text::SystemFonts,
    /// Lay out text with installed fonts when the file has no stored glyph outlines.
    pub system_fonts: bool,
}

impl Default for Renderer {
    fn default() -> Self {
        Renderer { images: ImageCache::default(), fonts: text::SystemFonts::default(), system_fonts: true }
    }
}

struct Patch {
    path: Vec<Guid>,
    data: Val,
    owner_depth: usize,
    derived: bool,
}

struct Ctx<'a> {
    doc: &'a Document,
    rc: RenderContext,
    stats: Stats,
    images: &'a mut ImageCache,
    fonts: Option<&'a mut text::SystemFonts>,
    patches: Vec<Patch>,
    symbol_stack: Vec<Guid>,
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Draws `root` (a page or any node) into `viewport`.
    pub fn render(&mut self, doc: &Document, root: Guid, viewport: &Viewport) -> Frame {
        let (w, h) = (viewport.width.max(1), viewport.height.max(1));
        let mut ctx = Ctx {
            doc,
            rc: RenderContext::new_with(w, h, RenderSettings::default()),
            stats: Stats::default(),
            images: &mut self.images,
            fonts: if self.system_fonts { Some(&mut self.fonts) } else { None },
            patches: Vec::new(),
            symbol_stack: Vec::new(),
        };
        if let Some(bg) = viewport.background {
            ctx.rc.set_paint(bg);
            ctx.rc.fill_rect(&Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        }
        if let Some(node) = doc.node(root) {
            if node.kind == "CANVAS" || node.kind == "DOCUMENT" {
                ctx.draw_children(node, &node.data, viewport.transform, None);
            } else {
                let parent = node.parent.map_or(Affine::IDENTITY, |p| absolute_transform(doc, p));
                ctx.draw(node, viewport.transform * parent, None);
            }
        }
        ctx.rc.flush();
        let mut pixmap = Pixmap::new(w, h);
        ctx.rc.render(&mut pixmap, &mut Resources::new());
        Frame { pixmap, stats: ctx.stats }
    }

    /// Renders `root` cropped to its content at `scale`, with `padding` pixels around it.
    pub fn render_fit(
        &mut self,
        doc: &Document,
        root: Guid,
        scale: f64,
        padding: f64,
        background: Option<AlphaColor<Srgb>>,
    ) -> Option<Frame> {
        let bounds = content_bounds(doc, root)?;
        let mut scale = scale.max(0.01);
        let longest = bounds.width().max(bounds.height()) * scale + padding * 2.0;
        if longest > f64::from(MAX_SIDE) {
            scale *= (f64::from(MAX_SIDE) - padding * 2.0) / (longest - padding * 2.0);
        }
        let width = (bounds.width() * scale + padding * 2.0).ceil().max(1.0) as u16;
        let height = (bounds.height() * scale + padding * 2.0).ceil().max(1.0) as u16;
        let transform = Affine::translate((padding, padding)) * Affine::scale(scale) * Affine::translate((-bounds.x0, -bounds.y0));
        Some(self.render(doc, root, &Viewport { width, height, transform, background }))
    }
}

/// The page background colour Figma shows behind a canvas.
pub fn page_background(doc: &Document, page: Guid) -> AlphaColor<Srgb> {
    doc.node(page)
        .and_then(|p| paint::color(p.data.get("backgroundColor"), 1.0))
        .unwrap_or(AlphaColor::new([0.96, 0.96, 0.96, 1.0]))
}

/// Node-to-page transform (product of ancestor transforms; pages contribute nothing).
pub fn absolute_transform(doc: &Document, id: Guid) -> Affine {
    let mut chain = Vec::new();
    let mut cur = Some(id);
    while let Some(c) = cur {
        let Some(node) = doc.node(c) else { break };
        if node.kind == "CANVAS" || node.kind == "DOCUMENT" {
            break;
        }
        chain.push(affine(node.data.get("transform")));
        cur = node.parent;
    }
    chain.iter().rev().fold(Affine::IDENTITY, |acc, t| acc * *t)
}

/// Page-space bounding box of a node's frame.
pub fn node_bounds(doc: &Document, id: Guid) -> Option<Rect> {
    let node = doc.node(id)?;
    let (w, h) = size(&node.data);
    Some(transformed_bounds(absolute_transform(doc, id), Rect::new(0.0, 0.0, w, h)))
}

fn visible(data: &Val) -> bool {
    data.flag("visible") != Some(false)
}

/// Bounds of what `root` draws: a page's visible children, or the node itself.
pub fn content_bounds(doc: &Document, root: Guid) -> Option<Rect> {
    let node = doc.node(root)?;
    if node.kind != "CANVAS" && node.kind != "DOCUMENT" {
        return node_bounds(doc, root).map(|b| b.inflate(1.0, 1.0));
    }
    let mut out: Option<Rect> = None;
    for child in doc.children(root) {
        let Some(c) = doc.node(*child) else { continue };
        if !visible(&c.data) {
            continue;
        }
        if let Some(b) = node_bounds(doc, *child) {
            out = Some(out.map_or(b, |o| o.union(b)));
        }
    }
    out.map(|b| b.inflate(1.0, 1.0))
}

/// Topmost, deepest visible node under `point` (page coordinates). Instances count as one layer.
pub fn hit_test(doc: &Document, page: Guid, point: Point) -> Option<Guid> {
    fn walk(doc: &Document, parent: Guid, parent_tf: Affine, point: Point) -> Option<Guid> {
        for id in doc.children(parent).iter().rev() {
            let Some(node) = doc.node(*id) else { continue };
            if !visible(&node.data) || node.data.flag("mask") == Some(true) {
                continue;
            }
            let tf = parent_tf * affine(node.data.get("transform"));
            let (w, h) = size(&node.data);
            let local = tf.inverse() * point;
            let inside = local.x >= 0.0 && local.y >= 0.0 && local.x <= w && local.y <= h;
            let clips = clips_content(&node.kind, &node.data);
            if node.kind != "INSTANCE" && (inside || !clips) {
                if let Some(hit) = walk(doc, *id, tf, point) {
                    return Some(hit);
                }
            }
            if inside && node.kind != "SECTION" {
                return Some(*id);
            }
        }
        None
    }
    walk(doc, page, Affine::IDENTITY, point)
}

impl Ctx<'_> {
    /// Node data with instance overrides applied, plus whether stored text glyphs are stale.
    fn patched<'n>(&self, node: &'n Node, path: Option<&[Guid]>) -> (Cow<'n, Val>, bool) {
        let Some(path) = path else {
            return (Cow::Borrowed(&node.data), false);
        };
        let mut matches: Vec<&Patch> = self.patches.iter().filter(|p| p.path == path).collect();
        if matches.is_empty() {
            return (Cow::Borrowed(&node.data), false);
        }
        matches.sort_by_key(|p| (std::cmp::Reverse(p.owner_depth), p.derived));
        let mut data = node.data.clone();
        let (mut text_changed, mut glyphs_fresh) = (false, false);
        for p in matches {
            if p.data.get("textData").is_some() && !p.derived {
                text_changed = true;
                glyphs_fresh = false;
            }
            if p.data.get("derivedTextData").is_some() && p.derived {
                glyphs_fresh = true;
            }
            data = data.merged(&p.data);
        }
        (Cow::Owned(data), text_changed && !glyphs_fresh)
    }

    fn draw(&mut self, node: &Node, parent: Affine, path: Option<&[Guid]>) {
        let (data, stale_glyphs) = self.patched(node, path);
        let data = data.as_ref();
        if !visible(data) || data.flag("mask") == Some(true) {
            return;
        }
        self.stats.nodes += 1;
        let kind = node.kind.as_str();
        let world = parent * affine(data.get("transform"));
        let (w, h) = size(data);
        let mut layers = 0;

        for effect in data.get("effects").map(Val::items).unwrap_or_default() {
            if effect.flag("visible") == Some(false) {
                continue;
            }
            let radius = effect.num("radius").unwrap_or(0.0).max(0.0);
            let primitive = match effect.text("type") {
                Some("DROP_SHADOW") if layers == 0 => {
                    self.stats.shadows += 1;
                    let offset = effect.get("offset");
                    FilterPrimitive::DropShadow {
                        dx: offset.and_then(|o| o.num("x")).unwrap_or(0.0),
                        dy: offset.and_then(|o| o.num("y")).unwrap_or(0.0),
                        std_deviation: radius / 2.0,
                        color: paint::color(effect.get("color"), 1.0).unwrap_or(AlphaColor::new([0.0, 0.0, 0.0, 0.25])),
                        edge_mode: EdgeMode::None,
                    }
                }
                Some("FOREGROUND_BLUR" | "LAYER_BLUR") if layers == 0 => {
                    self.stats.blurs += 1;
                    FilterPrimitive::GaussianBlur { std_deviation: radius / 2.0, edge_mode: EdgeMode::None }
                }
                _ => {
                    self.stats.skipped_effects += 1;
                    continue;
                }
            };
            self.rc.set_transform(world);
            self.rc.push_filter_layer(Filter::from_primitive(primitive));
            layers += 1;
        }
        let opacity = data.num("opacity").unwrap_or(1.0).clamp(0.0, 1.0);
        if opacity < 1.0 {
            self.rc.set_transform(world);
            self.rc.push_opacity_layer(opacity);
            layers += 1;
        }

        // Fills.
        let mut fill_paths = geometry(data, "fillGeometry", &self.doc.blobs);
        if kind == "TEXT" {
            fill_paths = self.text_paths(data, stale_glyphs).into_iter().map(|p| (p, Fill::NonZero)).collect();
        } else if fill_paths.is_empty() {
            if let Some(shape) = base_shape(kind, data) {
                fill_paths.push((shape, Fill::NonZero));
            }
        }
        self.paint_paths(data, "fillPaints", &fill_paths, world, w, h);

        // Strokes: precomputed outline geometry, or stroke the base shape.
        if data.get("strokePaints").is_some_and(|p| !p.items().is_empty()) {
            let outlines = geometry(data, "strokeGeometry", &self.doc.blobs);
            if !outlines.is_empty() {
                self.paint_paths(data, "strokePaints", &outlines, world, w, h);
            } else if let (Some(weight), Some(shape)) = (data.num("strokeWeight"), base_shape(kind, data)) {
                if weight > 0.0 {
                    let outline = vello_cpu::kurbo::stroke(
                        shape.path_elements(0.1),
                        &Stroke::new(f64::from(weight)),
                        &Default::default(),
                        0.1,
                    );
                    self.paint_paths(data, "strokePaints", &[(outline, Fill::NonZero)], world, w, h);
                }
            }
        }

        // Children.
        if kind != "TEXT" {
            let clip = clips_content(kind, data).then(|| base_shape(kind, data)).flatten();
            if let Some(clip) = &clip {
                self.rc.set_transform(world);
                self.rc.push_clip_layer(clip);
            }
            if kind == "INSTANCE" {
                self.draw_instance(data, world, path);
            } else {
                let child_path = path.map(|p| p.to_vec());
                self.draw_children(node, data, world, child_path.as_deref());
            }
            if clip.is_some() {
                self.rc.pop_layer();
            }
        }
        for _ in 0..layers {
            self.rc.pop_layer();
        }
    }

    fn text_paths(&mut self, data: &Val, stale_glyphs: bool) -> Vec<BezPath> {
        if !stale_glyphs {
            let stored = text::stored_glyphs(data, &self.doc.blobs);
            if !stored.is_empty() {
                self.stats.text_stored_glyphs += 1;
                return stored;
            }
        }
        if text::characters(data).trim().is_empty() {
            return Vec::new();
        }
        if let Some(paths) = self.fonts.as_mut().and_then(|f| f.layout(data)) {
            if !paths.is_empty() {
                self.stats.text_system_fonts += 1;
                return paths;
            }
        }
        self.stats.text_missing += 1;
        Vec::new()
    }

    fn paint_paths(&mut self, data: &Val, key: &str, paths: &[(BezPath, Fill)], world: Affine, w: f64, h: f64) {
        if paths.is_empty() {
            return;
        }
        for p in data.get(key).map(Val::items).unwrap_or_default() {
            let Some(resolved) = paint::resolve(p, w, h, self.doc, self.images, &mut self.stats) else {
                continue;
            };
            self.rc.set_transform(world);
            if let Some(clip) = resolved.clip {
                self.rc.push_clip_layer(&clip.to_path(0.1));
            }
            self.rc.set_paint(resolved.paint);
            self.rc.set_paint_transform(resolved.transform);
            for (path, rule) in paths {
                self.rc.set_fill_rule(*rule);
                self.rc.fill_path(path);
            }
            self.rc.set_paint_transform(Affine::IDENTITY);
            if resolved.clip.is_some() {
                self.rc.pop_layer();
            }
        }
    }

    /// Draws children, turning mask layers into clips for the siblings above them.
    fn draw_children(&mut self, node: &Node, _data: &Val, world: Affine, path: Option<&[Guid]>) {
        let mut mask_layers = 0;
        for id in self.doc.children(node.id) {
            let Some(child) = self.doc.node(*id) else { continue };
            let child_path = path.map(|p| {
                let mut v = p.to_vec();
                v.push(child.override_key());
                v
            });
            let (child_data, _) = self.patched(child, child_path.as_deref());
            if child_data.flag("mask") == Some(true) && visible(&child_data) {
                let tf = world * affine(child_data.get("transform"));
                let mut shapes = geometry(&child_data, "fillGeometry", &self.doc.blobs);
                if shapes.is_empty() {
                    shapes.extend(base_shape(&child.kind, &child_data).map(|s| (s, Fill::NonZero)));
                }
                let mut combined = BezPath::new();
                for (shape, _) in shapes {
                    combined.extend(shape);
                }
                if !combined.elements().is_empty() {
                    self.stats.masks += 1;
                    self.rc.set_transform(tf);
                    self.rc.push_clip_layer(&combined);
                    mask_layers += 1;
                }
                continue;
            }
            self.draw(child, world, child_path.as_deref());
        }
        for _ in 0..mask_layers {
            self.rc.pop_layer();
        }
    }

    fn draw_instance(&mut self, data: &Val, world: Affine, path: Option<&[Guid]>) {
        self.stats.instances += 1;
        let symbol_id = data
            .get("overriddenSymbolID")
            .and_then(Val::as_guid)
            .or_else(|| data.path(&["symbolData", "symbolID"]).and_then(Val::as_guid));
        let Some(symbol) = symbol_id.and_then(|id| self.doc.node(id)) else {
            self.stats.missing_symbols += 1;
            return;
        };
        if self.symbol_stack.contains(&symbol.id) || self.symbol_stack.len() >= MAX_INSTANCE_DEPTH {
            return;
        }
        let base: Vec<Guid> = path.map(<[Guid]>::to_vec).unwrap_or_default();
        let owner_depth = self.symbol_stack.len();
        let before = self.patches.len();
        let lists = [
            (data.path(&["symbolData", "symbolOverrides"]), false),
            (data.get("derivedSymbolData"), true),
        ];
        for (list, derived) in lists {
            for item in list.map(Val::items).unwrap_or_default() {
                let guids: Vec<Guid> = item
                    .path(&["guidPath", "guids"])
                    .map(Val::items)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(Val::as_guid)
                    .collect();
                if guids.is_empty() {
                    continue;
                }
                let mut full = base.clone();
                full.extend(guids);
                self.patches.push(Patch { path: full, data: item.clone(), owner_depth, derived });
            }
        }
        self.symbol_stack.push(symbol.id);
        self.draw_children(symbol, &symbol.data, world, Some(&base));
        self.symbol_stack.pop();
        self.patches.truncate(before);
    }
}
