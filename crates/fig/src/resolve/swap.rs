//! Instance swaps: fields the instance only copied from its old component follow the new one.
use crate::{Document, Guid, Obj, Val};

/// Fields an instance takes from its main component unless they are overridden.
pub(crate) const SWAP_INHERITED: [&str; 27] = [
    "fillPaints",
    "strokePaints",
    "strokeWeight",
    "strokeAlign",
    "strokeJoin",
    "strokeCap",
    "dashPattern",
    "cornerRadius",
    "cornerSmoothing",
    "rectangleCornerRadiiIndependent",
    "rectangleTopLeftCornerRadius",
    "rectangleTopRightCornerRadius",
    "rectangleBottomLeftCornerRadius",
    "rectangleBottomRightCornerRadius",
    "borderStrokeWeightsIndependent",
    "borderTopWeight",
    "borderRightWeight",
    "borderBottomWeight",
    "borderLeftWeight",
    "effects",
    "opacity",
    "blendMode",
    "styleIdForFill",
    "styleIdForStrokeFill",
    "styleIdForEffect",
    "frameMaskDisabled",
    "variableConsumptionMap",
];

/// Stored outlines that depend on the shape fields above.
pub(crate) const SHAPE_GEOMETRY: [&str; 2] = ["fillGeometry", "strokeGeometry"];

/// Whether `data` swaps away from `base`'s own `symbolData.symbolID`.
pub(crate) fn has_swap(base: &Val, data: &Val) -> bool {
    matches!(
        (
            data.get("overriddenSymbolID").and_then(Val::as_guid),
            base.path(&["symbolData", "symbolID"])
                .and_then(Val::as_guid),
        ),
        (Some(new), Some(old)) if new != old
    )
}

fn is_shape_affecting(key: &str) -> bool {
    !matches!(key, "fillPaints" | "effects" | "opacity" | "blendMode")
}

/// Computes `(set, unset)` updates to transition an instance's inherited fields from
/// `old_symbol` to `new_symbol`, preserving any field the instance itself overrode.
/// Drops stored `fillGeometry`/`strokeGeometry` when shape-affecting fields change.
pub(crate) fn swap_inherited_patch(
    doc: &Document,
    inst_data: &Val,
    old_symbol: Guid,
    new_symbol: Guid,
    extra_keys: &[&'static str],
    set: &mut Obj,
    unset: &mut Vec<String>,
) {
    if old_symbol == new_symbol {
        return;
    }
    let (Some(old), Some(new)) = (doc.node(old_symbol), doc.node(new_symbol)) else {
        return;
    };
    let mut shape_changed = false;
    for &key in SWAP_INHERITED.iter().chain(extra_keys.iter()) {
        let own = inst_data.get(key);
        if own != old.data.get(key) || own == new.data.get(key) {
            continue;
        }
        if SWAP_INHERITED.contains(&key) {
            shape_changed |= is_shape_affecting(key);
        }
        match new.data.get(key) {
            Some(v) => {
                set.insert(key, v.clone());
            }
            None => {
                if own.is_some() && !unset.iter().any(|u| u == key) {
                    unset.push(key.to_string());
                }
            }
        }
    }
    if shape_changed {
        for key in SHAPE_GEOMETRY {
            if inst_data.get(key).is_some() && !unset.iter().any(|u| u == key) {
                unset.push(key.to_string());
            }
        }
    }
}

/// After an instance swap (`overriddenSymbolID` in `data` differs from the instance's own
/// `symbolData.symbolID` in `base`), takes every field from the new component that the
/// instance only copied from the old one. Fields that differ from the old component, in the
/// instance itself or set by an override, are the user's overrides and stay. Stored geometry
/// built for the old shape is dropped when the shape changed, so the base shape is drawn.
pub fn inherit_swapped(doc: &Document, base: &Val, data: &mut Val) {
    let new = data.get("overriddenSymbolID").and_then(Val::as_guid);
    let old = base
        .path(&["symbolData", "symbolID"])
        .and_then(Val::as_guid);
    let (Some(new), Some(old)) = (new, old) else {
        return;
    };
    if new == old {
        return;
    }
    let (Some(new), Some(old)) = (doc.node(new), doc.node(old)) else {
        return;
    };
    let mut shape_changed = false;
    for key in SWAP_INHERITED {
        let own = base.get(key);
        if own != old.data.get(key) || data.get(key) != own || own == new.data.get(key) {
            continue;
        }
        shape_changed |= is_shape_affecting(key);
        set_or_remove(data, key, new.data.get(key));
    }
    for key in SHAPE_GEOMETRY {
        if shape_changed && data.get(key) == base.get(key) {
            set_or_remove(data, key, None);
        }
    }
}

fn set_or_remove(data: &mut Val, key: &str, value: Option<&Val>) {
    match (value, data) {
        (Some(v), data) => data.set(key, v.clone()),
        (None, Val::Obj(o)) => {
            o.remove(key);
        }
        (None, _) => {}
    }
}
