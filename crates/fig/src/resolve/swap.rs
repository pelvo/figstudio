//! Instance swaps: fields the instance only copied from its old component follow the new one.
use crate::{Document, Val};

/// Fields an instance takes from its main component unless they are overridden.
const SWAP_INHERITED: [&str; 26] = [
    "fillPaints",
    "strokePaints",
    "strokeWeight",
    "strokeAlign",
    "strokeJoin",
    "strokeCap",
    "dashPattern",
    "cornerRadius",
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
const SHAPE_GEOMETRY: [&str; 2] = ["fillGeometry", "strokeGeometry"];

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
        shape_changed |= !matches!(key, "fillPaints" | "effects" | "opacity" | "blendMode");
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
