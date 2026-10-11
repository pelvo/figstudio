//! Accounting for whole-node shadow layers: their size (charged against the render's budget)
//! and how many are open at once (never more than one).

use figstudio_fig::Val;
use vello_cpu::kurbo::{Affine, Rect};

use crate::shadow_layer::{MAX_SHADOW_LAYER_PIXELS_TOTAL, MAX_SHADOW_PIXELS};
use crate::{Ctx, effect_reach};

impl Ctx<'_> {
    /// Notes one more open whole-node shadow layer (closed by `close_whole_layer`).
    pub(crate) fn open_whole_layer(&mut self) {
        self.whole_open += 1;
        self.stats.shadow_layer_depth_peak =
            self.stats.shadow_layer_depth_peak.max(self.whole_open);
    }

    pub(crate) fn close_whole_layer(&mut self) {
        self.whole_open -= 1;
    }

    /// Device pixels of a layer around `bounds` plus the effect's reach, clamped to the render
    /// target grown by that reach. Vello cannot allocate more per layer: a filter layer's strip
    /// generator is the target plus the filter's source padding (vello_common 0.3.0
    /// `viewport.rs:94-107`), so drawing is clipped to it, and its pixmap is that bbox plus the
    /// filter padding (`filter/mod.rs:156-157`, `geometry.rs:219` `expand`), tile-snapped. Both
    /// paddings are about the blur reach per side, so target + reach on every side is the same
    /// order (a few tile pixels of snapping aside). When the target is small, every layer is
    /// small. Huge for non-finite bounds.
    pub(crate) fn layer_pixels(&self, effect: &Val, world: Affine, bounds: Rect) -> usize {
        let [a, b, c, d, _, _] = world.as_coeffs();
        let reach = effect_reach(effect) * a.hypot(b).max(c.hypot(d));
        let bounds = bounds.inflate(reach, reach);
        let area = bounds.intersect(self.screen.inflate(reach, reach));
        let pixels = area.width().max(0.0) * area.height().max(0.0);
        if bounds.width().is_finite() && bounds.height().is_finite() && pixels.is_finite() {
            // Saturating float-to-int cast: a huge area is just "too big".
            pixels.ceil() as usize
        } else {
            usize::MAX / 4
        }
    }

    /// Whether one layer around `bounds` is under the per-layer cap.
    pub(crate) fn layer_fits(&self, effect: &Val, world: Affine, bounds: Rect) -> bool {
        self.layer_pixels(effect, world, bounds) as f64 <= MAX_SHADOW_PIXELS
    }

    /// `layer_fits`, and the layer is charged to the render's total (see
    /// `MAX_SHADOW_LAYER_PIXELS_TOTAL`); false, uncharged, when either cap would be exceeded.
    pub(crate) fn charge_layer(&mut self, effect: &Val, world: Affine, bounds: Rect) -> bool {
        let pixels = self.layer_pixels(effect, world, bounds);
        let fits = pixels as f64 <= MAX_SHADOW_PIXELS
            && self.stats.shadow_layer_pixels + pixels <= MAX_SHADOW_LAYER_PIXELS_TOTAL;
        if fits {
            self.stats.shadow_layer_pixels += pixels;
        }
        fits
    }
}
