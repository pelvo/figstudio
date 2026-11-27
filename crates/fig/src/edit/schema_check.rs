//! Up-front Kiwi schema validation for [`NodePatch`](super::NodePatch) writes.
//!
//! Ensures every mutated or created field conforms to the document's own Kiwi schema
//! before [`Document::apply_txn`](crate::Document::apply_txn) mutates any state.

use kiwi_schema::{
    Def, DefKind, Field, Schema, TYPE_BOOL, TYPE_BYTE, TYPE_FLOAT, TYPE_INT, TYPE_INT64,
    TYPE_STRING, TYPE_UINT, TYPE_UINT64,
};

use crate::{FigError, Obj, Val, intern};

const MAX_COERCE_DEPTH: usize = 128;

/// Policy controlling whether `TYPE_FLOAT` validation rejects values that are non-finite in the
/// Kiwi `f32` wire representation. Newly authored writes enforce `FiniteF32`; history undo/redo
/// uses `PreserveExisting` so pre-existing Figma-authored `NaN`s survive deletion and edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FloatPolicy {
    FiniteF32,
    PreserveExisting,
}

/// Whether `v` remains finite after narrowing to Kiwi's 32-bit float wire format.
pub(crate) fn is_finite_f32(v: f64) -> bool {
    (v as f32).is_finite()
}

/// Ensures `v` is finite in Kiwi's `f32` wire representation.
pub(crate) fn require_finite_f32(v: f64, at: &str) -> Result<f64, FigError> {
    if is_finite_f32(v) {
        Ok(v)
    } else {
        Err(FigError::malformed(format!(
            "{at}: expected finite 32-bit float"
        )))
    }
}

fn node_change_def(schema: &Schema) -> Option<&Def> {
    schema.def("NodeChange").or_else(|| {
        let f = schema.def("Message")?.field("nodeChanges")?;
        schema.defs.get(usize::try_from(f.type_id).ok()?)
    })
}

/// Validates a full `NodeChange` object against `schema`.
pub fn validate_node_data(schema: &Schema, data: &Val, at: &str) -> Result<(), FigError> {
    validate_node_data_with(schema, data, at, FloatPolicy::FiniteF32)
}

pub(crate) fn validate_node_data_with(
    schema: &Schema,
    data: &Val,
    at: &str,
    floats: FloatPolicy,
) -> Result<(), FigError> {
    let Some(nc_idx) = schema
        .defs
        .iter()
        .position(|d| d.name == "NodeChange" && d.kind == DefKind::Message)
    else {
        // Synthetic test schemas may name the node message `Node` via `Message.nodeChanges`.
        let fallback = schema
            .def("Message")
            .and_then(|m| m.field("nodeChanges"))
            .and_then(|f| usize::try_from(f.type_id).ok());
        if let Some(idx) = fallback {
            return validate_value(schema, idx as i32, data, at, floats);
        }
        return Err(FigError::malformed("schema has no NodeChange definition"));
    };
    if data.get("guid").and_then(Val::as_guid).is_none() {
        return Err(FigError::malformed(format!("{at}: missing valid guid")));
    }
    validate_value(schema, nc_idx as i32, data, at, floats)
}

fn lookup_node_field<'a>(schema: &'a Schema, key: &str, at: &str) -> Result<&'a Field, FigError> {
    let def = node_change_def(schema)
        .ok_or_else(|| FigError::malformed("schema has no NodeChange definition"))?;
    def.field(key)
        .ok_or_else(|| FigError::malformed(format!("{at}: {} has no field {key}", def.name)))
}

/// Validates a single top-level field write `(key, val)` on `NodeChange`.
pub fn validate_node_field(
    schema: &Schema,
    key: &str,
    val: &Val,
    at: &str,
) -> Result<(), FigError> {
    validate_node_field_with(schema, key, val, at, FloatPolicy::FiniteF32)
}

pub(crate) fn validate_node_field_with(
    schema: &Schema,
    key: &str,
    val: &Val,
    at: &str,
    floats: FloatPolicy,
) -> Result<(), FigError> {
    let field = lookup_node_field(schema, key, at)?;
    validate_field(schema, field, val, &format!("{at}.{key}"), floats)
}

/// Validates `variant` against the enum definition of a top-level `NodeChange` field before
/// interning, so untrusted invalid enum names never consume process interner budget.
pub(crate) fn intern_node_enum(
    schema: &Schema,
    field_name: &str,
    variant: &str,
    at: &str,
) -> Result<Val, FigError> {
    intern_node_enum_path(schema, &[field_name], variant, at)
}

/// Validates `variant` against the enum definition at a nested field `path` rooted at
/// `NodeChange` before interning.
pub(crate) fn intern_node_enum_path(
    schema: &Schema,
    path: &[&str],
    variant: &str,
    at: &str,
) -> Result<Val, FigError> {
    let mut def = node_change_def(schema)
        .ok_or_else(|| FigError::malformed("schema has no NodeChange definition"))?;
    let mut field_opt = None;
    for &segment in path {
        let field = def.field(segment).ok_or_else(|| {
            FigError::malformed(format!("{at}: {} has no field {segment}", def.name))
        })?;
        let next_def = usize::try_from(field.type_id)
            .ok()
            .and_then(|i| schema.defs.get(i))
            .ok_or_else(|| {
                FigError::malformed(format!("{at}.{segment}: unknown type {}", field.type_id))
            })?;
        def = next_def;
        field_opt = Some(field);
    }
    if field_opt.is_none() || def.kind != DefKind::Enum {
        return Err(FigError::malformed(format!(
            "{at}: expected enum definition, found {}",
            def.name
        )));
    }
    let Some(entry) = def.field(variant) else {
        return Err(FigError::malformed(format!(
            "{at}: {} has no variant {variant}",
            def.name
        )));
    };
    Ok(Val::Enum(intern(&entry.name)))
}

/// Schema-guided JSON-to-[`Val`] coercion for a top-level `NodeChange` field (`PatchRaw`).
/// Validates every top-level and nested field name and enum variant against `schema` before
/// interning so rejected payloads never consume process-wide interner budget.
pub(crate) fn coerce_json_node_field(
    schema: &Schema,
    key: &str,
    jv: &serde_json::Value,
    at: &str,
) -> Result<(&'static str, Val), FigError> {
    let field = lookup_node_field(schema, key, at)?;
    let val = coerce_json_field(schema, field, jv, &format!("{at}.{key}"), 0)?;
    Ok((intern(&field.name), val))
}

fn coerce_json_field(
    schema: &Schema,
    field: &Field,
    jv: &serde_json::Value,
    at: &str,
    depth: usize,
) -> Result<Val, FigError> {
    if depth > MAX_COERCE_DEPTH {
        return Err(FigError::malformed(format!("{at}: nesting depth exceeded")));
    }
    if !field.is_array {
        return coerce_json_value(schema, field.type_id, jv, at, depth);
    }
    let Some(items) = jv.as_array() else {
        return Err(FigError::malformed(format!(
            "{at}: {} must be a list",
            field.name
        )));
    };
    if u32::try_from(items.len()).is_err() {
        return Err(FigError::malformed(format!("{at}: list too long")));
    }
    if field.type_id == TYPE_BYTE {
        let mut bytes = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let Some(b) = item.as_i64().and_then(|n| u8::try_from(n).ok()) else {
                return Err(FigError::malformed(format!(
                    "{at}[{i}]: expected byte (0..=255)"
                )));
            };
            bytes.push(b);
        }
        return Ok(Val::Bytes(bytes));
    }
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        out.push(coerce_json_value(
            schema,
            field.type_id,
            item,
            &format!("{at}[{i}]"),
            depth + 1,
        )?);
    }
    Ok(Val::Arr(out))
}

fn coerce_json_value(
    schema: &Schema,
    type_id: i32,
    jv: &serde_json::Value,
    at: &str,
    depth: usize,
) -> Result<Val, FigError> {
    if depth > MAX_COERCE_DEPTH {
        return Err(FigError::malformed(format!("{at}: nesting depth exceeded")));
    }
    match type_id {
        TYPE_BOOL => jv
            .as_bool()
            .map(Val::Bool)
            .ok_or_else(|| FigError::malformed(format!("{at}: expected bool"))),
        TYPE_BYTE => jv
            .as_i64()
            .filter(|n| u8::try_from(*n).is_ok())
            .map(Val::Int)
            .ok_or_else(|| FigError::malformed(format!("{at}: expected byte (0..=255)"))),
        TYPE_INT => jv
            .as_i64()
            .filter(|n| i32::try_from(*n).is_ok())
            .map(Val::Int)
            .ok_or_else(|| FigError::malformed(format!("{at}: expected 32-bit int"))),
        TYPE_UINT => jv
            .as_i64()
            .filter(|n| u32::try_from(*n).is_ok())
            .map(Val::Int)
            .ok_or_else(|| FigError::malformed(format!("{at}: expected 32-bit uint"))),
        TYPE_INT64 | TYPE_UINT64 => jv
            .as_i64()
            .map(Val::Int)
            .ok_or_else(|| FigError::malformed(format!("{at}: expected 64-bit integer"))),
        TYPE_FLOAT => match jv {
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(Val::Int(i))
                } else if let Some(f) = n.as_f64() {
                    require_finite_f32(f, at)?;
                    Ok(Val::Float(f))
                } else {
                    Err(FigError::malformed(format!("{at}: expected float")))
                }
            }
            serde_json::Value::Object(map) if map.len() == 1 => {
                if let Some(serde_json::Value::String(hex)) = map.get("$float_bits")
                    && hex.len() == 16
                    && hex.bytes().all(|b| b.is_ascii_hexdigit())
                    && let Ok(bits) = u64::from_str_radix(hex, 16)
                {
                    let f = f64::from_bits(bits);
                    require_finite_f32(f, at)?;
                    return Ok(Val::Float(f));
                }
                Err(FigError::malformed(format!("{at}: expected float")))
            }
            _ => Err(FigError::malformed(format!("{at}: expected float"))),
        },
        TYPE_STRING => jv
            .as_str()
            .map(|s| Val::Str(s.to_owned()))
            .ok_or_else(|| FigError::malformed(format!("{at}: expected string"))),
        other => {
            let Some(def) = usize::try_from(other).ok().and_then(|i| schema.defs.get(i)) else {
                return Err(FigError::malformed(format!("{at}: unknown type {other}")));
            };
            match def.kind {
                DefKind::Enum => {
                    let Some(s) = jv.as_str() else {
                        return Err(FigError::malformed(format!(
                            "{at}: expected enum {}",
                            def.name
                        )));
                    };
                    let Some(variant) = def.field(s) else {
                        return Err(FigError::malformed(format!(
                            "{at}: {} has no variant {s}",
                            def.name
                        )));
                    };
                    Ok(Val::Enum(intern(&variant.name)))
                }
                DefKind::Struct | DefKind::Message => {
                    let Some(map) = jv.as_object() else {
                        return Err(FigError::malformed(format!(
                            "{at}: expected object for {}",
                            def.name
                        )));
                    };
                    let mut out = Obj::with_capacity(map.len());
                    for (k, fv) in map {
                        let Some(f) = def.field(k) else {
                            return Err(FigError::malformed(format!(
                                "{at}: {} has no field {k}",
                                def.name
                            )));
                        };
                        let coerced =
                            coerce_json_field(schema, f, fv, &format!("{at}.{k}"), depth + 1)?;
                        out.insert(&f.name, coerced);
                    }
                    Ok(Val::Obj(out))
                }
            }
        }
    }
}

/// Coerces a JSON-deserialized [`Val`] for a top-level `NodeChange` field against `schema`:
/// - string values at `DefKind::Enum` positions become `Val::Enum` (validating the variant
///   against the schema before interning it);
/// - integer arrays at `byte[]` positions become `Val::Bytes`;
/// - nested arrays, structs, and messages are coerced recursively.
pub fn coerce_node_field(schema: &Schema, key: &str, val: Val, at: &str) -> Result<Val, FigError> {
    let field = lookup_node_field(schema, key, at)?;
    coerce_field(schema, field, val, &format!("{at}.{key}"), 0)
}

fn coerce_field(
    schema: &Schema,
    field: &Field,
    v: Val,
    at: &str,
    depth: usize,
) -> Result<Val, FigError> {
    if depth > MAX_COERCE_DEPTH {
        return Err(FigError::malformed(format!("{at}: nesting depth exceeded")));
    }
    let v = v.into_owned();
    if !field.is_array {
        return coerce_value(schema, field.type_id, v, at, depth);
    }
    if field.type_id == TYPE_BYTE {
        return match v {
            Val::Bytes(b) => Ok(Val::Bytes(b)),
            Val::Arr(items) => {
                if u32::try_from(items.len()).is_err() {
                    return Err(FigError::malformed(format!("{at}: list too long")));
                }
                let mut bytes = Vec::with_capacity(items.len());
                for (i, item) in items.into_iter().enumerate() {
                    match item.view() {
                        Val::Int(n) => {
                            let b = u8::try_from(*n).map_err(|_| {
                                FigError::malformed(format!("{at}[{i}]: expected byte (0..=255)"))
                            })?;
                            bytes.push(b);
                        }
                        _ => {
                            return Err(FigError::malformed(format!(
                                "{at}[{i}]: expected byte (0..=255)"
                            )));
                        }
                    }
                }
                Ok(Val::Bytes(bytes))
            }
            _ => Err(FigError::malformed(format!(
                "{at}: {} must be a list",
                field.name
            ))),
        };
    }
    match v {
        Val::Arr(items) => {
            if u32::try_from(items.len()).is_err() {
                return Err(FigError::malformed(format!("{at}: list too long")));
            }
            let mut out = Vec::with_capacity(items.len());
            for (i, item) in items.into_iter().enumerate() {
                out.push(coerce_value(
                    schema,
                    field.type_id,
                    item,
                    &format!("{at}[{i}]"),
                    depth + 1,
                )?);
            }
            Ok(Val::Arr(out))
        }
        _ => Err(FigError::malformed(format!(
            "{at}: {} must be a list",
            field.name
        ))),
    }
}

fn coerce_value(
    schema: &Schema,
    type_id: i32,
    v: Val,
    at: &str,
    depth: usize,
) -> Result<Val, FigError> {
    if depth > MAX_COERCE_DEPTH {
        return Err(FigError::malformed(format!("{at}: nesting depth exceeded")));
    }
    let v = v.into_owned();
    match type_id {
        TYPE_BOOL | TYPE_BYTE | TYPE_INT | TYPE_UINT | TYPE_INT64 | TYPE_UINT64 | TYPE_FLOAT
        | TYPE_STRING => Ok(v),
        other => {
            let Some(def) = usize::try_from(other).ok().and_then(|i| schema.defs.get(i)) else {
                return Err(FigError::malformed(format!("{at}: unknown type {other}")));
            };
            match def.kind {
                DefKind::Enum => match v {
                    Val::Enum(variant) => {
                        if def.field(variant).is_none() {
                            return Err(FigError::malformed(format!(
                                "{at}: {} has no variant {variant}",
                                def.name
                            )));
                        }
                        Ok(Val::Enum(variant))
                    }
                    Val::Str(s) => {
                        let Some(variant) = def.field(&s) else {
                            return Err(FigError::malformed(format!(
                                "{at}: {} has no variant {s}",
                                def.name
                            )));
                        };
                        Ok(Val::Enum(intern(&variant.name)))
                    }
                    _ => Err(FigError::malformed(format!(
                        "{at}: expected enum {}",
                        def.name
                    ))),
                },
                DefKind::Struct | DefKind::Message => match v {
                    Val::Obj(obj) => {
                        let mut out = Obj::with_capacity(obj.len());
                        for (k, fv) in obj.iter() {
                            let Some(f) = def.field(k) else {
                                return Err(FigError::malformed(format!(
                                    "{at}: {} has no field {k}",
                                    def.name
                                )));
                            };
                            let coerced = coerce_field(
                                schema,
                                f,
                                fv.clone(),
                                &format!("{at}.{k}"),
                                depth + 1,
                            )?;
                            out.insert(&f.name, coerced);
                        }
                        Ok(Val::Obj(out))
                    }
                    other => Ok(other),
                },
            }
        }
    }
}

fn validate_field(
    schema: &Schema,
    field: &Field,
    v: &Val,
    at: &str,
    floats: FloatPolicy,
) -> Result<(), FigError> {
    let v = v.view();
    if !field.is_array {
        return validate_value(schema, field.type_id, v, at, floats);
    }
    match v {
        Val::Bytes(_) if field.type_id == TYPE_BYTE => Ok(()),
        Val::Arr(items) => {
            if u32::try_from(items.len()).is_err() {
                return Err(FigError::malformed(format!("{at}: list too long")));
            }
            for (i, item) in items.iter().enumerate() {
                validate_value(schema, field.type_id, item, &format!("{at}[{i}]"), floats)?;
            }
            Ok(())
        }
        _ => Err(FigError::malformed(format!(
            "{at}: {} must be a list",
            field.name
        ))),
    }
}

fn validate_value(
    schema: &Schema,
    type_id: i32,
    v: &Val,
    at: &str,
    floats: FloatPolicy,
) -> Result<(), FigError> {
    let v = v.view();
    match type_id {
        TYPE_BOOL => match v {
            Val::Bool(_) => Ok(()),
            _ => Err(FigError::malformed(format!("{at}: expected bool"))),
        },
        TYPE_BYTE => match v {
            Val::Int(i) if u8::try_from(*i).is_ok() => Ok(()),
            _ => Err(FigError::malformed(format!(
                "{at}: expected byte (0..=255)"
            ))),
        },
        TYPE_INT => match v {
            Val::Int(i) if i32::try_from(*i).is_ok() => Ok(()),
            _ => Err(FigError::malformed(format!("{at}: expected 32-bit int"))),
        },
        TYPE_UINT => match v {
            Val::Int(i) if u32::try_from(*i).is_ok() => Ok(()),
            _ => Err(FigError::malformed(format!("{at}: expected 32-bit uint"))),
        },
        TYPE_INT64 | TYPE_UINT64 => match v {
            Val::Int(_) => Ok(()),
            _ => Err(FigError::malformed(format!(
                "{at}: expected 64-bit integer"
            ))),
        },
        TYPE_FLOAT => match v {
            Val::Float(f)
                if matches!(floats, FloatPolicy::PreserveExisting) || is_finite_f32(*f) =>
            {
                Ok(())
            }
            Val::Float(_) => Err(FigError::malformed(format!(
                "{at}: expected finite 32-bit float"
            ))),
            Val::Int(_) => Ok(()),
            _ => Err(FigError::malformed(format!("{at}: expected float"))),
        },
        TYPE_STRING => match v {
            Val::Str(_) => Ok(()),
            _ => Err(FigError::malformed(format!("{at}: expected string"))),
        },
        other => {
            let Some(def) = usize::try_from(other).ok().and_then(|i| schema.defs.get(i)) else {
                return Err(FigError::malformed(format!("{at}: unknown type {other}")));
            };
            match def.kind {
                DefKind::Enum => {
                    let Val::Enum(variant) = v else {
                        return Err(FigError::malformed(format!(
                            "{at}: expected enum {}",
                            def.name
                        )));
                    };
                    if def.field(variant).is_none() {
                        return Err(FigError::malformed(format!(
                            "{at}: {} has no variant {variant}",
                            def.name
                        )));
                    }
                    Ok(())
                }
                DefKind::Struct if matches!(v, Val::Guid(_)) => {
                    let ok = def.fields.len() == 2
                        && def.field("sessionID").is_some()
                        && def.field("localID").is_some();
                    if ok {
                        Ok(())
                    } else {
                        Err(FigError::malformed(format!(
                            "{at}: {} is not a GUID struct",
                            def.name
                        )))
                    }
                }
                DefKind::Struct => {
                    for f in &def.fields {
                        let Some(fv) = v.get(&f.name) else {
                            return Err(FigError::malformed(format!(
                                "{at}: struct {} missing required field {}",
                                def.name, f.name
                            )));
                        };
                        validate_field(schema, f, fv, &format!("{at}.{}", f.name), floats)?;
                    }
                    // The writer encodes only the schema's fields, so an extra one would be lost.
                    if let Val::Obj(obj) = v
                        && let Some(k) = obj.keys().find(|k| def.field(k).is_none())
                    {
                        return Err(FigError::malformed(format!(
                            "{at}: struct {} has no field {k}",
                            def.name
                        )));
                    }
                    Ok(())
                }
                DefKind::Message => {
                    let Val::Obj(obj) = v else {
                        return Err(FigError::malformed(format!(
                            "{at}: expected object for {}",
                            def.name
                        )));
                    };
                    for (k, fv) in obj.iter() {
                        let Some(f) = def.field(k) else {
                            return Err(FigError::malformed(format!(
                                "{at}: {} has no field {k}",
                                def.name
                            )));
                        };
                        validate_field(schema, f, fv, &format!("{at}.{k}"), floats)?;
                    }
                    Ok(())
                }
            }
        }
    }
}
