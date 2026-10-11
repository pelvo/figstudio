//! Drop shadows cast from the whole rendered node (fills, strokes and children), used when
//! the fill shape alone is not the node's silhouette.
//!
//! One shadow wraps the content in a single `DropShadow` layer. Several shadows draw the
//! content once per shadow into a `DropShadowOnly` layer (behind, in list order) and then the
//! real content once, so a shadow is never cast from another shadow. Spread, non-normal blend
//! and anisotropic blur stay on the older fill-mask path (vello_cpu 0.3 has no morphology
//! filter) and are counted there.

use figstudio_fig::{Guid, Node, Val};
use vello_cpu::filter_effects::{EdgeMode, Filter, FilterPrimitive};
use vello_cpu::kurbo::{Affine, Rect};

use crate::geom::{GeomPath, transformed_bounds};
use crate::{Ctx, effect_reach, size};

/// Largest device area (px²) of one shadow layer, inflated by the blur reach and clipped to the
/// screen: about 16 MB of RGBA per layer. Never reached on the 42-page measurement file, so it
/// is a memory bound, not a speed one; larger layers take the counted fill-mask path.
pub(crate) const MAX_SHADOW_PIXELS: f64 = 4_000_000.0;
/// Most nodes (instances count their symbol's nodes) redrawn once per shadow when a node has
/// two or more shadows. Chosen by measurement on a 42-page file: 64 cleared 2308 approximated
/// shadows at +0.3% CPU, 512 cleared 2084 at +0.4% total and +12% on the worst page (gate:
/// +10% total, +25% per page).
pub(crate) const MAX_REPLAY_DESCENDANTS: usize = 512;
/// Most drop shadows per node cast from the whole node; every one after the first redraws the
/// content once, so this bounds the redraw factor. Nodes in the file have at most 3.
pub(crate) const MAX_WHOLE_SHADOWS: usize = 4;

impl Ctx<'_> {
    /// The node's visible drop shadows that can be cast from the whole node, or none when too
    /// many or too costly to redraw; the others (spread...) take the old counted path.
    pub(crate) fn whole_shadow_plan<'d>(
        &self,
        id: Guid,
        data: &'d Val,
        world: Affine,
        (w, h): (f64, f64),
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
        let drops: Vec<&Val> = visible
            .iter()
            .copied()
            .filter(|e| supported(e, world) && self.layer_fits(e, world, w, h))
            .take(MAX_WHOLE_SHADOWS + 1)
            .collect();
        // Without a fill the old path has no mask to draw the rest from: keep it for all.
        if !casts && drops.len() != visible.len() {
            return Vec::new();
        }
        let ok = drops.len() <= MAX_WHOLE_SHADOWS
            && (drops.len() < 2 || (self.replay_depth == 0 && self.replayable(id)));
        if ok { drops } else { Vec::new() }
    }

    /// Opens the single shadow layer (returns 1: the caller closes it) or draws the behind-layers
    /// of several shadows (returns 0; the caller then draws the real content).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_whole_shadows(
        &mut self,
        plan: &[&Val],
        node: &Node,
        data: &Val,
        world: Affine,
        fill_paths: &[GeomPath],
        path: Option<&[Guid]>,
    ) -> usize {
        match plan {
            [] => return 0,
            [one] => {
                self.push_shadow(one, world, false);
                return 1;
            }
            _ => {}
        }
        for effect in plan {
            if !self.charge_work() {
                self.stats.skipped_effects += 1;
                continue;
            }
            // The replay draws what the real pass draws again: keep its counters out.
            let before = self.stats.clone();
            let exhausted = before.budget_exhausted;
            let wanted = self.wanted.len();
            self.replay_depth += 1;
            self.push_shadow(effect, world, true);
            self.draw_body(node, data, world, size(data), fill_paths, path, &[]);
            self.rc.pop_layer();
            self.replay_depth -= 1;
            let spent = self.stats.budget_exhausted.saturating_sub(exhausted);
            self.stats = before;
            self.stats.budget_exhausted += spent;
            self.stats.shadows += 1;
            self.wanted.truncate(wanted);
        }
        0
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

    /// Device area of the shadow layer (node bounds plus blur reach, on screen) is affordable.
    fn layer_fits(&self, effect: &Val, world: Affine, w: f64, h: f64) -> bool {
        let [a, b, c, d, _, _] = world.as_coeffs();
        let reach = effect_reach(effect) * a.hypot(b).max(c.hypot(d));
        let bounds = transformed_bounds(world, Rect::new(0.0, 0.0, w, h)).inflate(reach, reach);
        let area = bounds.intersect(self.screen.inflate(reach, reach));
        let pixels = area.width().max(0.0) * area.height().max(0.0);
        bounds.width().is_finite() && bounds.height().is_finite() && pixels <= MAX_SHADOW_PIXELS
    }

    /// Redrawing is only affordable for small subtrees; instances count their symbol's nodes.
    fn replayable(&self, id: Guid) -> bool {
        let mut stack = vec![id];
        let mut count = 0;
        while let Some(id) = stack.pop() {
            let Some(node) = self.doc.node(id) else {
                return false;
            };
            count += 1;
            if count > MAX_REPLAY_DESCENDANTS {
                return false;
            }
            stack.extend_from_slice(self.doc.children(id));
            if node.kind == "INSTANCE" {
                let symbol = node
                    .data
                    .get("overriddenSymbolID")
                    .and_then(Val::as_guid)
                    .or_else(|| {
                        node.data
                            .path(&["symbolData", "symbolID"])
                            .and_then(Val::as_guid)
                    });
                stack.extend(symbol);
            }
        }
        true
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
