//! Stored derived layout is authoritative; constraints fill in omitted translations.
use figstudio_fig::Val;

use crate::geom::{affine, size};

pub(crate) fn constrain(base: &Val, data: &mut Val, old_parent: &Val, parent: &Val) {
    if data.get("transform") != base.get("transform") {
        return;
    }
    // Auto layout, not constraints, places in-flow children; it keeps them at the padding.
    let stack = matches!(parent.text("stackMode"), Some("HORIZONTAL" | "VERTICAL"));
    if stack && data.text("stackPositioning") != Some("ABSOLUTE") {
        return;
    }
    let old = size(old_parent);
    let new = size(parent);
    let before = size(base);
    let after = size(data);
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
