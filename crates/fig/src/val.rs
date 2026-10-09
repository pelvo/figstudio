//! Owned, schema-independent copy of a decoded Kiwi value.
//!
//! Objects are small vectors of `(field name, value)` pairs. Field and enum names come from
//! the file schema and are interned once per process, so a document with millions of fields
//! does not allocate a string per field.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Enum(&'static str),
    Bytes(Vec<u8>),
    Arr(Vec<Val>),
    Obj(Obj),
}

/// Returns a `'static` copy of `s`, shared by every caller. Meant for schema identifiers
/// (field and enum names), which are a small fixed set; each distinct string is kept once.
pub fn intern(s: &str) -> &'static str {
    static TABLE: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
    let mut table = TABLE
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(&found) = table.get(s) {
        return found;
    }
    let leaked: &'static str = Box::leak(s.to_owned().into_boxed_str());
    table.insert(leaked);
    leaked
}

/// Object fields in decode order. Lookups are linear; objects have a handful of fields.
#[derive(Clone, Debug, Default)]
pub struct Obj(Vec<(&'static str, Val)>);

impl Obj {
    pub fn new() -> Obj {
        Obj(Vec::new())
    }

    pub fn with_capacity(n: usize) -> Obj {
        Obj(Vec::with_capacity(n))
    }

    pub fn get(&self, key: &str) -> Option<&Val> {
        self.0.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Val> {
        self.0.iter_mut().find(|(k, _)| *k == key).map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Sets `key`, replacing an existing value in place. Returns the old value.
    pub fn insert(&mut self, key: &str, value: Val) -> Option<Val> {
        if let Some(slot) = self.get_mut(key) {
            return Some(std::mem::replace(slot, value));
        }
        self.0.push((intern(key), value));
        None
    }

    /// Adds a field known not to be present yet (used by the decoder).
    pub(crate) fn push(&mut self, key: &'static str, value: Val) {
        self.0.push((key, value));
    }

    pub fn remove(&mut self, key: &str) -> Option<Val> {
        let i = self.0.iter().position(|(k, _)| *k == key)?;
        Some(self.0.remove(i).1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&'static str, &Val)> {
        self.0.iter().map(|(k, v)| (*k, v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.0.iter().map(|(k, _)| *k)
    }

    pub fn values(&self) -> impl Iterator<Item = &Val> {
        self.0.iter().map(|(_, v)| v)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn shrink_to_fit(&mut self) {
        self.0.shrink_to_fit();
    }
}

/// Field order does not matter for equality.
impl PartialEq for Obj {
    fn eq(&self, other: &Obj) -> bool {
        self.len() == other.len() && self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl<const N: usize> From<[(&str, Val); N]> for Obj {
    fn from(fields: [(&str, Val); N]) -> Obj {
        let mut o = Obj::with_capacity(N);
        for (k, v) in fields {
            o.insert(k, v);
        }
        o
    }
}

/// Structural equality. Floats compare by value, and NaN equals NaN with the same bits, so a
/// document always equals an unchanged copy of itself (Figma stores NaN in some fields).
impl PartialEq for Val {
    fn eq(&self, other: &Val) -> bool {
        match (self, other) {
            (Val::Bool(a), Val::Bool(b)) => a == b,
            (Val::Int(a), Val::Int(b)) => a == b,
            (Val::Float(a), Val::Float(b)) => a == b || a.to_bits() == b.to_bits(),
            (Val::Str(a), Val::Str(b)) => a == b,
            (Val::Enum(a), Val::Enum(b)) => a == b,
            (Val::Bytes(a), Val::Bytes(b)) => a == b,
            (Val::Arr(a), Val::Arr(b)) => a == b,
            (Val::Obj(a), Val::Obj(b)) => a == b,
            _ => false,
        }
    }
}

/// Node identifier: `sessionID:localID`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Guid(pub u32, pub u32);

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.0, self.1)
    }
}

impl std::str::FromStr for Guid {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (a, b) = s
            .split_once(':')
            .ok_or_else(|| format!("node id must look like 12:34, got {s}"))?;
        let parse = |x: &str| x.parse::<u32>().map_err(|_| format!("bad node id {s}"));
        Ok(Guid(parse(a)?, parse(b)?))
    }
}

impl Guid {
    pub fn to_val(self) -> Val {
        Val::Obj(Obj::from([
            ("sessionID", Val::Int(i64::from(self.0))),
            ("localID", Val::Int(i64::from(self.1))),
        ]))
    }
}

impl Val {
    pub fn get(&self, key: &str) -> Option<&Val> {
        match self {
            Val::Obj(o) => o.get(key),
            _ => None,
        }
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Val> {
        match self {
            Val::Obj(o) => o.get_mut(key),
            _ => None,
        }
    }

    /// Sets a field on an object; does nothing for other values.
    pub fn set(&mut self, key: &str, value: Val) {
        if let Val::Obj(o) = self {
            o.insert(key, value);
        }
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Val> {
        keys.iter().try_fold(self, |v, k| v.get(k))
    }

    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Val::Int(i) => Some(*i as f32),
            Val::Float(f) => Some(*f as f32),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self {
            Val::Int(i) => u32::try_from(*i).ok(),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Val::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Strings and enum names.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Val::Str(s) => Some(s),
            Val::Enum(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Val::Bytes(b) => Some(b),
            Val::Arr(a) if a.is_empty() => Some(&[]),
            _ => None,
        }
    }

    /// Array items; empty for anything that is not an array.
    pub fn items(&self) -> &[Val] {
        match self {
            Val::Arr(a) => a,
            _ => &[],
        }
    }

    pub fn num(&self, key: &str) -> Option<f32> {
        self.get(key)?.as_f32()
    }

    pub fn flag(&self, key: &str) -> Option<bool> {
        self.get(key)?.as_bool()
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        self.get(key)?.as_str()
    }

    pub fn as_guid(&self) -> Option<Guid> {
        Some(Guid(
            self.get("sessionID")?.as_u32()?,
            self.get("localID")?.as_u32()?,
        ))
    }

    /// Shallow merge of `patch` object fields over `self`. `symbolData` is merged one level deeper,
    /// so an instance-swap override does not drop the nested instance's own overrides.
    pub fn merged(&self, patch: &Val) -> Val {
        let (Val::Obj(base), Val::Obj(over)) = (self, patch) else {
            return self.clone();
        };
        let mut out = base.clone();
        for (k, v) in over.iter() {
            if k == "guidPath" {
                continue;
            }
            if k == "symbolData"
                && let Some(existing @ Val::Obj(_)) = base.get(k)
            {
                out.insert(k, existing.merged(v));
                continue;
            }
            out.insert(k, v.clone());
        }
        Val::Obj(out)
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
