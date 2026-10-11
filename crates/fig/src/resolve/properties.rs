//! Component properties are scoped to the instance that owns their references.
use std::collections::HashMap;
use std::sync::Arc;

use crate::{Document, Guid, Node, Obj, Val};

/// Property definitions by id across the whole document; built once per document.
pub type Definitions = HashMap<Guid, Val>;

/// Every `componentPropDefs` entry in `doc`, by id.
pub fn definitions(doc: &Document) -> Definitions {
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

#[derive(Debug)]
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
