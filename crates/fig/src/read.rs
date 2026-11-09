//! Kiwi decoding straight into `Val`, driven by the file's own schema.
//!
//! Decoding directly (instead of through `kiwi_schema::Value`) avoids a second copy of the
//! whole document and a hash map per object, which dominated memory on large files.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use kiwi_schema::{
    ByteBuffer, DefKind, Field, Schema, TYPE_BOOL, TYPE_BYTE, TYPE_FLOAT, TYPE_INT, TYPE_INT64,
    TYPE_STRING, TYPE_UINT, TYPE_UINT64,
};

use crate::val::{Guid, Lazy, Obj, Val, intern, intern_schema};

/// Deepest nesting accepted; real documents stay far below this.
const MAX_DEPTH: u32 = 256;
const MAX_DECODED_ITEMS: usize = 100_000_000;
const MAX_SCHEMA_IDENTIFIERS: usize = 100_000;
const MAX_SCHEMA_IDENTIFIER_BYTES: usize = 1024;

pub(crate) type R<T> = Result<T, String>;

fn bad<T>(r: Result<T, ()>) -> R<T> {
    r.map_err(|()| "the document data is truncated or invalid".to_string())
}

/// Node fields kept encoded until first read ([`Lazy`]). Together over half of a large
/// document's decoded tree; the first three are read only when an instance is resolved,
/// `derivedTextData` (stored glyph outlines) only when that text is drawn or selected.
const LAZY_FIELDS: [&str; 4] = [
    "derivedSymbolData",
    "parameterConsumptionMap",
    "variableConsumptionMap",
    "derivedTextData",
];

/// How a node record's field is read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Plain,
    /// Equal encoded values decode once and are shared ([`Val::Shared`]): nodes repeat the
    /// same paints, transforms and sizes thousands of times.
    Share,
    /// Kept encoded until first read ([`Lazy`]); equal bytes share one.
    Lazy,
}

/// Node fields never shared: unique per node, so sharing would only cost lookups.
const UNIQUE_FIELDS: [&str; 2] = ["guid", "parentIndex"];

/// What decoding needs from a file's schema. Shared (`Arc`) by the document's lazy fields so
/// they can decode themselves later, and by the writer to tell their schema is its own.
pub(crate) struct Decoder {
    pub schema: Schema,
    /// Interned field names per definition, in `def.fields` order.
    names: Vec<Vec<&'static str>>,
    /// Enum value -> interned name, per definition.
    enums: Vec<HashMap<u32, &'static str>>,
    /// Per definition: a `{sessionID, localID}` struct, decoded as `Val::Guid` (no heap).
    guids: Vec<bool>,
    /// Per definition and field: how it is read inside node records.
    modes: Vec<Box<[Mode]>>,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Decoder({} definitions)", self.schema.defs.len())
    }
}

/// First occurrences of shared node fields: (definition, field, hash of the bytes) -> the
/// byte range in the message and the value made from it. A second value with the same hash
/// is not shared (and not kept).
type Seen = HashMap<(usize, usize, u64), (usize, usize, Val)>;

pub(crate) struct Reader {
    pub dec: Arc<Decoder>,
    decoded_items: Cell<usize>,
    /// Set while node records are read: their fields are read by [`Decoder::modes`].
    pub nodes: Cell<bool>,
    /// Only while node records are read; [`Reader::end_nodes`] frees it.
    seen: RefCell<Seen>,
    /// Decoding bytes [`Reader::skip_field`] already checked at open. List lengths were
    /// checked then against the whole document; the field's own bytes can be fewer (a list of
    /// zero-byte empty structs), so the check is not repeated.
    walked: bool,
}

/// Decodes a field kept by [`Lazy`]: field `field` of definition `def`, at node-field depth.
pub(crate) fn decode_field(dec: &Arc<Decoder>, def: usize, field: usize, bytes: &[u8]) -> R<Val> {
    let Some(f) = dec.schema.defs.get(def).and_then(|d| d.fields.get(field)) else {
        return Err("the schema has no such field".into());
    };
    let reader = Reader {
        walked: true,
        ..Reader::with(dec.clone())
    };
    reader.field(&mut ByteBuffer::new(bytes), f, 2)
}

/// Fields of nested messages kept encoded until first read, as (message, field): an
/// instance's overrides are read only when the instance is entered.
const LAZY_NESTED: [(&str, &str); 1] = [("SymbolData", "symbolOverrides")];

/// How each field is read in node records: the record's [`LAZY_FIELDS`] and the nested
/// [`LAZY_NESTED`] lazily, the record's other fields that own memory shared, everything else
/// plainly.
fn modes(schema: &Schema) -> Vec<Box<[Mode]>> {
    let node = schema
        .def("Message")
        .and_then(|m| m.field("nodeChanges"))
        .and_then(|f| usize::try_from(f.type_id).ok());
    let owns_memory = |f: &Field| {
        f.is_array
            || f.type_id == TYPE_STRING
            || usize::try_from(f.type_id)
                .ok()
                .and_then(|i| schema.defs.get(i))
                .is_some_and(|d| d.kind != DefKind::Enum && !is_guid(d))
    };
    let mode = |f: &Field| {
        if LAZY_FIELDS.contains(&f.name.as_str()) {
            Mode::Lazy
        } else if owns_memory(f) && !UNIQUE_FIELDS.contains(&f.name.as_str()) {
            Mode::Share
        } else {
            Mode::Plain
        }
    };
    schema
        .defs
        .iter()
        .enumerate()
        .map(
            |(i, d)| match node == Some(i) && d.kind == DefKind::Message {
                true => d.fields.iter().map(mode).collect(),
                false => d
                    .fields
                    .iter()
                    .map(
                        |f| match LAZY_NESTED.contains(&(d.name.as_str(), f.name.as_str())) {
                            true if d.kind == DefKind::Message => Mode::Lazy,
                            _ => Mode::Plain,
                        },
                    )
                    .collect(),
            },
        )
        .collect()
}

/// A struct of exactly `sessionID: uint, localID: uint`, in that order.
fn is_guid(def: &kiwi_schema::Def) -> bool {
    let uint = |f: &Field, name: &str| f.name == name && f.type_id == TYPE_UINT && !f.is_array;
    def.kind == DefKind::Struct
        && matches!(&def.fields[..], [a, b] if uint(a, "sessionID") && uint(b, "localID"))
}

impl Reader {
    fn with(dec: Arc<Decoder>) -> Reader {
        Reader {
            dec,
            decoded_items: Cell::new(0),
            nodes: Cell::new(false),
            seen: RefCell::default(),
            walked: false,
        }
    }

    pub fn schema(&self) -> &Schema {
        &self.dec.schema
    }

    pub fn new(schema: Schema) -> R<Self> {
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
        // Documents with the same schema share one decoder, so their still-encoded fields can
        // be told equal (and written) by comparing bytes; see `Val`'s `PartialEq`.
        static LIVE: OnceLock<Mutex<Vec<Weak<Decoder>>>> = OnceLock::new();
        let mut live = LIVE
            .get_or_init(Default::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        live.retain(|d| d.strong_count() > 0);
        if let Some(dec) = live
            .iter()
            .filter_map(Weak::upgrade)
            .find(|d| d.schema == schema)
        {
            return Ok(Reader::with(dec));
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
        let guids = schema.defs.iter().map(is_guid).collect();
        let modes = modes(&schema);
        let dec = Arc::new(Decoder {
            schema,
            names,
            enums,
            guids,
            modes,
        });
        live.push(Arc::downgrade(&dec));
        Ok(Reader::with(dec))
    }

    /// Spends `n` of the decoded-items budget.
    fn count(&self, n: usize) -> R<()> {
        let total = self.decoded_items.get().saturating_add(n);
        if total > MAX_DECODED_ITEMS {
            return Err("the document contains too many decoded items".into());
        }
        self.decoded_items.set(total);
        Ok(())
    }

    /// Reads past a field without building it, with every check [`Reader::field`] makes (and
    /// the same budget), so decoding the skipped bytes later cannot fail.
    fn skip_field(&self, bb: &mut ByteBuffer, field: &Field, depth: u32) -> R<()> {
        if !field.is_array {
            return self.skip(bb, field.type_id, depth);
        }
        let len = self.len(bb)?;
        self.count(len)?;
        if field.type_id == TYPE_BYTE {
            bad(bb.read_bytes(len))?;
            return Ok(());
        }
        self.count(len)?;
        for _ in 0..len {
            self.skip(bb, field.type_id, depth)?;
        }
        Ok(())
    }

    fn skip(&self, bb: &mut ByteBuffer, type_id: i32, depth: u32) -> R<()> {
        match type_id {
            TYPE_BOOL => drop(bad(bb.read_bool())?),
            TYPE_BYTE => drop(bad(bb.read_byte())?),
            TYPE_INT => drop(bad(bb.read_var_int())?),
            TYPE_UINT => drop(bad(bb.read_var_uint())?),
            TYPE_FLOAT => drop(bad(bb.read_var_float())?),
            TYPE_STRING => drop(bad(bb.read_string())?),
            TYPE_INT64 => drop(bad(bb.read_var_int64())?),
            TYPE_UINT64 => drop(bad(bb.read_var_uint64())?),
            _ => {
                let (idx, def) = self.def(type_id, depth)?;
                match def.kind {
                    DefKind::Enum => {
                        let v = bad(bb.read_var_uint())?;
                        if !self.dec.enums[idx].contains_key(&v) {
                            return Err(format!("{} has no value {v}", def.name));
                        }
                    }
                    DefKind::Struct => {
                        for f in &def.fields {
                            self.skip_field(bb, f, depth + 1)?;
                        }
                    }
                    DefKind::Message => loop {
                        let tag = bad(bb.read_var_uint())?;
                        if tag == 0 {
                            break;
                        }
                        let Some(&i) = def.field_value_to_index.get(&tag) else {
                            return Err(format!("{} has no field {tag}", def.name));
                        };
                        self.skip_field(bb, &def.fields[i], depth + 1)?;
                    },
                }
            }
        }
        Ok(())
    }

    /// Node records are read: stop sharing and free what was kept to share.
    pub fn end_nodes(&self) {
        self.nodes.set(false);
        self.seen.take();
    }

    /// Reads a shared or lazy field of a node record: equal bytes of the same field give the
    /// value made the first time (a clone of one `Arc`), so they are neither decoded nor
    /// stored again.
    fn node_field(
        &self,
        bb: &mut ByteBuffer,
        mode: Mode,
        def: usize,
        field: usize,
        depth: u32,
    ) -> R<Val> {
        let f = &self.dec.schema.defs[def].fields[field];
        let (start, budget) = (bb.index(), self.decoded_items.get());
        self.skip_field(bb, f, depth)?;
        let (all, end) = (bb.data(), bb.index());
        let bytes = &all[start..end];
        let mut hasher = std::hash::DefaultHasher::new();
        std::hash::Hash::hash(bytes, &mut hasher);
        let key = (def, field, std::hash::Hasher::finish(&hasher));
        let taken = match self.seen.borrow().get(&key) {
            Some((s, e, v)) if &all[*s..*e] == bytes => return Ok(v.clone()),
            Some(_) => true,
            None => false,
        };
        let v = if mode == Mode::Lazy {
            Val::Lazy(Arc::new(Lazy::new(
                self.dec.clone(),
                def,
                field,
                bytes.into(),
            )))
        } else {
            // Read again over the same buffer, so the byte ranges nested node records record
            // stay ranges of `all`. It spends the budget the skip spent, not twice as much.
            self.decoded_items.set(budget);
            let mut again = ByteBuffer::new(all);
            bad(again.read_bytes(start))?;
            let v = self.field(&mut again, f, depth)?;
            match v {
                Val::Str(_) | Val::Bytes(_) | Val::Arr(_) | Val::Obj(_) => Val::Shared(Arc::new(v)),
                v => v,
            }
        };
        if !taken {
            self.seen.borrow_mut().insert(key, (start, end, v.clone()));
        }
        Ok(v)
    }

    /// The definition `type_id` names, checked, with its index.
    fn def(&self, type_id: i32, depth: u32) -> R<(usize, &kiwi_schema::Def)> {
        if depth > MAX_DEPTH {
            return Err("the document nests too deeply".into());
        }
        let schema = &self.dec.schema;
        match usize::try_from(type_id)
            .ok()
            .filter(|i| *i < schema.defs.len())
        {
            Some(idx) => Ok((idx, &schema.defs[idx])),
            None => Err(format!("the schema has no type {type_id}")),
        }
    }

    /// Array length, checked against the bytes left so a corrupt length cannot allocate wildly.
    pub fn len(&self, bb: &mut ByteBuffer) -> R<usize> {
        let len = usize::try_from(bad(bb.read_var_uint())?)
            .map_err(|_| "the document list is too large".to_string())?;
        let left = bb.data().len().saturating_sub(bb.index());
        if len > left && !self.walked {
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
            let total = self.decoded_items.get().saturating_add(len);
            if total > MAX_DECODED_ITEMS {
                return Err("the document contains too many decoded items".into());
            }
            self.decoded_items.set(total);
            return Ok(Val::Bytes(bad(bb.read_bytes(len))?.to_vec()));
        }
        let total = self.decoded_items.get().saturating_add(len);
        if total > MAX_DECODED_ITEMS {
            return Err("the document contains too many decoded items".into());
        }
        self.decoded_items.set(total);
        let total = self.decoded_items.get().saturating_add(len);
        if total > MAX_DECODED_ITEMS {
            return Err("the document contains too many decoded items".into());
        }
        self.decoded_items.set(total);
        let mut items = Vec::new();
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
                let (idx, def) = self.def(type_id, depth)?;
                match def.kind {
                    DefKind::Enum => {
                        let v = bad(bb.read_var_uint())?;
                        let name = self.dec.enums[idx]
                            .get(&v)
                            .ok_or_else(|| format!("{} has no value {v}", def.name))?;
                        Val::Enum(name)
                    }
                    DefKind::Struct if self.dec.guids[idx] => {
                        Val::Guid(Guid(bad(bb.read_var_uint())?, bad(bb.read_var_uint())?))
                    }
                    DefKind::Struct => {
                        let mut o = Obj::with_capacity(def.fields.len());
                        for (i, f) in def.fields.iter().enumerate() {
                            o.push(self.dec.names[idx][i], self.field(bb, f, depth + 1)?);
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
                            let key = self.dec.names[idx][i];
                            let v = match self.dec.modes[idx][i] {
                                Mode::Plain => self.field(bb, &def.fields[i], depth + 1)?,
                                _ if !self.nodes.get() => {
                                    self.field(bb, &def.fields[i], depth + 1)?
                                }
                                mode => self.node_field(bb, mode, idx, i, depth + 1)?,
                            };
                            // A repeated field keeps the last value, like the reference decoder.
                            match o.get_raw_mut(key) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use kiwi_schema::{Def, Schema};

    /// A list of zero-byte items (structs with no fields) is checked at open against the
    /// bytes left in the whole document, but a lazy field holds only its own bytes; decoding
    /// it later must still succeed, or its value would silently read as empty.
    #[test]
    fn a_lazy_list_of_empty_structs_decodes_like_it_was_checked() {
        let schema = Schema::new(vec![
            Def::new("Empty".into(), DefKind::Struct, Vec::new()),
            Def::new(
                "Holder".into(),
                DefKind::Struct,
                vec![Field {
                    name: "items".into(),
                    type_id: 0,
                    is_array: true,
                    value: 0,
                }],
            ),
        ]);
        let reader = Reader::new(schema).unwrap();
        let field = &reader.schema().defs[1].fields[0];
        // At open: three empty items, then more of the document.
        let mut bb = ByteBuffer::new(&[3, 0, 0, 0]);
        reader.skip_field(&mut bb, field, 2).unwrap();
        assert_eq!(bb.index(), 1);
        // Later: the field's own byte only.
        let val = decode_field(&reader.dec, 1, 0, &[3]).unwrap();
        assert_eq!(val.items().len(), 3);
    }
}
