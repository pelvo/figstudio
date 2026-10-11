//! Read-only, schema-independent variables and styles indexed from a document.
use crate::{Document, Guid, Val};
use std::collections::HashMap;

/// Keep the renderer's historical 24-hop ceiling. Lower limits can leave valid library
/// chains unresolved; raising it permits more work on hostile alias cycles.
pub const MAX_ALIAS_DEPTH: usize = 24;

mod types;
pub use types::{Collection, Mode, ModeValue, Style, StyleKind, Value, Variable};

/// An owned read-only index; no decoding, fonts or write-time changes.
#[derive(Debug, Default)]
pub struct Tokens {
    collections: HashMap<Guid, Collection>,
    variables: HashMap<Guid, Variable>,
    styles: HashMap<Guid, Style>,
    variable_assets: HashMap<(String, String), Guid>,
    style_assets: HashMap<(String, String), Guid>,
}

impl Tokens {
    /// Index local collections, variables and styles in document order.
    pub fn new(doc: &Document) -> Self {
        let mut out = Self::default();
        for node in &doc.nodes {
            let d = &node.data;
            if node.kind == "VARIABLE_SET" {
                let mut modes: Vec<_> = d
                    .get("variableSetModes")
                    .into_iter()
                    .flat_map(Val::items)
                    .filter_map(|m| {
                        Some(Mode {
                            id: m.get("id")?.as_guid()?,
                            name: m.text("name").unwrap_or("").into(),
                            sort_position: m.text("sortPosition").unwrap_or("").into(),
                        })
                    })
                    .collect();
                modes.sort_by(|a, b| a.sort_position.cmp(&b.sort_position));
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
                // Legacy rendering indexed only variables with a first stored payload.
                if out.variables[&node.id].first_value.is_some()
                    && let (Some(k), Some(v)) = (d.text("key"), d.text("version"))
                {
                    out.variable_assets.insert((k.into(), v.into()), node.id);
                }
            }
            if let Some(kind) = d.text("styleType") {
                out.styles.insert(
                    node.id,
                    Style {
                        id: node.id,
                        name: node.name.clone(),
                        kind: match kind {
                            "FILL" => StyleKind::Fill,
                            "STROKE" => StyleKind::Stroke,
                            "TEXT" => StyleKind::Text,
                            "EFFECT" => StyleKind::Effect,
                            "GRID" => StyleKind::Grid,
                            other => StyleKind::Other(other.into()),
                        },
                        paints: d.get("fillPaints").cloned(),
                        data: d.clone(),
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

    /// Look up a collection by local identifier.
    pub fn collection(&self, id: Guid) -> Option<&Collection> {
        self.collections.get(&id)
    }

    /// Iterate local variables; iteration order is unspecified.
    pub fn variables(&self) -> impl Iterator<Item = &Variable> {
        self.variables.values()
    }

    /// Resolve a local or published variable reference.
    pub fn variable(&self, reference: &Val) -> Option<&Variable> {
        self.variables
            .get(&reference_id(reference, &self.variable_assets)?)
    }

    /// Resolve a local or published style reference.
    pub fn style(&self, reference: &Val) -> Option<&Style> {
        self.styles
            .get(&reference_id(reference, &self.style_assets)?)
    }

    /// Resolve aliases in the requested mode, or first stored entries when absent.
    pub fn resolve(&self, data: &Val, mode: Option<Guid>) -> Option<Value> {
        self.resolve_at(data, mode, 0)
    }

    /// Resolve with the renderer's existing depth count and first-stored-mode semantics.
    ///
    /// Pass `None` for the mode to preserve renderer compatibility; sorted collection
    /// defaults are intentionally independent of this historical storage-order rule.
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

/// Infer numeric font weight from the stored style name, without loading fonts.
pub fn font_weight(style: &str) -> u16 {
    let s = style.to_ascii_lowercase().replace([' ', '-'], "");
    for (names, weight) in [
        (["thin", "hairline"], 100),
        (["extralight", "ultralight"], 200),
        (["semibold", "demibold"], 600),
        (["extrabold", "ultrabold"], 800),
        (["black", "heavy"], 900),
        (["bold", "bold"], 700),
        (["medium", "medium"], 500),
        (["light", "light"], 300),
    ] {
        if names.iter().any(|n| s.contains(n)) {
            return weight;
        }
    }
    400
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
