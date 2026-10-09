//! Renders decoded `.fig` documents with Vello CPU.
//!
//! The same code path serves the desktop app, the headless CLI and the browser build.

pub mod geom;
mod instance;
mod layout;
pub mod paint;
mod text;
mod variables;

use std::borrow::Cow;
use std::fmt;

use figstudio_fig::{Document, Guid, Node, Val};
use vello_cpu::color::{AlphaColor, Srgb};
use vello_cpu::filter_effects::{EdgeMode, Filter, FilterPrimitive};
use vello_cpu::kurbo::{Affine, BezPath, Point, Rect, Shape, Stroke};
use vello_cpu::{Pixmap, RenderContext, RenderSettings, Resources};

use crate::geom::{
    GeomPath, affine, base_shape, clips_content, geometry, size, transformed_bounds,
};
use crate::paint::ImageCache;

pub use vello_cpu::{color, kurbo};

/// Instances nested deeper than this are not expanded (guards against cycles).
const MAX_INSTANCE_DEPTH: usize = 24;
/// Total attempted-work units per render. The real large-file page render used 12,418 work
/// units, so this cap leaves over 160x headroom before any truncation is possible.
pub const MAX_RENDER_WORK: usize = 2_000_000;
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
    pub approximated_blends: usize,
    pub approximated_paints: usize,
    /// Layers skipped because they are outside the viewport or smaller than a pixel.
    pub culled: usize,
    /// Nodes or instance expansions skipped after exhausting the shared work budget.
    pub budget_exhausted: usize,
    pub depth_skipped: usize,
    pub preparation_skipped: usize,
    pub attempted_work: usize,
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} nodes, {} instances", self.nodes, self.instances)?;
        let mut note = |n: usize, what: &str| {
            if n > 0 {
                write!(f, ", {n} {what}")
            } else {
                Ok(())
            }
        };
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
        note(self.approximated_blends, "blend modes approximated")?;
        note(self.approximated_paints, "paints approximated")?;
        note(self.culled, "off screen")?;
        note(self.budget_exhausted, "layers skipped (work budget)")?;
        note(self.depth_skipped, "layers skipped (depth)")?;
        note(self.preparation_skipped, "bounds omitted (budget)")
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
        Renderer {
            images: ImageCache::default(),
            fonts: text::SystemFonts::default(),
            system_fonts: true,
        }
    }
}

struct Patch {
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
    patch_index: std::collections::HashMap<Vec<Guid>, Vec<usize>>,
    symbol_stack: Vec<Guid>,
    instance_scales: Vec<f64>,
    properties: instance::Properties,
    variables: variables::Variables,
    work_left: usize,
    depth: usize,
    exhausted: bool,
    /// Output area in pixels, for culling.
    screen: Rect,
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn image_cache_bytes(&self) -> usize {
        self.images.retained_bytes()
    }
    pub fn render(&mut self, doc: &Document, root: Guid, viewport: &Viewport) -> Frame {
        self.render_budgeted(doc, root, viewport, MAX_RENDER_WORK, 0)
    }
    fn render_budgeted(
        &mut self,
        doc: &Document,
        root: Guid,
        viewport: &Viewport,
        work: usize,
        preparation_skipped: usize,
    ) -> Frame {
        let (w, h) = (viewport.width.max(1), viewport.height.max(1));
        let mut work_left = work;
        let properties = instance::Properties::new(doc, &mut work_left);
        let variables = variables::Variables::new(doc, &mut work_left);
        let preparation_skipped = preparation_skipped + usize::from(work_left == 0);
        let mut ctx = Ctx {
            doc,
            rc: RenderContext::new_with(w, h, RenderSettings::default()),
            stats: Stats {
                preparation_skipped,
                ..Stats::default()
            },
            images: &mut self.images,
            fonts: if self.system_fonts {
                Some(&mut self.fonts)
            } else {
                None
            },
            patches: Vec::new(),
            patch_index: std::collections::HashMap::new(),
            symbol_stack: Vec::new(),
            instance_scales: Vec::new(),
            properties,
            variables,
            work_left,
            depth: 0,
            exhausted: false,
            screen: Rect::new(0.0, 0.0, f64::from(w), f64::from(h)),
        };
        if let Some(bg) = viewport.background {
            ctx.rc.set_paint(bg);
            ctx.rc
                .fill_rect(&Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        }
        if let Some(node) = doc.node(root) {
            if node.kind == "CANVAS" || node.kind == "DOCUMENT" {
                ctx.draw_children(node, &node.data, viewport.transform, None);
            } else {
                let parent = node.parent.map_or(Some(Affine::IDENTITY), |p| {
                    absolute_transform_with_budget(doc, p, &mut ctx.work_left)
                });
                if let Some(parent) = parent {
                    if ctx.charge_work() {
                        ctx.draw(node, viewport.transform * parent, None);
                    }
                } else {
                    ctx.exhausted = true;
                    ctx.stats.budget_exhausted += 1;
                }
            }
        }
        ctx.rc.flush();
        let mut pixmap = Pixmap::new(w, h);
        ctx.rc.render(&mut pixmap, &mut Resources::new());
        ctx.stats.attempted_work = work.saturating_sub(ctx.work_left);
        Frame {
            pixmap,
            stats: ctx.stats,
        }
    }
    pub fn render_fit(
        &mut self,
        doc: &Document,
        root: Guid,
        scale: f64,
        padding: f64,
        background: Option<AlphaColor<Srgb>>,
    ) -> Option<Frame> {
        let (bounds, omitted, prep_work) = content_bounds_with_budget(doc, root, MAX_RENDER_WORK);
        let bounds = bounds?;
        let mut scale = scale.max(0.01);
        let longest = bounds.width().max(bounds.height()) * scale + padding * 2.0;
        if longest > f64::from(MAX_SIDE) {
            scale *= (f64::from(MAX_SIDE) - padding * 2.0) / (longest - padding * 2.0);
        }
        let width = (bounds.width() * scale + padding * 2.0).ceil().max(1.0) as u16;
        let height = (bounds.height() * scale + padding * 2.0).ceil().max(1.0) as u16;
        let transform = Affine::translate((padding, padding))
            * Affine::scale(scale)
            * Affine::translate((-bounds.x0, -bounds.y0));
        let mut frame = self.render_budgeted(
            doc,
            root,
            &Viewport {
                width,
                height,
                transform,
                background,
            },
            MAX_RENDER_WORK.saturating_sub(prep_work),
            omitted,
        );
        frame.stats.attempted_work += prep_work;
        Some(frame)
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
    let mut remaining = usize::MAX;
    absolute_transform_with_budget(doc, id, &mut remaining).unwrap_or(Affine::IDENTITY)
}
fn absolute_transform_with_budget(
    doc: &Document,
    id: Guid,
    remaining: &mut usize,
) -> Option<Affine> {
    let mut chain = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cur = Some(id);
    while let Some(c) = cur {
        if *remaining == 0 {
            return None;
        }
        *remaining -= 1;
        if !seen.insert(c) {
            break;
        }
        let Some(node) = doc.node(c) else {
            break;
        };
        if node.kind == "CANVAS" || node.kind == "DOCUMENT" {
            break;
        }
        chain.push(affine(node.data.get("transform")));
        cur = node.parent;
    }
    Some(chain.iter().rev().fold(Affine::IDENTITY, |acc, t| acc * *t))
}
pub fn node_bounds(doc: &Document, id: Guid) -> Option<Rect> {
    let mut remaining = usize::MAX;
    node_bounds_with_budget(doc, id, &mut remaining)
}
fn node_bounds_with_budget(doc: &Document, id: Guid, remaining: &mut usize) -> Option<Rect> {
    let node = doc.node(id)?;
    let (w, h) = size(&node.data);
    Some(transformed_bounds(
        absolute_transform_with_budget(doc, id, remaining)?,
        Rect::new(0.0, 0.0, w, h),
    ))
}
fn visible(data: &Val) -> bool {
    data.flag("visible") != Some(false)
}
pub fn content_bounds(doc: &Document, root: Guid) -> Option<Rect> {
    content_bounds_with_budget(doc, root, MAX_RENDER_WORK).0
}
fn content_bounds_with_budget(
    doc: &Document,
    root: Guid,
    mut remaining: usize,
) -> (Option<Rect>, usize, usize) {
    let Some(node) = doc.node(root) else {
        return (None, 0, 0);
    };
    let initial = remaining;
    let root_parent = if node.kind == "CANVAS" || node.kind == "DOCUMENT" {
        Affine::IDENTITY
    } else {
        node.parent
            .and_then(|parent| absolute_transform_with_budget(doc, parent, &mut remaining))
            .unwrap_or(Affine::IDENTITY)
    };
    let mut out: Option<Rect> = None;
    let mut omitted = 0;
    let mut seen = std::collections::HashSet::new();
    let mut stack = Vec::new();
    if node.kind == "CANVAS" || node.kind == "DOCUMENT" {
        stack.extend(doc.children(root).iter().rev().map(|id| (*id, root_parent)));
    } else {
        stack.push((root, root_parent));
    }
    while let Some((id, parent)) = stack.pop() {
        if remaining == 0 {
            omitted += 1 + stack.len();
            break;
        }
        remaining -= 1;
        if !seen.insert(id) {
            continue;
        }
        let Some(child) = doc.node(id) else { continue };
        if !visible(&child.data) {
            continue;
        }
        let world = parent * affine(child.data.get("transform"));
        let (w, h) = size(&child.data);
        let margin = overhang(&child.kind, &child.data, w, h);
        let mut bounds =
            transformed_bounds(world, Rect::new(-margin, -margin, w + margin, h + margin));
        if clips_content(&child.kind, &child.data) {
            bounds = bounds.intersect(transformed_bounds(world, Rect::new(0.0, 0.0, w, h)));
        }
        out = Some(out.map_or(bounds, |current| current.union(bounds)));
        if !clips_content(&child.kind, &child.data) {
            stack.extend(doc.children(id).iter().rev().map(|id| (*id, world)));
        }
    }
    (
        out.map(|b| b.inflate(1.0, 1.0)),
        omitted,
        initial - remaining,
    )
}

/// Topmost, deepest visible node under `point` (page coordinates). Instances count as one layer.
pub fn hit_test(doc: &Document, page: Guid, point: Point) -> Option<Guid> {
    fn walk(
        doc: &Document,
        parent: Guid,
        parent_tf: Affine,
        point: Point,
        seen: &mut std::collections::HashSet<Guid>,
        depth: usize,
    ) -> Option<Guid> {
        if depth >= 128 || !seen.insert(parent) {
            return None;
        }
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
            if node.kind != "INSTANCE"
                && (inside || !clips)
                && let Some(hit) = walk(doc, *id, tf, point, seen, depth + 1)
            {
                return Some(hit);
            }
            if inside && node.kind != "SECTION" {
                return Some(*id);
            }
        }
        None
    }
    walk(
        doc,
        page,
        Affine::IDENTITY,
        point,
        &mut std::collections::HashSet::new(),
        0,
    )
}

/// How far (in local units) a node can draw outside its frame: strokes, effects, and text
/// that overflows its box. Deliberately generous; culling must never hide something visible.
fn overhang(kind: &str, data: &Val, w: f64, h: f64) -> f64 {
    let mut m = 1.0;
    if data
        .get("strokePaints")
        .is_some_and(|p| !p.items().is_empty())
    {
        // Outside strokes plus miter joins (Figma's miter limit is 4).
        m += f64::from(data.num("strokeWeight").unwrap_or(1.0).max(0.0)) * 4.0;
    }
    for e in data.get("effects").map(Val::items).unwrap_or_default() {
        if e.flag("visible") == Some(false) {
            continue;
        }
        let r = f64::from(e.num("radius").unwrap_or(0.0).abs());
        let spread = f64::from(e.num("spread").unwrap_or(0.0).abs());
        let (dx, dy) = e.get("offset").map_or((0.0, 0.0), |o| {
            (
                f64::from(o.num("x").unwrap_or(0.0)),
                f64::from(o.num("y").unwrap_or(0.0)),
            )
        });
        m += r * 3.0 + spread + dx.abs() + dy.abs();
    }
    if kind == "TEXT" {
        m += w.max(h) + f64::from(data.num("fontSize").unwrap_or(12.0)) * 4.0;
    }
    m
}

impl Ctx<'_> {
    /// True when the node certainly draws nothing inside the output. Only nodes whose content
    /// stays within their frame are tested: leaves, clipping frames and instances, and boolean
    /// operations. Other containers are walked so their children decide for themselves.
    fn off_screen(
        &self,
        node: &Node,
        kind: &str,
        data: &Val,
        world: Affine,
        w: f64,
        h: f64,
    ) -> bool {
        let leaf = kind != "INSTANCE" && self.doc.children(node.id).is_empty();
        let bounded = leaf || kind == "BOOLEAN_OPERATION" || clips_content(kind, data);
        if !bounded {
            return false;
        }
        let m = overhang(kind, data, w, h);
        let b = transformed_bounds(world, Rect::new(-m, -m, w + m, h + m));
        if !b.overlaps(self.screen) {
            return true;
        }
        // A leaf smaller than a quarter pixel each way contributes nothing visible.
        leaf && kind != "TEXT" && {
            let tiny = transformed_bounds(world, Rect::new(0.0, 0.0, w, h));
            tiny.width() < 0.25 && tiny.height() < 0.25 && m <= 1.0
        }
    }

    /// Node data with instance overrides applied, plus whether stored text glyphs are stale.
    fn patched<'n>(&self, node: &'n Node, path: Option<&[Guid]>) -> (Cow<'n, Val>, bool) {
        let Some(path) = path else {
            return (Cow::Borrowed(&node.data), false);
        };
        let mut matches: Vec<&Patch> = self
            .patch_index
            .get(path)
            .into_iter()
            .flatten()
            .filter_map(|index| self.patches.get(*index))
            .collect();
        let mut property_patch = self.properties.patch(&node.data);
        let scale = self.instance_scales.last().copied().unwrap_or(1.0);
        if scale != 1.0 && !matches.iter().any(|p| p.data.get("transform").is_some()) {
            let tf = affine(node.data.get("transform")).as_coeffs();
            let mut transform = node
                .data
                .get("transform")
                .cloned()
                .unwrap_or(Val::Obj(figstudio_fig::Obj::new()));
            transform.set("m02", Val::Float(tf[4] * scale));
            transform.set("m12", Val::Float(tf[5] * scale));
            property_patch.set("transform", transform);
        }
        if matches.is_empty() && property_patch == Val::Obj(figstudio_fig::Obj::new()) {
            return (Cow::Borrowed(&node.data), false);
        }
        matches.sort_by_key(|p| (std::cmp::Reverse(p.owner_depth), p.derived));
        let mut data = node.data.merged(&property_patch);
        let mut text_changed = data.get("textData") != node.data.get("textData");
        let mut glyphs_fresh = false;
        for p in matches {
            if p.data.get("textData").is_some() && !p.derived {
                text_changed = true;
                glyphs_fresh = false;
            }
            if p.data.get("derivedTextData").is_some() && p.derived {
                glyphs_fresh = true;
            }
            data = data.merged(&p.data);
            // An explicit paint override replaces the paints of a style it does not restate.
            for (style, key) in variables::STYLED_PAINTS {
                if p.data.get(key).is_some()
                    && p.data.get(style).is_none()
                    && let Val::Obj(o) = &mut data
                {
                    o.remove(style);
                }
            }
        }
        (Cow::Owned(data), text_changed && !glyphs_fresh)
    }

    fn draw(&mut self, node: &Node, parent: Affine, path: Option<&[Guid]>) {
        self.draw_with_data(node, parent, path, None);
    }

    fn draw_with_data<'n>(
        &mut self,
        node: &'n Node,
        parent: Affine,
        path: Option<&[Guid]>,
        patched: Option<(Cow<'n, Val>, bool)>,
    ) {
        if self.depth >= 96 {
            self.stats.depth_skipped += 1;
            return;
        }
        if !self.charge_work() {
            return;
        }
        self.depth += 1;
        let (data, stale_glyphs) = patched.unwrap_or_else(|| self.patched(node, path));
        let (data, unresolved_variables) = self.variables.paints(data.as_ref());
        let data = data.as_ref();
        self.stats.approximated_paints += unresolved_variables;
        if !visible(data) || data.flag("mask") == Some(true) {
            self.depth -= 1;
            return;
        }
        let kind = node.kind.as_str();
        let world = parent * affine(data.get("transform"));
        let (w, h) = size(data);
        if self.off_screen(node, kind, data, world, w, h) {
            self.stats.culled += 1;
            self.depth -= 1;
            return;
        }
        self.stats.nodes += 1;
        if data.text("blendMode").is_some_and(|mode| mode != "NORMAL") {
            self.stats.approximated_blends += 1;
        }
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
                        color: paint::color(effect.get("color"), 1.0)
                            .unwrap_or(AlphaColor::new([0.0, 0.0, 0.0, 0.25])),
                        edge_mode: EdgeMode::None,
                    }
                }
                // Progressive blur fades in across the layer; a uniform blur looks far worse
                // than none, so it is skipped (and counted) until it is implemented.
                Some("FOREGROUND_BLUR" | "LAYER_BLUR")
                    if effect.text("blurOpType") == Some("PROGRESSIVE") =>
                {
                    self.stats.skipped_effects += 1;
                    continue;
                }
                Some("FOREGROUND_BLUR" | "LAYER_BLUR") if layers == 0 => {
                    self.stats.blurs += 1;
                    FilterPrimitive::GaussianBlur {
                        std_deviation: radius / 2.0,
                        edge_mode: EdgeMode::None,
                    }
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
        let mut fill_paths = geometry(
            data,
            "fillGeometry",
            &self.doc.blobs,
            &mut self.stats.unsupported_paints,
        );
        if kind == "VECTOR"
            && data
                .get("fillGeometry")
                .is_none_or(|geometry| geometry.items().is_empty())
        {
            self.stats.unsupported_paints += 1;
        }
        if kind == "TEXT" {
            fill_paths = self
                .text_paths(data, stale_glyphs)
                .into_iter()
                .map(GeomPath::plain)
                .collect();
        } else if fill_paths.is_empty()
            && let Some(shape) = base_shape(kind, data)
        {
            fill_paths.push(GeomPath::plain(shape));
        }
        self.paint_paths(data, "fillPaints", &fill_paths, world, w, h);

        // Strokes: precomputed outline geometry, or stroke the base shape.
        if data
            .get("strokePaints")
            .is_some_and(|p| !p.items().is_empty())
        {
            let outlines = geometry(
                data,
                "strokeGeometry",
                &self.doc.blobs,
                &mut self.stats.unsupported_paints,
            );
            if !outlines.is_empty() {
                self.paint_paths(data, "strokePaints", &outlines, world, w, h);
            } else if let (Some(weight), Some(shape)) =
                (data.num("strokeWeight"), base_shape(kind, data))
                && weight > 0.0
            {
                let outline = vello_cpu::kurbo::stroke(
                    shape.path_elements(0.1),
                    &Stroke::new(f64::from(weight)),
                    &Default::default(),
                    0.1,
                );
                self.paint_paths(
                    data,
                    "strokePaints",
                    &[GeomPath::plain(outline)],
                    world,
                    w,
                    h,
                );
            }
        }

        // Children.
        if kind != "TEXT" {
            let clip = clips_content(kind, data)
                .then(|| base_shape(kind, data))
                .flatten();
            if let Some(clip) = &clip {
                self.rc.set_transform(world);
                self.rc.push_clip_layer(clip);
            }
            if kind == "INSTANCE" {
                self.draw_instance(data, world, path);
            } else {
                // Override paths contain instance ancestors, not ordinary frame/group ancestors.
                let child_path = path.map(|p| &p[..p.len().saturating_sub(1)]);
                self.draw_children(node, data, world, child_path);
            }
            if clip.is_some() {
                self.rc.pop_layer();
            }
        }
        for _ in 0..layers {
            self.rc.pop_layer();
        }
        self.depth -= 1;
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
        if let Some(paths) = self.fonts.as_mut().and_then(|f| f.layout(data))
            && !paths.is_empty()
        {
            self.stats.text_system_fonts += 1;
            return paths;
        }
        self.stats.text_missing += 1;
        Vec::new()
    }

    /// Paints `paths` with the node's `key` paints. Paths with a vector style id use the paints
    /// from `vectorData.styleOverrideTable` instead, when that entry overrides them.
    fn paint_paths(
        &mut self,
        data: &Val,
        key: &str,
        paths: &[GeomPath],
        world: Affine,
        w: f64,
        h: f64,
    ) {
        if paths.is_empty() {
            return;
        }
        let styles = data
            .path(&["vectorData", "styleOverrideTable"])
            .map(Val::items)
            .unwrap_or_default();
        static NONE: Val = Val::Arr(Vec::new());
        let paints_for = |style: u32| -> &Val {
            if style != 0
                && let Some(entry) = styles
                    .iter()
                    .find(|e| e.get("styleID").and_then(Val::as_u32) == Some(style))
                && let Some(p) = entry.get(key)
            {
                return p;
            }
            data.get(key).unwrap_or(&NONE)
        };
        // Group consecutive paths that share a paint list, keeping draw order.
        let mut start = 0;
        while start < paths.len() {
            let paints = paints_for(paths[start].style);
            let mut end = start + 1;
            while end < paths.len() && std::ptr::eq(paints_for(paths[end].style), paints) {
                end += 1;
            }
            for p in paints.items() {
                let Some(resolved) =
                    paint::resolve(p, w, h, self.doc, self.images, &mut self.stats)
                else {
                    continue;
                };
                self.rc.set_transform(world);
                if let Some(clip) = resolved.clip {
                    self.rc.push_clip_layer(&clip.to_path(0.1));
                }
                self.rc.set_paint(resolved.paint);
                self.rc.set_paint_transform(resolved.transform);
                for g in &paths[start..end] {
                    self.rc.set_fill_rule(g.rule);
                    self.rc.fill_path(&g.path);
                }
                self.rc.set_paint_transform(Affine::IDENTITY);
                if resolved.clip.is_some() {
                    self.rc.pop_layer();
                }
            }
            start = end;
        }
    }

    /// Draws children, turning mask layers into clips for the siblings above them.
    fn draw_children(&mut self, node: &Node, data: &Val, world: Affine, path: Option<&[Guid]>) {
        let mut mask_layers = 0;
        let children = self.doc.children(node.id);
        for index in 0..children.len() {
            let id = &children[if data.flag("stackReverseZIndex") == Some(true) {
                children.len() - 1 - index
            } else {
                index
            }];
            if self.exhausted || !self.charge_work() {
                break;
            }
            let Some(child) = self.doc.node(*id) else {
                continue;
            };
            let child_path = path.map(|p| {
                let mut v = p.to_vec();
                v.push(child.override_key());
                v
            });
            let (mut child_data, stale_glyphs) = self.patched(child, child_path.as_deref());
            if path.is_some() && size(data) != size(&node.data) {
                layout::constrain(&child.data, child_data.to_mut(), &node.data, data);
            }
            if child_data.flag("mask") == Some(true) && visible(&child_data) {
                if !self.charge_work() {
                    break;
                }
                let tf = world * affine(child_data.get("transform"));
                let mut shapes = geometry(
                    &child_data,
                    "fillGeometry",
                    &self.doc.blobs,
                    &mut self.stats.unsupported_paints,
                );
                if shapes.is_empty() {
                    shapes.extend(base_shape(&child.kind, &child_data).map(GeomPath::plain));
                }
                let mut combined = BezPath::new();
                for g in shapes {
                    combined.extend(g.path);
                }
                if !combined.elements().is_empty() {
                    self.stats.masks += 1;
                    self.rc.set_transform(tf);
                    self.rc.push_clip_path(&combined);
                    self.rc.set_transform(world);
                    mask_layers += 1;
                }
                continue;
            }
            self.draw_with_data(
                child,
                world,
                child_path.as_deref(),
                Some((child_data, stale_glyphs)),
            );
        }
        for _ in 0..mask_layers {
            self.rc.pop_clip();
        }
    }

    fn charge_work(&mut self) -> bool {
        if self.exhausted || self.work_left == 0 {
            self.exhausted = true;
            self.stats.budget_exhausted += 1;
            false
        } else {
            self.work_left -= 1;
            true
        }
    }
    fn draw_instance(&mut self, data: &Val, world: Affine, path: Option<&[Guid]>) {
        self.stats.instances += 1;
        let symbol_id = data
            .get("overriddenSymbolID")
            .and_then(Val::as_guid)
            .or_else(|| {
                data.path(&["symbolData", "symbolID"])
                    .and_then(Val::as_guid)
            });
        let Some(symbol) = symbol_id.and_then(|id| self.doc.node(id)) else {
            self.stats.missing_symbols += 1;
            return;
        };
        if self.symbol_stack.contains(&symbol.id) {
            self.stats.depth_skipped += 1;
            return;
        }
        if self.symbol_stack.len() >= MAX_INSTANCE_DEPTH {
            self.stats.depth_skipped += 1;
            return;
        }
        if !self.charge_work() {
            return;
        }
        let base: Vec<Guid> = path.map(<[Guid]>::to_vec).unwrap_or_default();
        let owner_depth = self.symbol_stack.len();
        let before = self.patches.len();
        let mut added_keys = Vec::new();
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
                let index = self.patches.len();
                self.patch_index
                    .entry(full.clone())
                    .or_default()
                    .push(index);
                added_keys.push(full);
                self.patches.push(Patch {
                    data: item.clone(),
                    owner_depth,
                    derived,
                });
            }
        }
        self.symbol_stack.push(symbol.id);
        self.properties.push(symbol, data);
        let scale = data
            .path(&["symbolData", "uniformScaleFactor"])
            .and_then(Val::as_f32)
            .filter(|s| s.is_finite() && *s > 0.0)
            .map_or(1.0, f64::from);
        self.instance_scales.push(scale);
        self.draw_children(symbol, &symbol.data, world, Some(&base));
        self.instance_scales.pop();
        self.properties.pop();
        self.symbol_stack.pop();
        self.patches.truncate(before);
        for key in added_keys {
            let remove = self.patch_index.get_mut(&key).is_some_and(|indexes| {
                indexes.pop();
                indexes.is_empty()
            });
            if remove {
                self.patch_index.remove(&key);
            }
        }
    }
}
