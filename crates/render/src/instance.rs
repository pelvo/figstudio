//! Component properties are scoped to the instance that owns their references.
use std::collections::HashMap;
use std::sync::Arc;

use figstudio_fig::{Document, Guid, Node, Obj, Val};

/// Property definitions by id across the whole document; built once per document.
pub(crate) type Definitions = HashMap<Guid, Val>;

/// Every `componentPropDefs` entry in `doc`, by id.
pub(crate) fn definitions(doc: &Document) -> Definitions {
    let mut definitions = HashMap::new();
    for node in &doc.nodes {
        for def in node
            .data
            .get("componentPropDefs")
            .map(Val::items)
            .unwrap_or_default()
        {
            if let Some(id) = def.get("id").and_then(Val::as_guid) {
                definitions.insert(id, def.clone());
            }
        }
    }
    definitions
}

pub(crate) struct Properties {
    definitions: Arc<Definitions>,
    scopes: Vec<HashMap<Guid, Val>>,
}

impl Properties {
    pub fn new(definitions: Arc<Definitions>) -> Self {
        Self {
            definitions,
            scopes: Vec::new(),
        }
    }

    fn default_value(&self, id: Guid, depth: usize) -> Option<Val> {
        if depth >= 24 {
            return None;
        }
        let def = self.definitions.get(&id)?;
        if let Some(parent) = def.get("parentPropDefId").and_then(Val::as_guid) {
            return self.default_value(parent, depth + 1);
        }
        property_value(def, "initialValue")
    }

    pub fn push(&mut self, symbol: &Node, data: &Val) {
        let mut scope = HashMap::new();
        for def in symbol
            .data
            .get("componentPropDefs")
            .map(Val::items)
            .unwrap_or_default()
        {
            if let Some(id) = def.get("id").and_then(Val::as_guid)
                && let Some(value) = self.default_value(id, 0)
            {
                scope.insert(id, value);
            }
        }
        for assignment in data
            .get("componentPropAssignments")
            .map(Val::items)
            .unwrap_or_default()
        {
            if let Some(id) = assignment.get("defID").and_then(Val::as_guid)
                && let Some(value) = property_value(assignment, "value")
            {
                scope.insert(id, value);
            }
        }
        self.scopes.push(scope);
    }

    pub fn pop(&mut self) {
        self.scopes.pop();
    }

    pub fn patch(&self, data: &Val) -> Val {
        let mut patch = Val::Obj(Obj::new());
        let Some(scope) = self.scopes.last() else {
            return patch;
        };
        for reference in data
            .get("componentPropRefs")
            .map(Val::items)
            .unwrap_or_default()
        {
            let Some(value) = reference
                .get("defID")
                .and_then(Val::as_guid)
                .and_then(|id| scope.get(&id))
            else {
                continue;
            };
            let field = match reference.text("componentPropNodeField") {
                Some("VISIBLE") => "visible",
                Some("TEXT_DATA") => "textData",
                Some("OVERRIDDEN_SYMBOL_ID") => "overriddenSymbolID",
                _ => continue,
            };
            patch.set(field, value.clone());
        }
        patch
    }
}

fn property_value(data: &Val, key: &str) -> Option<Val> {
    let value = data
        .path(&["varValue", "value"])
        .or_else(|| data.get(key))?;
    value
        .get("boolValue")
        .cloned()
        .or_else(|| value.get("textDataValue").cloned())
        .or_else(|| {
            value.get("textValue").map(|v| {
                if v.as_str().is_some() {
                    Val::Obj(Obj::from([("characters", v.clone())]))
                } else {
                    v.clone()
                }
            })
        })
        .or_else(|| value.path(&["symbolIdValue", "guid"]).cloned())
        .or_else(|| value.get("guidValue").cloned())
}

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
pub(crate) fn inherit_swapped(doc: &Document, base: &Val, data: &mut Val) {
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
