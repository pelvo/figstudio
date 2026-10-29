//! Shader fills: Figma code shaders (`CUSTOM` paints) drawn by a [`ShaderFills`] host
//! (figstudio-shader runs their code on the GPU). Without one, or when it cannot draw a
//! paint, the paint is skipped and counted.

use std::sync::Arc;

use figstudio_fig::{Document, Val};
use vello_cpu::Pixmap;

/// Draws shader paints.
pub trait ShaderFills: Send + Sync {
    /// The premultiplied pixels the `CUSTOM` paint `paint` draws over `width × height`
    /// pixels, or `None` when its shader cannot run here.
    fn fill(&self, doc: &Document, paint: &Val, width: u16, height: u16) -> Option<Arc<Pixmap>>;
}

/// Most pixels a side of a shader fill is drawn at.
pub const MAX_SIDE: f64 = 2048.0;

/// Pixel size to draw a `w × h` node-unit fill at `screen` pixels per unit: the scale is
/// rounded up to a power of two, so zooming re-runs a shader only every doubling. Both
/// sides shrink together past [`MAX_SIDE`]: shaders read the output's aspect ratio.
pub(crate) fn pixel_size(w: f64, h: f64, screen: f64) -> (u16, u16) {
    let mut scale = screen.max(1.0 / 64.0).log2().ceil().exp2();
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
        let (w, h) = pixel_size(1580.0, 935.0, 2.0);
        assert_eq!(w, 2048);
        assert!(
            (f64::from(w) / f64::from(h) - 1580.0 / 935.0).abs() < 2e-3,
            "{w}x{h}"
        );
        assert_eq!(pixel_size(100.0, 50.0, 1.0), (100, 50));
        assert_eq!(pixel_size(100.0, 50.0, 0.7), (100, 50));
    }
}
