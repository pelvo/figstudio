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

use crate::geom::{GeomPath, size};
use crate::{Ctx, effect_reach};

/// Largest device area (px²) of one shadow layer: the node's rendered bounds plus the blur
/// reach, intersected with the render target (Vello never allocates more than the target plus
/// filter padding). 8M px is 32 MiB of RGBA per layer; it admits the 6M px targets of the
/// 42-page file at scale 0.1 (4M refused them: counters 2922 vs 2233) and refuses anything
/// that needs a bigger target. Where the extent of a node's content is uncertain the whole
/// target stands in, so such nodes fall back, counted, whenever the target is above this.
pub(crate) const MAX_SHADOW_PIXELS: f64 = 8_000_000.0;
/// Most nodes (instances count their symbol's) in a subtree that is redrawn per extra shadow.
/// Chosen by measurement on the 42-page file (CPU time, quiet machine, gate +10% total and
/// +25% per page): 512 made one page +31-45% (+1.2 s) for ~250 fewer counted shadows, 128
/// kept the worst page near +6% and the total near +2%.
pub(crate) const MAX_REPLAY_DESCENDANTS: usize = 128;
/// Most drop shadows per node cast from the whole node; every one after the first redraws the
/// content once, so this bounds the redraw factor. Nodes in the file have at most 3.
pub(crate) const MAX_WHOLE_SHADOWS: usize = 4;
/// Runtime cap on the work all redraws of one node (nested ones included) may use, as a
/// multiple of its stored subtree size per whole-node shadow. Overrides can make a node draw
/// more than it stores, so the plan is only a hint; this cap is the bound. Once spent, the
/// remaining whole-node shadows of the node fall back and are counted.
const REPLAY_WORK_FACTOR: usize = 4;
/// Nodes visited (and charged to the work budget) when bounding a node's rendered alpha; past
/// it the whole target stands in.
pub(crate) const MAX_BOUNDS_VISITS: usize = 256;

/// The node's drop shadows that are cast from the whole node, and the bounds of what it draws.
pub(crate) struct ShadowPlan<'d> {
    pub whole: Vec<&'d Val>,
    /// Work units the redraws of this node may use (see `REPLAY_WORK_FACTOR`).
    pub replay_budget: usize,
    /// Output bounds of the node's rendered alpha (the whole target when unknown).
    pub bounds: Rect,
}

impl Ctx<'_> {
    /// Plans the visible drop shadows: those cast from the whole node, or none when the layer
    /// is too big or a redraw too costly; the others (spread...) take the old counted path,
    /// where a whole-content layer is only allowed if `bounds` fits (`layer_fits`).
    pub(crate) fn whole_shadow_plan<'d>(
        &mut self,
        node: &Node,
        data: &'d Val,
        world: Affine,
        (casts, wanted): (bool, bool),
    ) -> ShadowPlan<'d> {
        let visible: Vec<&Val> = data
            .get("effects")
            .map(Val::items)
            .unwrap_or_default()
            .iter()
            .filter(|e| e.flag("visible") != Some(false))
            .filter(|e| e.text("type") == Some("DROP_SHADOW"))
            .collect();
        if visible.is_empty() {
            return ShadowPlan {
                whole: Vec::new(),
                replay_budget: 0,
                bounds: self.screen,
            };
        }
        // Inside a redraw whose budget is spent nothing more is replayed, so do not plan
        // (planning walks the subtree and would make deep nests quadratic per redraw).
        let spent = self.work_left <= self.replay_floor;
        let bounds = if spent {
            self.screen
        } else {
            self.alpha_bounds(node, data, world)
        };
        let none = |bounds| ShadowPlan {
            whole: Vec::new(),
            replay_budget: 0,
            bounds,
        };
        if !wanted || spent {
            return none(bounds);
        }
        let drops: Vec<&Val> = visible
            .iter()
            .copied()
            .filter(|e| supported(e, world) && self.layer_fits(e, world, bounds))
            .take(MAX_WHOLE_SHADOWS + 1)
            .collect();
        // Without a fill the old path has no mask to draw the rest from: keep it for all.
        if drops.is_empty()
            || (!casts && drops.len() != visible.len())
            || drops.len() > MAX_WHOLE_SHADOWS
        {
            return none(bounds);
        }
        let redraws = drops.len() > 1 || drops.len() < visible.len();
        let mut replay_budget = 0;
        if redraws {
            let Some(count) = self.small_subtree(node) else {
                return none(bounds);
            };
            replay_budget = REPLAY_WORK_FACTOR * count * drops.len();
        }
        ShadowPlan {
            whole: drops,
            replay_budget,
            bounds,
        }
    }

    /// Draws the drop shadows behind the node, in list order, and opens the one layer a lone
    /// whole-node shadow needs (returns 1: the caller closes it).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_behind_shadows(
        &mut self,
        behind: &[&Val],
        (whole, replay_budget): (&[&Val], usize),
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
        // Redraws of this node (and of nodes inside them) stop at this work level.
        let outer_floor = self.replay_floor;
        self.replay_floor = outer_floor.max(self.work_left.saturating_sub(replay_budget));
        for effect in behind {
            if !is_whole(effect) {
                self.draw_drop_shadow(effect, fill_paths, world);
            } else if self.work_left > self.replay_floor && self.charge_work() {
                self.replay_shadow(effect, node, data, world, fill_paths, path, inner);
            } else if fill_paths.is_empty() {
                self.stats.skipped_effects += 1;
            } else {
                self.stats.approximated_effects += 1;
                self.draw_drop_shadow(effect, fill_paths, world);
            }
        }
        self.replay_floor = outer_floor;
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

    /// Whether a layer around `bounds` (plus the effect's reach) is affordable. Vello allocates
    /// at most the part of the layer inside the target (plus the filter padding), so the area
    /// is taken inside the target: when the target itself is small, every layer fits.
    pub(crate) fn layer_fits(&self, effect: &Val, world: Affine, bounds: Rect) -> bool {
        let [a, b, c, d, _, _] = world.as_coeffs();
        let reach = effect_reach(effect) * a.hypot(b).max(c.hypot(d));
        let bounds = bounds.inflate(reach, reach);
        let area = bounds.intersect(self.screen.inflate(reach, reach));
        let pixels = area.width().max(0.0) * area.height().max(0.0);
        bounds.width().is_finite() && bounds.height().is_finite() && pixels <= MAX_SHADOW_PIXELS
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
