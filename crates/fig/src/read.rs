//! Kiwi decoding straight into `Val`, driven by the file's own schema.
//!
//! Decoding directly (instead of through `kiwi_schema::Value`) avoids a second copy of the
//! whole document and a hash map per object, which dominated memory on large files.

use std::collections::HashMap;

use kiwi_schema::{
    ByteBuffer, DefKind, Field, Schema, TYPE_BOOL, TYPE_BYTE, TYPE_FLOAT, TYPE_INT, TYPE_INT64,
    TYPE_STRING, TYPE_UINT, TYPE_UINT64,
};

use crate::val::{Obj, Val, intern, intern_schema};

/// Deepest nesting accepted; real documents stay far below this.
const MAX_DEPTH: u32 = 256;
const MAX_SCHEMA_IDENTIFIERS: usize = 4096;
const MAX_SCHEMA_IDENTIFIER_BYTES: usize = 256;

pub(crate) type R<T> = Result<T, String>;

fn bad<T>(r: Result<T, ()>) -> R<T> {
    r.map_err(|()| "the document data is truncated or invalid".to_string())
}

pub(crate) struct Reader<'s> {
    pub schema: &'s Schema,
    /// Interned field names per definition, in `def.fields` order.
    names: Vec<Vec<&'static str>>,
    /// Enum value -> interned name, per definition.
    enums: Vec<HashMap<u32, &'static str>>,
}

impl<'s> Reader<'s> {
    pub fn new(schema: &'s Schema) -> R<Self> {
        let identifier_count = schema.defs.iter().map(|d| d.fields.len()).sum::<usize>();
        if identifier_count > MAX_SCHEMA_IDENTIFIERS
            || schema.defs.iter().any(|d| {
                d.name.len() > MAX_SCHEMA_IDENTIFIER_BYTES
                    || d.fields
                        .iter()
                        .any(|f| f.name.len() > MAX_SCHEMA_IDENTIFIER_BYTES)
            })
        {
            return Err("schema has too many or oversized identifiers".into());
        }
        intern_schema(schema.defs.iter().flat_map(|d| {
            std::iter::once(d.name.as_str()).chain(d.fields.iter().map(|f| f.name.as_str()))
        }))?;
        let names = schema
            .defs
            .iter()
            .map(|d| d.fields.iter().map(|f| intern(&f.name)).collect())
            .collect();
        let enums = schema
            .defs
            .iter()
            .map(|d| match d.kind {
                DefKind::Enum => d
                    .fields
                    .iter()
                    .map(|f| (f.value, intern(&f.name)))
                    .collect(),
                _ => HashMap::new(),
            })
            .collect();
        Ok(Reader {
            schema,
            names,
            enums,
        })
    }

    /// Array length, checked against the bytes left so a corrupt length cannot allocate wildly.
    pub fn len(&self, bb: &mut ByteBuffer) -> R<usize> {
        let len = usize::try_from(bad(bb.read_var_uint())?)
            .map_err(|_| "the document list is too large".to_string())?;
        let left = bb.data().len().saturating_sub(bb.index());
        if len > left {
            return Err("the document data is truncated (list longer than the data left)".into());
        }
        Ok(len)
    }

    pub fn field(&self, bb: &mut ByteBuffer, field: &Field, depth: u32) -> R<Val> {
        if !field.is_array {
            return self.value(bb, field.type_id, depth);
        }
        let len = self.len(bb)?;
        if field.type_id == TYPE_BYTE {
            return Ok(Val::Bytes(bad(bb.read_bytes(len))?.to_vec()));
        }
        let mut items = Vec::with_capacity(len);
        for _ in 0..len {
            items.push(self.value(bb, field.type_id, depth)?);
        }
        Ok(Val::Arr(items))
    }

    pub fn value(&self, bb: &mut ByteBuffer, type_id: i32, depth: u32) -> R<Val> {
        Ok(match type_id {
            TYPE_BOOL => Val::Bool(bad(bb.read_bool())?),
            TYPE_BYTE => Val::Int(i64::from(bad(bb.read_byte())?)),
            TYPE_INT => Val::Int(i64::from(bad(bb.read_var_int())?)),
            TYPE_UINT => Val::Int(i64::from(bad(bb.read_var_uint())?)),
            TYPE_FLOAT => Val::Float(f64::from(bad(bb.read_var_float())?)),
            TYPE_STRING => Val::Str(bad(bb.read_string())?.into_owned()),
            TYPE_INT64 => Val::Int(bad(bb.read_var_int64())?),
            // Stored as the same 64 bits; the writer turns them back into the unsigned value.
            TYPE_UINT64 => Val::Int(bad(bb.read_var_uint64())? as i64),
            _ => {
                if depth > MAX_DEPTH {
                    return Err("the document nests too deeply".into());
                }
                let Some(idx) = usize::try_from(type_id)
                    .ok()
                    .filter(|i| *i < self.schema.defs.len())
                else {
                    return Err(format!("the schema has no type {type_id}"));
                };
                let def = &self.schema.defs[idx];
                match def.kind {
                    DefKind::Enum => {
                        let v = bad(bb.read_var_uint())?;
                        let name = self.enums[idx]
                            .get(&v)
                            .ok_or_else(|| format!("{} has no value {v}", def.name))?;
                        Val::Enum(name)
                    }
                    DefKind::Struct => {
                        let mut o = Obj::with_capacity(def.fields.len());
                        for (i, f) in def.fields.iter().enumerate() {
                            o.push(self.names[idx][i], self.field(bb, f, depth + 1)?);
                        }
                        Val::Obj(o)
                    }
                    DefKind::Message => {
                        let mut o = Obj::new();
                        loop {
                            let tag = bad(bb.read_var_uint())?;
                            if tag == 0 {
                                break;
                            }
                            let Some(&i) = def.field_value_to_index.get(&tag) else {
                                return Err(format!("{} has no field {tag}", def.name));
                            };
                            let v = self.field(bb, &def.fields[i], depth + 1)?;
                            let key = self.names[idx][i];
                            // A repeated field keeps the last value, like the reference decoder.
                            match o.get_mut(key) {
                                Some(slot) => *slot = v,
                                None => o.push(key, v),
                            }
                        }
                        o.shrink_to_fit();
                        Val::Obj(o)
                    }
                }
            }
        })
    }
}
