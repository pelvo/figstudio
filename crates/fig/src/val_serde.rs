//! Natural JSON projection: enums become strings and byte buffers become arrays.

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser::SerializeMap};

/// JSON cannot carry nonfinite floats; encode them losslessly as their bits.
const FLOAT_BITS_KEY: &str = "$float_bits";

fn float_bits_json(value: f64) -> String {
    format!("{value:016x}", value = value.to_bits())
}

fn float_from_bits_json(value: &str) -> Option<f64> {
    (value.len() == 16 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| u64::from_str_radix(value, 16).ok().map(f64::from_bits))
        .flatten()
}

use crate::val::{Obj, Val, intern_schema};

impl Serialize for Val {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Bool(v) => serializer.serialize_bool(*v),
            Self::Int(v) => serializer.serialize_i64(*v),
            Self::Float(v) if v.is_finite() => serializer.serialize_f64(*v),
            Self::Float(v) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(FLOAT_BITS_KEY, &float_bits_json(*v))?;
                map.end()
            }
            Self::Str(v) => serializer.serialize_str(v),
            Self::Enum(v) => serializer.serialize_str(v),
            Self::Bytes(v) => v.serialize(serializer),
            Self::Arr(v) => v.serialize(serializer),
            Self::Obj(v) => v.serialize(serializer),
        }
    }
}

impl Serialize for Obj {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = self.iter().collect::<Vec<_>>();
        fields.sort_by_key(|(key, _)| *key);
        let mut map = serializer.serialize_map(Some(fields.len()))?;
        for (key, value) in fields {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Val {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> de::Visitor<'de> for ValueVisitor {
    type Value = Val;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a boolean, signed number, string, array or object")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Val, E> {
        Ok(Val::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Val, E> {
        Ok(Val::Int(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Val, E> {
        i64::try_from(v).map(Val::Int).map_err(E::custom)
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Val, E> {
        if !v.is_finite() {
            return Err(E::custom("nonfinite value is not JSON"));
        }
        Ok(Val::Float(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Val, E> {
        Ok(Val::Str(v.into()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<Val, E> {
        Ok(Val::Str(v))
    }

    fn visit_seq<A: de::SeqAccess<'de>>(self, mut seq: A) -> Result<Val, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element()? {
            values.push(value);
        }
        Ok(Val::Arr(values))
    }

    fn visit_map<A: de::MapAccess<'de>>(self, map: A) -> Result<Val, A::Error> {
        let obj = ObjectVisitor.visit_map(map)?;
        // A sole $float_bits key with 16 hex digits decodes a nonfinite float;
        // anything else (extra keys, non-hex value) is an ordinary object.
        if obj.len() == 1
            && let Some((key, Val::Str(value))) = obj.iter().next()
            && key == FLOAT_BITS_KEY
            && let Some(float) = float_from_bits_json(value.as_str())
        {
            return Ok(Val::Float(float));
        }
        Ok(Val::Obj(obj))
    }
}

impl<'de> Deserialize<'de> for Obj {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(ObjectVisitor)
    }
}

struct ObjectVisitor;

impl<'de> de::Visitor<'de> for ObjectVisitor {
    type Value = Obj;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an object with unique keys within the schema identifier budget")
    }

    fn visit_map<A: de::MapAccess<'de>>(self, mut map: A) -> Result<Obj, A::Error> {
        let mut fields = BTreeMap::<String, Val>::new();
        while let Some((key, value)) = map.next_entry()? {
            if fields.insert(key, value).is_some() {
                return Err(de::Error::custom("duplicate object key"));
            }
        }
        intern_schema(fields.keys().map(String::as_str)).map_err(de::Error::custom)?;
        let mut object = Obj::with_capacity(fields.len());
        for (key, value) in fields {
            object.insert(&key, value);
        }
        Ok(object)
    }
}
