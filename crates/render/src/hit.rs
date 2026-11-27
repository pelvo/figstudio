//! Hit-testing for page walks and resolved-instance picking.
//!
//! Raw [`hit_test`] and [`hit_chain`] preserve the legacy rectangular box walk.
//! Options-bearing [`hit_chain_with_options`], [`hits_node`], and [`hits_layer`] test painted
//! geometry for path-bearing arrow/line layers (`CONNECTOR`, `LINE`, `VECTOR`) and pass empty
//! interior clicks through nonpainting group containers (`GROUP`, `FRAME` with
//! `resizeToFit: true`). Primitive shapes, ordinary frames, text, instances and booleans retain
//! box hit-testing.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};

use figstudio_fig::{Document, Guid, Val};
use vello_cpu::kurbo::{
    Affine, BezPath, ParamCurve, ParamCurveNearest, PathSeg, Point, Rect, Shape,
};
use vello_cpu::peniko::Fill;

use crate::connector::{self, PartRef};
use crate::geom::{GeomPath, affine, clips_content, closed_network, geometry, size};
use crate::{children_topmost_first, clips_hits, network, stroke, visible};

const MAX_HIT_DEPTH: usize = 128;
const NEAREST_ACCURACY: f64 = 0.1;

/// Options for geometry hit-testing via [`hit_chain_with_options`], [`hits_node`], and
/// [`hits_layer`]. Default is exact geometry (`tolerance = 0.0`, `skip_locked = false`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitOptions {
    /// Extra page-space pointer slop around thin strokes (e.g. `4.0 / zoom`).
    pub tolerance: f64,
    /// When true, `locked: true` layers (and their subtrees) pass pointer hits through.
    pub skip_locked: bool,
}

impl HitOptions {
    fn normalized(self) -> Option<Self> {
        self.tolerance.is_finite().then_some(Self {
            tolerance: self.tolerance.max(0.0),
            skip_locked: self.skip_locked,
        })
    }
}

/// Active rectangular clip ancestor carried down a hit walk so stroke tolerance cannot steal
/// clicks from strokes that are completely clipped out of view.
#[derive(Clone, Copy, Debug)]
pub struct HitClip {
    inv: Affine,
    w: f64,
    h: f64,
}

impl HitClip {
    /// Builds a clip box for `transform` and `data`, or `None` when the transform is singular.
    pub fn new(transform: Affine, data: &Val) -> Option<Self> {
        let det = transform.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let (w, h) = size(data);
        Some(Self {
            inv: transform.inverse(),
            w,
            h,
        })
    }

    fn contains(&self, point: Point) -> bool {
        let local = self.inv * point;
        local.is_finite()
            && local.x >= 0.0
            && local.y >= 0.0
            && local.x <= self.w
            && local.y <= self.h
    }
}

/// Topmost, deepest visible node under `point` (page coordinates) using rectangular box
/// hit-testing. Instances count as one layer.
pub fn hit_test(doc: &Document, page: Guid, point: Point) -> Option<Guid> {
    hit_chain(doc, page, point).last().copied()
}

/// Topmost hit's ancestry, from the page's child to the deepest visible layer, using
/// rectangular box hit-testing (legacy contract; locked layers are not skipped).
/// Instances remain leaves; sections are traversed but never selected themselves.
pub fn hit_chain(doc: &Document, page: Guid, point: Point) -> Vec<Guid> {
    if !point.is_finite() {
        return Vec::new();
    }
    Walk::new(doc, WalkMode::Box, point)
        .run(page, Affine::IDENTITY, 0)
        .unwrap_or_default()
}

/// Like [`hit_chain`], using geometry hit-testing for path/connector/group layers with explicit
/// stroke hit tolerance and locked-layer handling.
pub fn hit_chain_with_options(
    doc: &Document,
    page: Guid,
    point: Point,
    options: HitOptions,
) -> Vec<Guid> {
    let Some(opts) = point.is_finite().then(|| options.normalized()).flatten() else {
        return Vec::new();
    };
    let styles = StyleLookup::new(doc);
    Walk::new(
        doc,
        WalkMode::Geometry {
            styles: &styles,
            options: opts,
        },
        point,
    )
    .run(page, Affine::IDENTITY, 0)
    .unwrap_or_default()
}

/// Whether hit-testing skips this layer (and its subtree): hidden, mask, or locked (when enabled).
pub fn hit_skippable(data: &Val, skip_locked: bool) -> bool {
    !visible(data)
        || data.flag("mask") == Some(true)
        || (skip_locked && data.flag("locked") == Some(true))
}

/// Whether `kind`/`data` is a hollow path, connector, or nonpainting group whose empty interior
/// passes clicks and body-move gizmos through.
pub fn is_hollow_layer(kind: &str, data: &Val) -> bool {
    matches!(kind, "CONNECTOR" | "LINE" | "VECTOR" | "GROUP")
        || (kind == "FRAME" && data.flag("resizeToFit") == Some(true))
}

/// Whether `point` hits `id` itself or (for hollow groups) any hit child inside `id`'s subtree.
pub fn hits_layer(
    doc: &Document,
    id: Guid,
    transform: Affine,
    point: Point,
    options: HitOptions,
) -> bool {
    let (Some(node), Some(opts)) = (
        doc.node(id),
        point.is_finite().then(|| options.normalized()).flatten(),
    ) else {
        return false;
    };
    let styles = StyleLookup::new(doc);
    if HitQuery::new(transform, &node.data, point, opts.tolerance)
        .is_some_and(|q| q.hits_node(&styles, &node.kind, &node.data, opts, &[]))
    {
        return true;
    }
    if node.kind == "GROUP" || (node.kind == "FRAME" && node.data.flag("resizeToFit") == Some(true))
    {
        return Walk::new(
            doc,
            WalkMode::Geometry {
                styles: &styles,
                options: opts,
            },
            point,
        )
        .run(id, transform, 0)
        .is_some();
    }
    false
}

/// Whether `point` (page coordinates) hits `kind`/`data`'s own surface or painted geometry
/// under `transform`, respecting active enclosing `clips` and `options`.
pub fn hits_node(
    doc: &Document,
    kind: &str,
    data: &Val,
    transform: Affine,
    point: Point,
    options: HitOptions,
    clips: &[HitClip],
) -> bool {
    let Some(opts) = point.is_finite().then(|| options.normalized()).flatten() else {
        return false;
    };
    let styles = StyleLookup::new(doc);
    HitQuery::new(transform, data, point, opts.tolerance)
        .is_some_and(|q| q.hits_node(&styles, kind, data, opts, clips))
}

#[derive(Clone, Copy)]
enum WalkMode<'a> {
    Box,
    Geometry {
        styles: &'a StyleLookup<'a>,
        options: HitOptions,
    },
}

struct Walk<'a> {
    doc: &'a Document,
    mode: WalkMode<'a>,
    point: Point,
    clips: Vec<HitClip>,
    seen: HashSet<Guid>,
}

impl<'a> Walk<'a> {
    fn new(doc: &'a Document, mode: WalkMode<'a>, point: Point) -> Self {
        Self {
            doc,
            mode,
            point,
            clips: Vec::new(),
            seen: HashSet::new(),
        }
    }

    fn run(&mut self, parent: Guid, parent_tf: Affine, depth: usize) -> Option<Vec<Guid>> {
        if depth >= MAX_HIT_DEPTH || !self.seen.insert(parent) {
            return None;
        }
        let data = &self.doc.node(parent)?.data;
        let (skip_locked, tolerance) = match self.mode {
            WalkMode::Box => (false, 0.0),
            WalkMode::Geometry { options, .. } => (options.skip_locked, options.tolerance),
        };
        for id in children_topmost_first(self.doc.children(parent), data) {
            let Some(node) = self.doc.node(id) else {
                continue;
            };
            if hit_skippable(&node.data, skip_locked) {
                continue;
            }
            let tf = parent_tf * affine(node.data.get("transform"));
            let Some(q) = HitQuery::new(tf, &node.data, self.point, tolerance) else {
                continue;
            };
            // Skip empty-child recursion (leaves) and only decode clip geometry outside the box.
            if node.kind != "INSTANCE"
                && !self.doc.children(id).is_empty()
                && (q.in_box() || !clips_hits(&node.kind, &node.data, &self.doc.blobs))
            {
                let pushed = if matches!(self.mode, WalkMode::Geometry { .. })
                    && clips_content(&node.kind, &node.data)
                    && let Some(clip) = HitClip::new(tf, &node.data)
                {
                    self.clips.push(clip);
                    true
                } else {
                    false
                };
                let child_hit = self.run(id, tf, depth + 1);
                if pushed {
                    self.clips.pop();
                }
                if let Some(mut hit) = child_hit {
                    if node.kind != "SECTION" {
                        hit.insert(0, id);
                    }
                    return Some(hit);
                }
            }
            let hit_self = match self.mode {
                WalkMode::Box => q.in_box() && node.kind != "SECTION",
                WalkMode::Geometry { styles, options } => {
                    q.hits_node(styles, &node.kind, &node.data, options, &self.clips)
                }
            };
            if hit_self {
                return Some(vec![id]);
            }
        }
        None
    }
}

/// Lazy document paint-style lookup matching `variables.rs`: O(1) `doc.node(guid)` for local
/// styles, and a one-time asset index only if a published `assetRef` style is encountered.
struct StyleLookup<'a> {
    doc: &'a Document,
    assets: OnceCell<HashMap<(&'a str, &'a str), &'a Val>>,
}

impl<'a> StyleLookup<'a> {
    fn new(doc: &'a Document) -> Self {
        Self {
            doc,
            assets: OnceCell::new(),
        }
    }

    fn style_paints(&self, reference: &Val) -> Option<&'a Val> {
        if let Some(guid) = reference
            .as_guid()
            .or_else(|| reference.get("guid").and_then(Val::as_guid))
        {
            let node = self.doc.node(guid)?;
            return node
                .data
                .text("styleType")
                .and_then(|_| node.data.get("fillPaints"));
        }
        let asset = reference.get("assetRef")?;
        let (key, version) = (asset.text("key")?, asset.text("version")?);
        let map = self.assets.get_or_init(|| {
            self.doc
                .nodes
                .iter()
                .filter_map(|n| {
                    n.data.text("styleType")?;
                    Some((
                        (n.data.text("key")?, n.data.text("version")?),
                        n.data.get("fillPaints")?,
                    ))
                })
                .collect()
        });
        map.get(&(key, version)).copied()
    }

    fn paints_from<'b>(&self, style_ref: Option<&Val>, raw: Option<&'b Val>) -> Option<&'b Val>
    where
        'a: 'b,
    {
        style_ref.and_then(|r| self.style_paints(r)).or(raw)
    }

    fn paints<'b>(&self, data: &'b Val, key: &str) -> Option<&'b Val>
    where
        'a: 'b,
    {
        let style_key = match key {
            "fillPaints" => "styleIdForFill",
            "strokePaints" => "styleIdForStrokeFill",
            _ => return data.get(key),
        };
        self.paints_from(data.get(style_key), data.get(key))
    }

    fn paints_for<'b>(&self, data: &'b Val, key: &str, style: u32) -> Option<&'b Val>
    where
        'a: 'b,
    {
        if style != 0
            && let Some(entry) = data
                .path(&["vectorData", "styleOverrideTable"])
                .map(Val::items)
                .unwrap_or_default()
                .iter()
                .find(|e| e.get("styleID").and_then(Val::as_u32) == Some(style))
            && let Some(p) = self.paints(entry, key)
        {
            return Some(p);
        }
        self.paints(data, key)
    }

    fn has_any_enabled_paint(&self, data: &Val, key: &str) -> bool {
        has_enabled_paint(self.paints(data, key))
            || data
                .path(&["vectorData", "styleOverrideTable"])
                .is_some_and(|t| {
                    t.items()
                        .iter()
                        .any(|e| has_enabled_paint(self.paints(e, key)))
                })
    }
}

#[derive(Clone, Copy)]
struct HitQuery {
    tf: Affine,
    local: Point,
    point: Point,
    w: f64,
    h: f64,
    tol: f64,
}

impl HitQuery {
    fn new(tf: Affine, data: &Val, point: Point, tol: f64) -> Option<Self> {
        let det = tf.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let local = tf.inverse() * point;
        if !local.is_finite() {
            return None;
        }
        let (w, h) = size(data);
        Some(Self {
            tf,
            local,
            point,
            w,
            h,
            tol,
        })
    }

    fn in_box(self) -> bool {
        self.local.x >= 0.0
            && self.local.y >= 0.0
            && self.local.x <= self.w
            && self.local.y <= self.h
    }

    fn sub_frame(self, parent: Affine, own: Option<&Val>) -> Option<(Affine, Point)> {
        if parent == Affine::IDENTITY && own.is_none() {
            return Some((self.tf, self.local));
        }
        let tf = self.tf * parent * affine(own);
        let det = tf.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let local = tf.inverse() * self.point;
        local.is_finite().then_some((tf, local))
    }

    fn hits_node(
        self,
        styles: &StyleLookup<'_>,
        kind: &str,
        data: &Val,
        options: HitOptions,
        clips: &[HitClip],
    ) -> bool {
        match kind {
            "SECTION" => false,
            "CONNECTOR" => self.hits_connector(styles, data, options, clips),
            "LINE" => self.hits_line(styles, data, clips),
            "VECTOR" => self.hits_vector(styles, data, clips),
            "GROUP" | "FRAME" if kind == "GROUP" || data.flag("resizeToFit") == Some(true) => {
                self.hits_group(styles, data, clips)
            }
            _ => self.in_box(),
        }
    }

    fn hits_connector(
        self,
        styles: &StyleLookup<'_>,
        data: &Val,
        options: HitOptions,
        clips: &[HitClip],
    ) -> bool {
        // Broad-phase using `derivedImmutableFrameData` alone (which holds part sizes/transforms),
        // deferring `nodeGenerationData` and any `Val` cloning to candidates near `self.point`.
        let derived = connector::layer_slots(data, "derivedImmutableFrameData");
        let has_derived = derived.iter().any(Option::is_some);
        let generated_cell: OnceCell<[Option<&Val>; 3]> = OnceCell::new();
        let generated =
            || *generated_cell.get_or_init(|| connector::layer_slots(data, "nodeGenerationData"));
        let mut label_tf_cache: Option<Affine> = None;

        for slot in 0..3 {
            let d = derived[slot];
            if d.is_none() && (has_derived || generated()[slot].is_none()) {
                continue;
            }
            let parent = if slot == 2 {
                *label_tf_cache.get_or_insert_with(|| {
                    let lb_tf = derived[1]
                        .and_then(|o| o.get("transform"))
                        .or_else(|| generated()[1].and_then(|o| o.get("transform")));
                    affine(lb_tf)
                })
            } else {
                Affine::IDENTITY
            };
            // Fast broad-phase on derived alone when present.
            if let Some(d) = d {
                let Some((tf, local)) = self.sub_frame(parent, d.get("transform")) else {
                    continue;
                };
                let (w, h) = size(d);
                let near = match slot {
                    0 => {
                        (w <= 0.0 && h <= 0.0)
                            || within_envelope(tf, local, w, h, stroke_weight(d).max(4.0), self.tol)
                    }
                    _ => local.x >= 0.0 && local.y >= 0.0 && local.x <= w && local.y <= h,
                };
                if !near {
                    continue;
                }
            }

            let part = PartRef {
                generated: generated()[slot],
                derived: d,
            };
            if !part.is_non_empty()
                || part.flag("visible") == Some(false)
                || part.flag("mask") == Some(true)
                || (options.skip_locked && part.flag("locked") == Some(true))
            {
                continue;
            }
            let Some((part_tf, local)) = self.sub_frame(parent, part.get("transform")) else {
                continue;
            };
            let (w, h) = part.get("size").map_or((0.0, 0.0), |s| {
                let get = |k| s.num(k).map_or(0.0, |v| f64::from(v).max(0.0));
                (get("x"), get("y"))
            });
            let in_box = local.x >= 0.0 && local.y >= 0.0 && local.x <= w && local.y <= h;
            match slot {
                0 => {
                    let part_val = part.to_val();
                    let q_part = HitQuery {
                        tf: part_tf,
                        local,
                        point: self.point,
                        w,
                        h,
                        tol: options.tolerance,
                    };
                    if q_part.hits_vector(styles, &part_val, clips) {
                        return true;
                    }
                }
                1 => {
                    if !in_box || (w <= 0.0 && h <= 0.0) {
                        continue;
                    }
                    let fill =
                        styles.paints_from(part.get("styleIdForFill"), part.get("fillPaints"));
                    let stroke = styles
                        .paints_from(part.get("styleIdForStrokeFill"), part.get("strokePaints"));
                    if (fill.is_none() && stroke.is_none())
                        || has_enabled_paint(fill)
                        || has_enabled_paint(stroke)
                    {
                        return true;
                    }
                }
                _ => {
                    if in_box && (w > 0.0 || h > 0.0) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn hits_group(self, styles: &StyleLookup<'_>, data: &Val, clips: &[HitClip]) -> bool {
        if has_enabled_paint(styles.paints(data, "fillPaints")) && self.in_box() {
            return true;
        }
        if has_enabled_paint(styles.paints(data, "strokePaints")) {
            let mut ignored = 0;
            let stroke_paths = geometry(data, "strokeGeometry", &styles.doc.blobs, &mut ignored);
            if !stroke_paths.is_empty() {
                let half = stroke::max_weight(data) * 0.5;
                return stroke_paths
                    .iter()
                    .any(|g| self.stroke_geom_hit(g, half, clips));
            }
            if stroke::max_weight(data) > 0.0 && self.in_box() {
                return true;
            }
        }
        false
    }

    fn hits_line(self, styles: &StyleLookup<'_>, data: &Val, clips: &[HitClip]) -> bool {
        let weight = stroke_weight(data);
        if (self.w > 0.0 || self.h > 0.0)
            && !within_envelope(self.tf, self.local, self.w, self.h, weight, self.tol)
        {
            return false;
        }
        if !has_enabled_paint(styles.paints(data, "strokePaints")) {
            return false;
        }
        let mut ignored = 0;
        let stroke_paths = geometry(data, "strokeGeometry", &styles.doc.blobs, &mut ignored);
        let half = weight * 0.5;
        if !stroke_paths.is_empty() {
            return stroke_paths
                .iter()
                .any(|g| self.stroke_geom_hit(g, half, clips));
        }
        if half <= 0.0 || self.w <= 0.0 {
            return false;
        }
        // Exact local cap geometry matching `stroke::outline`: Butt (default), Square, or Round.
        let near_local = match data.text("strokeCap") {
            Some("ROUND") => {
                let cx = self.local.x.clamp(0.0, self.w);
                let (dx, dy) = (self.local.x - cx, self.local.y);
                let dist = dx.hypot(dy);
                if dist <= half {
                    return true;
                }
                Point::new(cx + dx * (half / dist), dy * (half / dist))
            }
            cap => {
                let cap_x = if cap == Some("SQUARE") { half } else { 0.0 };
                if self.local.x >= -cap_x
                    && self.local.x <= self.w + cap_x
                    && self.local.y.abs() <= half
                {
                    return true;
                }
                Point::new(
                    self.local.x.clamp(-cap_x, self.w + cap_x),
                    self.local.y.clamp(-half, half),
                )
            }
        };
        if self.tol <= 0.0 {
            return false;
        }
        let normal_scale = (self.tf * Point::new(0.0, 1.0) - self.tf * Point::ZERO).hypot();
        let extra = (self.tol - half * normal_scale).max(0.0);
        if extra <= 0.0 {
            return false;
        }
        let near_page = self.tf * near_local;
        (self.point - near_page).hypot() <= extra && clips_contain(clips, near_page)
    }

    fn hits_vector(self, styles: &StyleLookup<'_>, data: &Val, clips: &[HitClip]) -> bool {
        let weight = stroke_weight(data);
        if (self.w > 0.0 || self.h > 0.0)
            && !within_envelope(self.tf, self.local, self.w, self.h, weight, self.tol)
        {
            return false;
        }

        let blobs = &styles.doc.blobs;
        let has_fill_geom = matches!(data.get("fillGeometry"), Some(Val::Arr(_)));
        let has_stroke_geom = matches!(data.get("strokeGeometry"), Some(Val::Arr(_)));
        let has_network = data.path(&["vectorData", "vectorNetworkBlob"]).is_some();

        if !has_fill_geom && !has_stroke_geom && !has_network {
            let fill_enabled = styles.has_any_enabled_paint(data, "fillPaints");
            let stroke_enabled = styles.has_any_enabled_paint(data, "strokePaints");
            return !(stroke_enabled && !fill_enabled) && self.in_box();
        }

        let cached_fills: OnceCell<Vec<GeomPath>> = OnceCell::new();
        let fills = || -> &[GeomPath] {
            cached_fills.get_or_init(|| {
                let mut ignored = 0;
                let mut paths = geometry(data, "fillGeometry", blobs, &mut ignored);
                if paths.is_empty() && has_network {
                    paths = network::region_fills(data, blobs);
                }
                paths
            })
        };

        if styles.has_any_enabled_paint(data, "fillPaints") {
            for g in fills() {
                if has_enabled_paint(styles.paints_for(data, "fillPaints", g.style))
                    && winding_contains(&g.path, g.rule, self.local)
                {
                    return true;
                }
            }
        }

        if styles.has_any_enabled_paint(data, "strokePaints") {
            let mut ignored = 0;
            let stroke_paths = geometry(data, "strokeGeometry", blobs, &mut ignored);
            let half = weight * 0.5;
            if !stroke_paths.is_empty() {
                let align = data
                    .text("strokeAlign")
                    .filter(|a| *a == "INSIDE" || *a == "OUTSIDE");
                let aligned = align.and_then(|a| {
                    let f = fills();
                    (!f.is_empty() && closed_network(data, blobs)).then_some(a == "OUTSIDE")
                });
                if let Some(outside) = aligned {
                    let fills = fills();
                    let in_fill = fills
                        .iter()
                        .any(|f| winding_contains(&f.path, f.rule, self.local));
                    let extra = extra_page_tol(self.tf, half, self.tol);
                    for g in &stroke_paths {
                        if !has_enabled_paint(styles.paints_for(data, "strokePaints", g.style)) {
                            continue;
                        }
                        if (in_fill != outside) && winding_contains(&g.path, g.rule, self.local) {
                            return true;
                        }
                        if extra > 0.0
                            && fills
                                .iter()
                                .any(|f| self.page_distance_le(&f.path, extra, clips))
                        {
                            return true;
                        }
                    }
                } else {
                    for g in &stroke_paths {
                        if has_enabled_paint(styles.paints_for(data, "strokePaints", g.style))
                            && self.stroke_geom_hit(g, half, clips)
                        {
                            return true;
                        }
                    }
                }
            } else if has_network && half > 0.0 {
                return network::any_stroke_segment(data, blobs, |seg, style| {
                    has_enabled_paint(styles.paints_for(data, "strokePaints", style))
                        && self.segment_hit(seg, half, clips)
                });
            }
        }

        false
    }

    fn stroke_geom_hit(self, g: &GeomPath, half_stroke_local: f64, clips: &[HitClip]) -> bool {
        if winding_contains(&g.path, g.rule, self.local) {
            return true;
        }
        let extra = extra_page_tol(self.tf, half_stroke_local, self.tol);
        extra > 0.0 && self.page_distance_le(&g.path, extra, clips)
    }

    fn segment_hit(self, seg: PathSeg, half_stroke_local: f64, clips: &[HitClip]) -> bool {
        let local_pad = (self.tol / min_scale_bound(self.tf)).max(half_stroke_local);
        if !seg
            .bounding_box()
            .inflate(local_pad, local_pad)
            .contains(self.local)
        {
            return false;
        }
        let nearest = seg.nearest(self.local, NEAREST_ACCURACY);
        if nearest.distance_sq <= half_stroke_local * half_stroke_local {
            return true;
        }
        if self.tol <= 0.0 {
            return false;
        }
        let page_seg = self.tf * seg;
        if page_seg.nearest(self.point, NEAREST_ACCURACY).distance_sq > self.tol * self.tol {
            return false;
        }
        if clips.is_empty() {
            return true;
        }
        let near_local = seg.eval(nearest.t);
        let delta = self.local - near_local;
        let dist = delta.hypot();
        let edge_local = if dist > 1e-9 {
            near_local + delta * (half_stroke_local / dist).min(1.0)
        } else {
            near_local
        };
        clips_contain(clips, self.tf * edge_local)
    }

    fn page_distance_le(self, path: &BezPath, page_tol: f64, clips: &[HitClip]) -> bool {
        if path.elements().is_empty() || !path.is_finite() {
            return false;
        }
        let local_pad = page_tol / min_scale_bound(self.tf);
        if !path
            .bounding_box()
            .inflate(local_pad, local_pad)
            .contains(self.local)
        {
            return false;
        }
        let r2 = page_tol * page_tol;
        for seg in path.segments() {
            if !seg
                .bounding_box()
                .inflate(local_pad, local_pad)
                .contains(self.local)
            {
                continue;
            }
            let page_seg = self.tf * seg;
            let near = page_seg.nearest(self.point, NEAREST_ACCURACY);
            if near.distance_sq <= r2
                && (clips.is_empty() || clips_contain(clips, page_seg.eval(near.t)))
            {
                return true;
            }
        }
        false
    }
}

fn clips_contain(clips: &[HitClip], point: Point) -> bool {
    clips.iter().all(|c| c.contains(point))
}

fn stroke_weight(data: &Val) -> f64 {
    data.num("strokeWeight")
        .map(f64::from)
        .filter(|w| w.is_finite())
        .map_or(0.0, |w| w.max(0.0))
}

fn within_envelope(tf: Affine, local: Point, w: f64, h: f64, weight: f64, tol: f64) -> bool {
    let local_tol = tol / min_scale_bound(tf);
    let margin = (weight * 6.0).max(16.0) + local_tol;
    Rect::new(0.0, 0.0, w, h)
        .inflate(margin, margin)
        .contains(local)
}

/// Whether `paints` holds at least one paint not explicitly hidden (`visible != false`).
/// Matches Figma's rule that a 0%-opacity enabled fill still participates in canvas hit testing,
/// while an empty or hidden fill list is hollow.
fn has_enabled_paint(paints: Option<&Val>) -> bool {
    paints.is_some_and(|p| {
        p.items()
            .iter()
            .any(|item| item.flag("visible") != Some(false))
    })
}

fn min_scale_bound(tf: Affine) -> f64 {
    let [a, b, c, d, _, _] = tf.as_coeffs();
    let frob = (a * a + b * b + c * c + d * d).sqrt();
    if frob <= 1e-12 {
        return 1e-12;
    }
    (tf.determinant().abs() / frob).max(1e-12)
}

fn winding_contains(path: &BezPath, rule: Fill, local: Point) -> bool {
    if path.elements().is_empty() || !path.is_finite() || !path.bounding_box().contains(local) {
        return false;
    }
    let w = path.winding(local);
    match rule {
        Fill::NonZero => w != 0,
        Fill::EvenOdd => (w & 1) != 0,
    }
}

fn extra_page_tol(tf: Affine, half_stroke_local: f64, tol: f64) -> f64 {
    (tol - half_stroke_local * min_scale_bound(tf)).max(0.0)
}
