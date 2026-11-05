//! Shader fills: Figma code shaders (`CUSTOM` paints) drawn by a [`ShaderFills`] host
//! (figstudio-shader runs their code on the GPU). Without one, or when it cannot draw a
//! paint, the paint is skipped and counted.

use std::sync::Arc;

use figstudio_fig::{Document, Val};
use vello_cpu::Pixmap;

/// What a shader host gives for a paint or effect.
#[derive(Clone, Debug)]
pub enum Shaded {
    /// The pixels the shader drew.
    Drawn(Arc<Pixmap>),
    /// Being drawn (asynchronously, in the browser): drawn later, counted as loading.
    Pending,
    /// The shader cannot run here (no code, no GPU, it failed).
    Unavailable,
}

impl Shaded {
    /// `Drawn` when `Some`.
    pub fn from_option(pixmap: Option<Arc<Pixmap>>) -> Self {
        pixmap.map_or(Shaded::Unavailable, Shaded::Drawn)
    }
}

/// Draws shader paints and effects.
pub trait ShaderFills: Send + Sync {
    /// The pixels the `CUSTOM` paint `paint` draws over `width × height` pixels at `time_ms`
    /// on the playback clock, or `None` when its shader cannot run here.
    fn fill(&self, doc: &Document, paint: &Val, width: u16, height: u16, time_ms: f64) -> Shaded;

    /// Whether the `CUSTOM` effect `effect` has a shader to run (checked before the layer is
    /// drawn for it).
    fn has_effect(&self, doc: &Document, effect: &Val) -> bool;

    /// The `CUSTOM` effect `effect` applied to `input` (the layer drawn without it,
    /// premultiplied), or `None` when its shader cannot run.
    fn effect(&self, doc: &Document, effect: &Val, input: &Pixmap, time_ms: f64) -> Shaded;
}

/// Most pixels a side of a shader fill is drawn at.
pub const MAX_SIDE: f64 = 2048.0;

/// Pixel size a shader draws a `w × h` node-unit layer at: one pixel per unit, whatever the
/// zoom or export scale, as Figma runs them (pixel-sized patterns such as halftone dots keep
/// their size: Figma's 2× export of a halftone layer has dots twice as big as a 2× run).
/// Both sides shrink together past [`MAX_SIDE`]: shaders read the output's aspect ratio.
pub(crate) fn pixel_size(w: f64, h: f64) -> (u16, u16) {
    let mut scale = 1.0;
    let longest = w.max(h) * scale;
    if longest > MAX_SIDE {
        scale *= MAX_SIDE / longest;
    }
    let side = |v: f64| (v * scale).round().clamp(1.0, MAX_SIDE) as u16;
    (side(w), side(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_big_fill_keeps_its_aspect_ratio() {
        let (w, h) = pixel_size(3160.0, 1870.0);
        assert_eq!(w, 2048);
        assert!(
            (f64::from(w) / f64::from(h) - 3160.0 / 1870.0).abs() < 2e-3,
            "{w}x{h}"
        );
        // One pixel per layer unit, at any zoom.
        assert_eq!(pixel_size(100.0, 50.0), (100, 50));
        assert_eq!(pixel_size(100.4, 49.6), (100, 50));
    }
}
