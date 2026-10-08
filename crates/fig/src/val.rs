//! Owned, schema-independent copy of a decoded Kiwi value.
//!
//! `kiwi_schema::Value` borrows field names from its `Schema`, which makes it
//! awkward to keep around in an app. `Val` owns everything.

use std::collections::BTreeMap;
use std::fmt;

use kiwi_schema::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Enum(String),
    Bytes(Vec<u8>),
    Arr(Vec<Val>),
    Obj(BTreeMap<String, Val>),
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

impl Val {
    pub fn from_kiwi(value: &Value) -> Val {
        match value {
            Value::Bool(b) => Val::Bool(*b),
            Value::Byte(b) => Val::Int(i64::from(*b)),
            Value::Int(i) => Val::Int(i64::from(*i)),
            Value::UInt(u) => Val::Int(i64::from(*u)),
            Value::Float(f) => Val::Float(f64::from(*f)),
            Value::String(s) => Val::Str(s.clone()),
            Value::Int64(i) => Val::Int(*i),
            Value::UInt64(u) => Val::Int(*u as i64),
            Value::Array(items) => {
                if !items.is_empty() && items.iter().all(|i| matches!(i, Value::Byte(_))) {
                    Val::Bytes(items.iter().map(Value::as_byte).collect())
                } else {
                    Val::Arr(items.iter().map(Val::from_kiwi).collect())
                }
            }
            Value::Enum(_, name) => Val::Enum((*name).to_string()),
            Value::Object(_, fields) => Val::Obj(
                fields
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), Val::from_kiwi(v)))
                    .collect(),
            ),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Val> {
        match self {
            Val::Obj(map) => map.get(key),
            _ => None,
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
            Val::Str(s) | Val::Enum(s) => Some(s),
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
        for (k, v) in over {
            if k == "guidPath" {
                continue;
            }
            if k == "symbolData"
                && let Some(existing @ Val::Obj(_)) = base.get(k)
            {
                out.insert(k.clone(), existing.merged(v));
                continue;
            }
            out.insert(k.clone(), v.clone());
        }
        Val::Obj(out)
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
