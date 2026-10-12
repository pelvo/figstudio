//! Uniformly scaled instances (`symbolData.uniformScaleFactor`): lengths a child inherits from
//! its component are multiplied by every scaled instance outside the level that states them.
//! The renderer and the resolver both scale with [`scale_inherited`], so they agree.
use crate::{Obj, Val};

/// Stroke weight fields scaled with the instance, each by its own owner.
pub const STROKE_LENGTHS: [&str; 5] = [
    "strokeWeight",
    "borderTopWeight",
    "borderRightWeight",
    "borderBottomWeight",
    "borderLeftWeight",
];

/// An instance's `uniformScaleFactor`, 1 when missing or invalid.
pub fn uniform_scale(data: &Val) -> f64 {
    data.path(&["symbolData", "uniformScaleFactor"])
        .and_then(Val::as_f32)
        .filter(|s| s.is_finite() && *s > 0.0)
        .map_or(1.0, f64::from)
}

/// Scales the translation (`outer_scale("transform")`), size (`"size"`), each stroke weight
/// and the stored glyphs (`"derivedTextData"`). `outer_scale(field)` is the product of the
/// uniform scales outside the level whose override states `field` (see
/// [`super::Overrides::outer_scale`]).
pub fn scale_inherited(data: &mut Val, outer_scale: impl Fn(&str) -> f64) {
    let move_by = outer_scale("transform");
    if move_by != 1.0 {
        let m = |k: &str| {
            data.path(&["transform", k])
                .and_then(Val::as_f32)
                .map_or(0.0, f64::from)
        };
        let (x, y) = (m("m02"), m("m12"));
        let mut transform = data
            .get("transform")
            .cloned()
            .unwrap_or(Val::Obj(Obj::new()));
        transform.set("m02", Val::Float(x * move_by));
        transform.set("m12", Val::Float(y * move_by));
        data.set("transform", transform);
    }
    let grow_by = outer_scale("size");
    if grow_by != 1.0
        && let Some(mut grown) = data.get("size").cloned()
    {
        let axis = |k: &str| grown.num(k).map_or(0.0, |v| f64::from(v).max(0.0));
        let (w, h) = (axis("x"), axis("y"));
        grown.set("x", Val::Float(w * grow_by));
        grown.set("y", Val::Float(h * grow_by));
        data.set("size", grown);
    }
    for key in STROKE_LENGTHS {
        let factor = outer_scale(key);
        if factor != 1.0
            && let Some(value) = data.num(key)
        {
            data.set(key, Val::Float(f64::from(value) * factor));
        }
    }
    let glyph_scale = outer_scale("derivedTextData");
    if glyph_scale != 1.0
        && let Some(Val::Arr(glyphs)) = data
            .get_mut("derivedTextData")
            .and_then(|d| d.get_mut("glyphs"))
    {
        for glyph in glyphs {
            let font_size = f64::from(glyph.num("fontSize").unwrap_or(12.0));
            glyph.set("fontSize", Val::Float(font_size * glyph_scale));
            if let Some(position) = glyph.get_mut("position") {
                for key in ["x", "y"] {
                    let value = f64::from(position.num(key).unwrap_or(0.0));
                    position.set(key, Val::Float(value * glyph_scale));
                }
            }
        }
    }
}
