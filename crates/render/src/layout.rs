//! Stored derived layout is authoritative; constraints fill in omitted translations.
use figstudio_fig::Val;

use crate::geom::{affine, size};

/// Stored outlines remain authoritative; inherited ink needing recomputation stays counted.
pub(crate) fn approximate_ink(
    data: &Val,
    kind: &str,
    stale_glyphs: bool,
    outer_scale: impl Fn(&str) -> f64,
) -> bool {
    let lists = ["fillGeometry", "strokeGeometry", "dashPattern", "effects"];
    let radii = [
        "cornerRadius",
        "rectangleTopLeftCornerRadius",
        "rectangleTopRightCornerRadius",
        "rectangleBottomRightCornerRadius",
        "rectangleBottomLeftCornerRadius",
    ];
    lists
        .iter()
        .any(|key| outer_scale(key) != 1.0 && data.get(key).is_some_and(|v| !v.items().is_empty()))
        || radii
            .iter()
            .any(|key| outer_scale(key) != 1.0 && data.num(key).is_some_and(|v| v != 0.0))
        || (kind == "TEXT"
            && (outer_scale("fontSize") != 1.0
                || ["lineHeight", "letterSpacing"]
                    .iter()
                    .any(|key| outer_scale(key) != 1.0 && data.get(key).is_some()))
            && (stale_glyphs
                || data
                    .path(&["derivedTextData", "glyphs"])
                    .is_none_or(|v| v.items().is_empty())))
}

pub(crate) fn constrain(
    base: &Val,
    data: &mut Val,
    old_parent: &Val,
    parent: &Val,
    (move_by, grow_by): (f64, f64),
) {
    if !move_by.is_finite() || move_by <= 0.0 || !grow_by.is_finite() {
        return;
    }
    // Auto layout, not constraints, places in-flow children; it keeps them at the padding.
    let stack = matches!(parent.text("stackMode"), Some("HORIZONTAL" | "VERTICAL"));
    if stack && data.text("stackPositioning") != Some("ABSOLUTE") {
        return;
    }
    let old = size(old_parent);
    // Compare parent and child lengths in the omitted translation's pre-scale space.
    let (w, h) = size(parent);
    let new = (w / move_by, h / move_by);
    let before = size(base);
    let (w, h) = size(data);
    let after = (w * grow_by / move_by, h * grow_by / move_by);
    let mut tf = affine(data.get("transform")).as_coeffs();
    for (field, index, old_len, new_len, old_child, new_child) in [
        ("horizontalConstraint", 4, old.0, new.0, before.0, after.0),
        ("verticalConstraint", 5, old.1, new.1, before.1, after.1),
    ] {
        tf[index] = match base.text(field) {
            Some("CENTER") => tf[index] + (new_len - old_len - new_child + old_child) / 2.0,
            Some("MAX") => tf[index] + new_len - old_len - new_child + old_child,
            Some("SCALE") if old_len > 1e-6 => tf[index] * new_len / old_len,
            _ => tf[index],
        };
    }
    if tf != affine(base.get("transform")).as_coeffs() {
        let mut transform = data
            .get("transform")
            .cloned()
            .unwrap_or(Val::Obj(figstudio_fig::Obj::new()));
        transform.set("m02", Val::Float(tf[4]));
        transform.set("m12", Val::Float(tf[5]));
        data.set("transform", transform);
    }
}
