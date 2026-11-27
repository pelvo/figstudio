//! Privacy-safe schema and field census for `.fig` files.
//!
//! Reports only schema type names, field names, enum variant names, and occurrence counts.
//! Document text, layer names, colours, font families, image hashes, and file paths are
//! never included in [`CensusReport`].

use std::collections::BTreeMap;

use kiwi_schema::{DefKind, Schema};
use serde::{Deserialize, Serialize};

use crate::{Document, FigError, Val};

/// Aggregated, privacy-safe field and enum census across one or more `.fig` documents.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CensusReport {
    pub files_scanned: usize,
    pub total_nodes: usize,
    pub total_blobs: usize,
    pub total_images: usize,
    pub schema_defs: usize,
    pub node_change_schema_fields: usize,
    pub node_kinds: BTreeMap<String, usize>,
    pub node_fields: BTreeMap<String, FieldStat>,
    pub paint_types: BTreeMap<String, usize>,
    pub effect_types: BTreeMap<String, usize>,
    pub stack_modes: BTreeMap<String, usize>,
    pub boolean_ops: BTreeMap<String, usize>,
    pub style_types: BTreeMap<String, usize>,
    pub variable_types: BTreeMap<String, usize>,
}

/// Schema type and observed occurrence count of one `NodeChange` field.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FieldStat {
    pub kiwi_type: String,
    pub count: usize,
}

impl CensusReport {
    /// Scans a single [`Document`] and folds its counts into `self`.
    pub fn add_document(&mut self, doc: &Document) -> Result<(), FigError> {
        let schema = doc.decoded_schema()?;
        self.files_scanned += 1;
        self.total_nodes += doc.nodes.len();
        self.total_blobs += doc.blobs.len();
        self.total_images += doc.images.len() + doc.lazy_images.len();
        self.schema_defs = self.schema_defs.max(schema.defs.len());

        if let Some(nc) = schema.def("NodeChange") {
            self.node_change_schema_fields = self.node_change_schema_fields.max(nc.fields.len());
            for f in &nc.fields {
                let ty = format_field_type(&schema, f.type_id, f.is_array);
                self.node_fields
                    .entry(f.name.clone())
                    .or_insert_with(|| FieldStat {
                        kiwi_type: ty,
                        count: 0,
                    });
            }
        }

        for node in &doc.nodes {
            *self.node_kinds.entry(node.kind.clone()).or_default() += 1;
            let Val::Obj(obj) = &node.data else {
                continue;
            };
            for key in obj.keys() {
                self.node_fields
                    .entry((*key).to_string())
                    .or_insert_with(|| FieldStat {
                        kiwi_type: "unknown".into(),
                        count: 0,
                    })
                    .count += 1;
            }
            if let Some(Val::Enum(sm)) = node.data.get("stackMode") {
                *self.stack_modes.entry((*sm).to_string()).or_default() += 1;
            }
            if let Some(Val::Enum(bo)) = node.data.get("booleanOperation") {
                *self.boolean_ops.entry((*bo).to_string()).or_default() += 1;
            }
            if let Some(Val::Enum(st)) = node.data.get("styleType") {
                *self.style_types.entry((*st).to_string()).or_default() += 1;
            }
            if let Some(Val::Enum(vt)) = node.data.get("variableResolvedType") {
                *self.variable_types.entry((*vt).to_string()).or_default() += 1;
            }
            for paint_key in ["fillPaints", "strokePaints", "backgroundPaints"] {
                if let Some(list) = node.data.get(paint_key) {
                    for p in list.items() {
                        if let Some(Val::Enum(pt)) = p.get("type") {
                            *self.paint_types.entry((*pt).to_string()).or_default() += 1;
                        }
                    }
                }
            }
            if let Some(effects) = node.data.get("effects") {
                for eff in effects.items() {
                    if let Some(Val::Enum(et)) = eff.get("type") {
                        *self.effect_types.entry((*et).to_string()).or_default() += 1;
                    }
                }
            }
        }
        Ok(())
    }

    /// Formats the census as pretty-printed JSON.
    pub fn to_json(&self) -> Result<String, FigError> {
        serde_json::to_string_pretty(self)
            .map_err(|e| FigError::malformed(format!("cannot serialize census JSON: {e}")))
    }

    /// Formats the census as a Markdown summary table suitable for `docs/EDIT-COVERAGE.md`.
    pub fn to_markdown(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        let _ = writeln!(out, "# `.fig` Schema & Edit Coverage Census\n");
        let _ = writeln!(
            out,
            "- Files scanned: **{}**\n- Total nodes: **{}**\n- Schema definitions: **{}**\n- `NodeChange` schema fields: **{}** (observed active: **{}**)\n",
            self.files_scanned,
            self.total_nodes,
            self.schema_defs,
            self.node_change_schema_fields,
            self.node_fields.values().filter(|f| f.count > 0).count(),
        );

        let write_tally = |buf: &mut String, title: &str, map: &BTreeMap<String, usize>| {
            let _ = writeln!(buf, "## {title}\n");
            let _ = writeln!(buf, "| Variant | Count |\n|---|---:|");
            let mut items: Vec<_> = map.iter().collect();
            items.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (k, v) in items {
                let _ = writeln!(buf, "| `{k}` | {v} |");
            }
            let _ = writeln!(buf);
        };

        write_tally(&mut out, "Node Types", &self.node_kinds);
        write_tally(&mut out, "Paint Types", &self.paint_types);
        write_tally(&mut out, "Effect Types", &self.effect_types);
        write_tally(&mut out, "Auto-Layout Stack Modes", &self.stack_modes);
        write_tally(&mut out, "Boolean Operations", &self.boolean_ops);
        write_tally(&mut out, "Style Types", &self.style_types);
        write_tally(&mut out, "Variable Resolved Types", &self.variable_types);

        let _ = writeln!(out, "## Observed `NodeChange` Fields\n");
        let _ = writeln!(
            out,
            "| Field | Kiwi Type | Observed Nodes |\n|---|---|---:|"
        );
        let mut fields: Vec<_> = self
            .node_fields
            .iter()
            .filter(|(_, s)| s.count > 0)
            .collect();
        fields.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(b.0)));
        for (name, stat) in fields {
            let _ = writeln!(out, "| `{name}` | `{}` | {} |", stat.kiwi_type, stat.count);
        }
        out
    }
}

fn format_field_type(schema: &Schema, type_id: i32, is_array: bool) -> String {
    let base = match type_id {
        kiwi_schema::TYPE_BOOL => "bool",
        kiwi_schema::TYPE_BYTE => "byte",
        kiwi_schema::TYPE_INT => "int",
        kiwi_schema::TYPE_UINT => "uint",
        kiwi_schema::TYPE_FLOAT => "float",
        kiwi_schema::TYPE_STRING => "string",
        kiwi_schema::TYPE_INT64 => "int64",
        kiwi_schema::TYPE_UINT64 => "uint64",
        other => usize::try_from(other)
            .ok()
            .and_then(|i| schema.defs.get(i))
            .map(|d| {
                let _ = DefKind::Enum;
                d.name.as_str()
            })
            .unwrap_or("unknown"),
    };
    if is_array {
        format!("{base}[]")
    } else {
        base.to_string()
    }
}
