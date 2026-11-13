//! Up-front Kiwi schema validation for [`NodePatch`](super::NodePatch) writes.
//!
//! Ensures every mutated or created field conforms to the document's own Kiwi schema
//! before [`Document::apply_txn`](crate::Document::apply_txn) mutates any state.

use kiwi_schema::{
    DefKind, Field, Schema, TYPE_BOOL, TYPE_BYTE, TYPE_FLOAT, TYPE_INT, TYPE_INT64, TYPE_STRING,
    TYPE_UINT, TYPE_UINT64,
};

use crate::{FigError, Val};

/// Validates a full `NodeChange` object against `schema`.
pub fn validate_node_data(schema: &Schema, data: &Val, at: &str) -> Result<(), FigError> {
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
            return validate_value(schema, idx as i32, data, at);
        }
        return Err(FigError::malformed("schema has no NodeChange definition"));
    };
    if data.get("guid").and_then(Val::as_guid).is_none() {
        return Err(FigError::malformed(format!("{at}: missing valid guid")));
    }
    validate_value(schema, nc_idx as i32, data, at)
}

/// Validates a single top-level field write `(key, val)` on `NodeChange`.
pub fn validate_node_field(
    schema: &Schema,
    key: &str,
    val: &Val,
    at: &str,
) -> Result<(), FigError> {
    let def = schema
        .def("NodeChange")
        .or_else(|| {
            let f = schema.def("Message")?.field("nodeChanges")?;
            schema.defs.get(usize::try_from(f.type_id).ok()?)
        })
        .ok_or_else(|| FigError::malformed("schema has no NodeChange definition"))?;
    let Some(field) = def.field(key) else {
        return Err(FigError::malformed(format!(
            "{at}: {} has no field {key}",
            def.name
        )));
    };
    validate_field(schema, field, val, &format!("{at}.{key}"))
}

fn validate_field(schema: &Schema, field: &Field, v: &Val, at: &str) -> Result<(), FigError> {
    let v = v.view();
    if !field.is_array {
        return validate_value(schema, field.type_id, v, at);
    }
    match v {
        Val::Bytes(_) if field.type_id == TYPE_BYTE => Ok(()),
        Val::Arr(items) => {
            if u32::try_from(items.len()).is_err() {
                return Err(FigError::malformed(format!("{at}: list too long")));
            }
            for (i, item) in items.iter().enumerate() {
                validate_value(schema, field.type_id, item, &format!("{at}[{i}]"))?;
            }
            Ok(())
        }
        _ => Err(FigError::malformed(format!(
            "{at}: {} must be a list",
            field.name
        ))),
    }
}

fn validate_value(schema: &Schema, type_id: i32, v: &Val, at: &str) -> Result<(), FigError> {
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
            Val::Float(_) | Val::Int(_) => Ok(()),
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
                        validate_field(schema, f, fv, &format!("{at}.{}", f.name))?;
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
                        validate_field(schema, f, fv, &format!("{at}.{k}"))?;
                    }
                    Ok(())
                }
            }
        }
    }
}
