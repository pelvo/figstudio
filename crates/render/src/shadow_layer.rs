//! Drop shadows cast from the whole rendered node (fills, strokes and children), used when
//! the fill shape alone is not the node's silhouette.
//!
//! One shadow alone wraps the content in a single `DropShadow` layer. Otherwise (several
//! shadows, or a mix with fill-mask shadows) every drop shadow is drawn behind in list order:
//! a whole-node one as a `DropShadowOnly` layer around a redraw of the content (with the same
//! inner shadows), a mask one from the fill; then the real content once. A shadow is never
//! cast from another shadow. Spread, non-normal blend
//! and anisotropic blur stay on the older fill-mask path (vello_cpu 0.3 has no morphology
//! filter) and are counted there.

use figstudio_fig::{Guid, Node, Val};
use vello_cpu::filter_effects::{EdgeMode, Filter, FilterPrimitive};
use vello_cpu::kurbo::{Affine, Rect};

use crate::geom::{GeomPath, affine, size, transformed_bounds};
use crate::{Ctx, clips_content, effect_reach, overhang, visible};

/// Largest device area (px²) of one shadow layer, inflated by the blur reach and clipped to the
/// screen: about 16 MB of RGBA per layer. Never reached on the 42-page measurement file, so it
/// is a memory bound, not a speed one; larger layers take the counted fill-mask path.
pub(crate) const MAX_SHADOW_PIXELS: f64 = 4_000_000.0;
/// Most node draws (instances count their symbol's nodes; nested replays multiply) that one
/// replay of a node may cost when it needs per-shadow redraws. Chosen by measurement on a
/// 42-page file: 64 cleared 2308 approximated shadows at +0.3% CPU, 512 cleared 2084 at +0.4%
/// total and +12% on the worst page (gate: +10% total, +25% per page).
pub(crate) const MAX_REPLAY_DESCENDANTS: usize = 512;
/// Most drop shadows per node cast from the whole node; every one after the first redraws the
/// content once, so this bounds the redraw factor. Nodes in the file have at most 3.
pub(crate) const MAX_WHOLE_SHADOWS: usize = 4;
/// Nodes visited when bounding a node's rendered alpha; past it the screen stands in.
const MAX_BOUNDS_VISITS: usize = 4096;

impl Ctx<'_> {
    /// The node's visible drop shadows that can be cast from the whole node, or none when the
    /// layer is too big or the redraws too costly; the others (spread...) take the old counted
    /// path. The answer depends only on the node, so a replay plans like the real pass.
    pub(crate) fn whole_shadow_plan<'d>(
        &self,
        id: Guid,
        data: &'d Val,
        world: Affine,
        (casts, wanted): (bool, bool),
    ) -> Vec<&'d Val> {
        if !wanted {
            return Vec::new();
        }
        let visible: Vec<&Val> = data
            .get("effects")
            .map(Val::items)
            .unwrap_or_default()
            .iter()
            .filter(|e| e.flag("visible") != Some(false))
            .filter(|e| e.text("type") == Some("DROP_SHADOW"))
            .collect();
        let supported: Vec<&Val> = visible
            .iter()
            .copied()
            .filter(|e| supported(e, world))
            .collect();
        if supported.is_empty() {
            return Vec::new();
        }
        let bounds = self.alpha_bounds(id, data, world);
        let drops: Vec<&Val> = supported
            .into_iter()
            .filter(|e| self.layer_fits(e, world, bounds))
            .take(MAX_WHOLE_SHADOWS + 1)
            .collect();
        // Without a fill the old path has no mask to draw the rest from: keep it for all.
        if (!casts && drops.len() != visible.len()) || drops.len() > MAX_WHOLE_SHADOWS {
            return Vec::new();
        }
        let redraws = drops.len() > 1 || drops.len() < visible.len();
        if redraws && self.draw_cost(id, 0, true) > MAX_REPLAY_DESCENDANTS {
            return Vec::new();
        }
        drops
    }

    /// Draws the drop shadows behind the node, in list order, and opens the one layer a lone
    /// whole-node shadow needs (returns 1: the caller closes it).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_behind_shadows(
        &mut self,
        behind: &[&Val],
        whole: &[&Val],
        node: &Node,
        data: &Val,
        world: Affine,
        fill_paths: &[GeomPath],
        path: Option<&[Guid]>,
        inner: &[&Val],
    ) -> usize {
        let is_whole = |e: &Val| whole.iter().any(|w| std::ptr::eq(*w, e));
        if let [one] = behind
            && is_whole(one)
        {
            self.push_shadow(one, world, false);
            return 1;
        }
        for effect in behind {
            if !is_whole(effect) {
                self.draw_drop_shadow(effect, fill_paths, world);
            } else if self.charge_work() {
                self.replay_shadow(effect, node, data, world, fill_paths, path, inner);
            } else {
                self.stats.skipped_effects += 1;
            }
        }
        0
    }

    /// One shadow-only layer around a redraw of the content. The redraw's counters and image
    /// requests are discarded: the real pass draws and counts the same nodes.
    #[allow(clippy::too_many_arguments)]
    fn replay_shadow(
        &mut self,
        effect: &Val,
        node: &Node,
        data: &Val,
        world: Affine,
        fill_paths: &[GeomPath],
        path: Option<&[Guid]>,
        inner: &[&Val],
    ) {
        let before = self.stats.clone();
        let wanted = self.wanted.len();
        self.push_shadow(effect, world, true);
        self.draw_body(node, data, world, size(data), fill_paths, path, inner);
        self.rc.pop_layer();
        let spent = self
            .stats
            .budget_exhausted
            .saturating_sub(before.budget_exhausted);
        self.stats = before;
        self.stats.budget_exhausted += spent;
        self.stats.shadows += 1;
        self.wanted.truncate(wanted);
    }

    fn push_shadow(&mut self, effect: &Val, world: Affine, only: bool) {
        let (dx, dy, std_deviation, color) = self.shadow_params(effect);
        let edge_mode = EdgeMode::None;
        let primitive = if only {
            FilterPrimitive::DropShadowOnly {
                dx,
                dy,
                std_deviation,
                color,
                edge_mode,
            }
        } else {
            self.stats.shadows += 1;
            FilterPrimitive::DropShadow {
                dx,
                dy,
                std_deviation,
                color,
                edge_mode,
            }
        };
        self.rc.set_transform(world);
        self.rc
            .push_filter_layer(Filter::from_primitive(primitive), effect_reach(effect));
    }

    /// Device area of the shadow layer (rendered bounds plus blur reach, on screen) is affordable.
    fn layer_fits(&self, effect: &Val, world: Affine, bounds: Rect) -> bool {
        let [a, b, c, d, _, _] = world.as_coeffs();
        let reach = effect_reach(effect) * a.hypot(b).max(c.hypot(d));
        let bounds = bounds.inflate(reach, reach);
        let area = bounds.intersect(self.screen.inflate(reach, reach));
        let pixels = area.width().max(0.0) * area.height().max(0.0);
        bounds.width().is_finite() && bounds.height().is_finite() && pixels <= MAX_SHADOW_PIXELS
    }

    /// Output bounds of everything the node draws: its box, strokes, and its descendants
    /// unless it clips them. Instances count their symbol's children at the instance origin
    /// (shortcut: overrides and resizing are ignored). Past `MAX_BOUNDS_VISITS` nodes the
    /// whole screen stands in, which only the area guard can then refuse.
    fn alpha_bounds(&self, id: Guid, data: &Val, world: Affine) -> Rect {
        let mut out: Option<Rect> = None;
        let mut stack = vec![(id, world, true)];
        let mut visits = 0;
        while let Some((id, world, root)) = stack.pop() {
            visits += 1;
            let Some(node) = self.doc.node(id) else {
                continue;
            };
            let d = if root { data } else { &node.data };
            if !root && !visible(d) {
                continue;
            }
            let world = if root {
                world
            } else {
                world * affine(d.get("transform"))
            };
            let (w, h) = size(d);
            let m = overhang(&node.kind, d, w, h);
            let mut bounds = transformed_bounds(world, Rect::new(-m, -m, w + m, h + m));
            let clips = clips_content(&node.kind, d);
            if clips {
                bounds = bounds.intersect(transformed_bounds(world, Rect::new(0.0, 0.0, w, h)));
            }
            out = Some(out.map_or(bounds, |o| o.union(bounds)));
            if visits > MAX_BOUNDS_VISITS {
                return self.screen;
            }
            if !clips {
                stack.extend(self.drawn_children(node).map(|c| (c, world, false)));
            }
        }
        out.unwrap_or(self.screen)
    }

    /// The nodes drawn inside `node`: its children, or its symbol's children for an instance.
    fn drawn_children(&self, node: &Node) -> impl Iterator<Item = Guid> + '_ {
        let symbol = (node.kind == "INSTANCE")
            .then(|| {
                node.data
                    .get("overriddenSymbolID")
                    .and_then(Val::as_guid)
                    .or_else(|| {
                        node.data
                            .path(&["symbolData", "symbolID"])
                            .and_then(Val::as_guid)
                    })
            })
            .flatten();
        self.doc
            .children(node.id)
            .iter()
            .chain(symbol.map_or(&[][..], |s| self.doc.children(s)))
            .copied()
    }

    /// Node draws needed to draw `id` once, counting the redraws of descendants that have
    /// several drop shadows (`replays`: the node's own too). Saturates past the limit.
    fn draw_cost(&self, id: Guid, depth: usize, replays: bool) -> usize {
        let cap = MAX_REPLAY_DESCENDANTS + 1;
        let Some(node) = self.doc.node(id) else {
            return 1;
        };
        if depth > 32 {
            return cap;
        }
        let mut cost = 1;
        for child in self.drawn_children(node) {
            cost += self.draw_cost(child, depth + 1, true);
            if cost >= cap {
                return cap;
            }
        }
        let drops = node
            .data
            .get("effects")
            .map(Val::items)
            .unwrap_or_default()
            .iter()
            .filter(|e| e.flag("visible") != Some(false) && e.text("type") == Some("DROP_SHADOW"))
            .count();
        if replays && drops > 1 {
            cost = cost.saturating_mul(drops + 1);
        }
        cost.min(cap)
    }
}

/// Zero spread, normal blend, finite numbers and (for a blur) a uniform scale.
fn supported(effect: &Val, world: Affine) -> bool {
    let [a, b, c, d, _, _] = world.as_coeffs();
    let (x, y) = (a.hypot(b), c.hypot(d));
    let radius = effect.num("radius").unwrap_or(0.0);
    let offset = effect.get("offset");
    let finite = [radius]
        .into_iter()
        .chain(offset.and_then(|o| o.num("x")))
        .chain(offset.and_then(|o| o.num("y")))
        .all(f32::is_finite);
    finite
        && radius >= 0.0
        && effect.num("spread").unwrap_or(0.0) == 0.0
        && effect.text("blendMode").unwrap_or("NORMAL") == "NORMAL"
        && !(radius > 0.0
            && ((x - y).abs() > 1e-6 * x.max(y) || (a * c + b * d).abs() > 1e-6 * x * y))
}
