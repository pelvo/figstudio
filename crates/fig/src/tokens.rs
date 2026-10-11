//! Read-only, schema-independent variables and styles indexed from a document.
use crate::{Document, Guid, Val};
use std::collections::HashMap;

/// Keep the renderer's historical 24-hop ceiling. Lower limits can leave valid library
/// chains unresolved; raising it permits more work on hostile alias cycles.
pub const MAX_ALIAS_DEPTH: usize = 24;

/// One named variable mode.
#[derive(Clone, Debug)]
pub struct Mode {
    pub id: Guid,
    pub name: String,
}

/// A local variable collection.
#[derive(Debug)]
pub struct Collection {
    pub id: Guid,
    pub name: String,
    pub modes: Vec<Mode>,
    pub default_mode: Option<Guid>,
}

/// One per-mode stored variable value.
#[derive(Debug)]
pub struct ModeValue {
    pub mode: Option<Guid>,
    pub data: Val,
}

/// A variable retains its original per-mode payload without modifying the document.
#[derive(Debug)]
pub struct Variable {
    pub id: Guid,
    pub name: String,
    pub collection: Option<Guid>,
    pub resolved_type: Option<String>,
    pub values: Vec<ModeValue>,
    /// Exact first stored entry, including absence, for renderer compatibility.
    pub first_value: Option<Val>,
}

/// Final variable value, after bounded alias resolution.
#[derive(Debug, PartialEq)]
pub enum Value {
    Color(Val),
    Float(f64),
    String(String),
    Bool(bool),
}

/// Stored style payload, addressed by either a local GUID or published asset reference.
#[derive(Debug)]
pub struct Style {
    pub id: Guid,
    pub name: String,
    pub kind: String,
    pub paints: Option<Val>,
    pub text: Val,
    pub effects: Option<Val>,
    pub grids: Option<Val>,
}

/// An owned read-only index; no decoding, fonts or write-time changes.
#[derive(Debug, Default)]
pub struct Tokens {
    pub collections: HashMap<Guid, Collection>,
    pub variables: HashMap<Guid, Variable>,
    pub styles: HashMap<Guid, Style>,
    variable_assets: HashMap<(String, String), Guid>,
    style_assets: HashMap<(String, String), Guid>,
}

impl Tokens {
    pub fn new(doc: &Document) -> Self {
        let mut out = Self::default();
        for node in &doc.nodes {
            let d = &node.data;
            if node.kind == "VARIABLE_SET" {
                let modes: Vec<_> = d
                    .get("variableSetModes")
                    .into_iter()
                    .flat_map(Val::items)
                    .filter_map(|m| {
                        Some(Mode {
                            id: m.get("id")?.as_guid()?,
                            name: m.text("name").unwrap_or("").into(),
                        })
                    })
                    .collect();
                let default_mode = d
                    .get("variableSetDefaultModeID")
                    .and_then(Val::as_guid)
                    .or_else(|| modes.first().map(|m| m.id));
                out.collections.insert(
                    node.id,
                    Collection {
                        id: node.id,
                        name: node.name.clone(),
                        modes,
                        default_mode,
                    },
                );
            }
            if node.kind == "VARIABLE" {
                let values = d
                    .path(&["variableDataValues", "entries"])
                    .into_iter()
                    .flat_map(Val::items)
                    .filter_map(|v| {
                        Some(ModeValue {
                            mode: v.get("modeID").and_then(Val::as_guid),
                            data: v.get("variableData")?.clone(),
                        })
                    })
                    .collect();
                out.variables.insert(
                    node.id,
                    Variable {
                        id: node.id,
                        name: node.name.clone(),
                        collection: d
                            .get("variableSetID")
                            .and_then(|v| reference_id(v, &HashMap::new())),
                        resolved_type: d.text("variableResolvedType").map(str::to_owned),
                        first_value: d
                            .path(&["variableDataValues", "entries"])
                            .and_then(|v| v.items().first())
                            .and_then(|v| v.get("variableData"))
                            .cloned(),
                        values,
                    },
                );
                if let (Some(k), Some(v)) = (d.text("key"), d.text("version")) {
                    out.variable_assets.insert((k.into(), v.into()), node.id);
                }
            }
            if let Some(kind) = d.text("styleType") {
                out.styles.insert(
                    node.id,
                    Style {
                        id: node.id,
                        name: node.name.clone(),
                        kind: kind.into(),
                        paints: d.get("fillPaints").cloned(),
                        text: d.clone(),
                        effects: d.get("effects").cloned(),
                        grids: d.get("layoutGrids").cloned(),
                    },
                );
                if let (Some(k), Some(v)) = (d.text("key"), d.text("version")) {
                    out.style_assets.insert((k.into(), v.into()), node.id);
                }
            }
        }
        out
    }

    pub fn variable(&self, reference: &Val) -> Option<&Variable> {
        self.variables
            .get(&reference_id(reference, &self.variable_assets)?)
    }

    pub fn style(&self, reference: &Val) -> Option<&Style> {
        self.styles
            .get(&reference_id(reference, &self.style_assets)?)
    }

    pub fn resolve(&self, data: &Val, mode: Option<Guid>) -> Option<Value> {
        self.resolve_at(data, mode, 0)
    }

    pub fn resolve_at(&self, data: &Val, mode: Option<Guid>, depth: usize) -> Option<Value> {
        if depth >= MAX_ALIAS_DEPTH {
            return None;
        }
        let value = data.get("value")?;
        if let Some(color) = value.get("colorValue") {
            return Some(Value::Color(color.clone()));
        }
        if let Some(Val::Float(v)) = value.get("floatValue") {
            return Some(Value::Float(*v));
        }
        if let Some(Val::Int(v)) = value.get("floatValue") {
            return Some(Value::Float(*v as f64));
        }
        if let Some(v) = value.text("textValue") {
            return Some(Value::String(v.into()));
        }
        if let Some(v) = value.flag("boolValue") {
            return Some(Value::Bool(v));
        }
        let target = self.variable(value.get("alias")?)?;
        let entry = match mode {
            None => target.first_value.as_ref()?,
            Some(m) => {
                &target
                    .values
                    .iter()
                    .find(|v| v.mode == Some(m))
                    .or_else(|| {
                        target
                            .collection
                            .and_then(|id| self.collections.get(&id))
                            .and_then(|c| target.values.iter().find(|v| v.mode == c.default_mode))
                    })
                    .or_else(|| target.values.first())?
                    .data
            }
        };
        self.resolve_at(entry, mode, depth + 1)
    }
}

fn reference_id(reference: &Val, assets: &HashMap<(String, String), Guid>) -> Option<Guid> {
    reference
        .as_guid()
        .or_else(|| reference.get("guid").and_then(Val::as_guid))
        .or_else(|| {
            let asset = reference.get("assetRef")?;
            assets
                .get(&(asset.text("key")?.into(), asset.text("version")?.into()))
                .copied()
        })
}
