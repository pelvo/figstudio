//! Representative paint colours for label contrast, not document rendering.
use figstudio_fig::Val;
use figstudio_render::{
    color::{AlphaColor, Srgb},
    paint::color,
};

/// Equal-stop average in premultiplied colour, not a spatial gradient sample.
pub(super) fn color_for(paint: &Val) -> Option<AlphaColor<Srgb>> {
    let opacity = paint.num("opacity").unwrap_or(1.0);
    match paint.text("type").unwrap_or("SOLID") {
        "SOLID" => color(paint.get("color"), opacity),
        "GRADIENT_LINEAR" | "GRADIENT_RADIAL" | "GRADIENT_ANGULAR" | "GRADIENT_DIAMOND" => {
            let stops = paint.get("stops")?.items();
            let mut sum = [0.0; 4];
            let mut count = 0;
            for stop in stops {
                let Some(c) = color(stop.get("color"), opacity) else {
                    continue;
                };
                let [r, g, b, a] = c.components;
                for (total, value) in sum.iter_mut().zip([r * a, g * a, b * a, a]) {
                    *total += value;
                }
                count += 1;
            }
            if count == 0 {
                return None;
            }
            let [r, g, b, a] = sum;
            Some(AlphaColor::new(if a > 0.0 {
                [r / a, g / a, b / a, a / count as f32]
            } else {
                [0.0; 4]
            }))
        }
        _ => None,
    }
}
