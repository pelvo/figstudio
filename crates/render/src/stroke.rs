//! Stroke outlines built from a node's stroke fields when no stored geometry applies:
//! join, cap and dash pattern, and independent per-side border weights.

use figstudio_fig::Val;
use vello_cpu::kurbo::{
    BezPath, Cap, Join, Rect, RoundedRect, RoundedRectRadii, Shape, Stroke, StrokeOpts,
};
use vello_cpu::peniko::Fill;

use crate::geom::{GeomPath, corner_radii, size};

/// Figma's miter limit (joins sharper than about 29 degrees are bevelled), as in SVG.
const MITER_LIMIT: f64 = 4.0;

/// Most dashes drawn along one outline. A tiny pattern on a long path would emit millions of
/// segments; past this the stroke is drawn solid and counted as approximated.
const MAX_DASHES: f64 = 10_000.0;

/// Fields whose change makes stored `strokeGeometry` stale.
pub(crate) const STROKE_SHAPE: [&str; 10] = [
    "strokeWeight",
    "strokeAlign",
    "strokeJoin",
    "strokeCap",
    "dashPattern",
    "borderStrokeWeightsIndependent",
    "borderTopWeight",
    "borderRightWeight",
    "borderBottomWeight",
    "borderLeftWeight",
];

const SIDES: [&str; 4] = [
    "borderTopWeight",
    "borderRightWeight",
    "borderBottomWeight",
    "borderLeftWeight",
];

/// Outline of `shape` stroked at `width` with the node's join, cap and dashes, plus whether
/// part of the style could not be drawn exactly (arrow caps, an oversized dash count).
pub(crate) fn outline(data: &Val, shape: &BezPath, width: f64) -> (BezPath, bool) {
    let join = match data.text("strokeJoin") {
        Some("ROUND") => Join::Round,
        Some("BEVEL") => Join::Bevel,
        _ => Join::Miter,
    };
    let (cap, mut approximated) = match data.text("strokeCap") {
        None | Some("NONE") => (Cap::Butt, false),
        Some("ROUND") => (Cap::Round, false),
        Some("SQUARE") => (Cap::Square, false),
        // shortcut: arrow and other line-end markers draw as butt caps; add them when a file needs it.
        Some(_) => (Cap::Butt, true),
    };
    let mut style = Stroke::new(width)
        .with_join(join)
        .with_miter_limit(MITER_LIMIT)
        .with_caps(cap);
    let dashes: Vec<f64> = data
        .get("dashPattern")
        .map(Val::items)
        .unwrap_or_default()
        .iter()
        .filter_map(Val::as_f32)
        .map(f64::from)
        .collect();
    let period: f64 = dashes.iter().sum();
    let valid = dashes.iter().all(|d| d.is_finite() && *d >= 0.0) && period > 0.0;
    if valid && shape.perimeter(0.1) / period <= MAX_DASHES {
        style = style.with_dashes(0.0, dashes);
    } else if !dashes.is_empty() {
        approximated = true;
    }
    let path = vello_cpu::kurbo::stroke(
        shape.path_elements(0.1),
        &style,
        &StrokeOpts::default(),
        0.1,
    );
    (path, approximated)
}

/// One side's weight; Figma omits a side whose weight is zero.
fn side(data: &Val, key: &str) -> f64 {
    data.num(key)
        .map(f64::from)
        .filter(|w| w.is_finite())
        .map_or(0.0, |w| w.max(0.0))
}

/// Largest weight any side of the node's stroke can have.
pub(crate) fn max_weight(data: &Val) -> f64 {
    SIDES
        .iter()
        .map(|k| side(data, k))
        .fold(side(data, "strokeWeight"), f64::max)
}

/// The border of a rectangle-like node with independent per-side weights, already placed
/// for its alignment (draw it without the inside/outside clip). `None` for uniform strokes.
pub(crate) fn sides(kind: &str, data: &Val) -> Option<GeomPath> {
    // Ellipses have no sides; Figma ignores per-side weights on them.
    if data.flag("borderStrokeWeightsIndependent") != Some(true) || kind == "ELLIPSE" {
        return None;
    }
    let [t, r, b, l] = SIDES.map(|k| side(data, k));
    // Share of each side's weight that lies outside the shape.
    let out = match data.text("strokeAlign") {
        Some("INSIDE") => 0.0,
        Some("OUTSIDE") => 1.0,
        _ => 0.5,
    };
    let inn = 1.0 - out;
    let (w, h) = size(data);
    let outer = Rect::new(-out * l, -out * t, w + out * r, h + out * b);
    let x0 = inn * l;
    let y0 = inn * t;
    let inner = Rect::new(x0, y0, (w - inn * r).max(x0), (h - inn * b).max(y0));
    let radii = corner_radii(data, w, h);
    // Each corner's radius grows by the outer part of its wider side and shrinks by the inner part.
    let corner = |radius: f64, a: f64, c: f64, share: f64, grow: bool| {
        if radius <= 0.0 {
            0.0
        } else if grow {
            radius + share * a.max(c)
        } else {
            (radius - share * a.max(c)).max(0.0)
        }
    };
    let ring = |rect: Rect, share: f64, grow: bool| {
        let radii = RoundedRectRadii::new(
            corner(radii.top_left, t, l, share, grow),
            corner(radii.top_right, t, r, share, grow),
            corner(radii.bottom_right, b, r, share, grow),
            corner(radii.bottom_left, b, l, share, grow),
        );
        RoundedRect::from_rect(rect, radii).to_path(0.1)
    };
    let mut path = ring(outer, out, true);
    path.extend(ring(inner, inn, false));
    Some(GeomPath {
        path,
        rule: Fill::EvenOdd,
        style: 0,
    })
}
