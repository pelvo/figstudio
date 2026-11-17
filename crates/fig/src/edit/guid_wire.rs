//! Serde wire helpers so `EditOp` JSON accepts `"12:34"`, `"12-34"`, or `[12, 34]` for `Guid`s
//! and serializes `Guid`s as canonical `"12:34"` strings without altering `Guid`'s core serde.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Guid;

pub fn parse_flex_guid(s: &str) -> Option<Guid> {
    let normalized = s.trim().replace('-', ":");
    normalized.parse::<Guid>().ok()
}

pub fn serialize<S: Serializer>(g: &Guid, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&g.to_string())
}

pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Guid, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Repr {
        Text(String),
        Pair([u32; 2]),
    }
    match Repr::deserialize(d)? {
        Repr::Text(t) => {
            parse_flex_guid(&t).ok_or_else(|| D::Error::custom(format!("invalid node id {t:?}")))
        }
        Repr::Pair([a, b]) => Ok(Guid(a, b)),
    }
}

pub mod opt {
    use super::*;

    pub fn serialize<S: Serializer>(g: &Option<Guid>, s: S) -> Result<S::Ok, S::Error> {
        match g {
            Some(id) => s.serialize_some(&id.to_string()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Guid>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum OptRepr {
            None,
            Text(String),
            Pair([u32; 2]),
        }
        match Option::<OptRepr>::deserialize(d)? {
            None | Some(OptRepr::None) => Ok(None),
            Some(OptRepr::Text(t)) => parse_flex_guid(&t)
                .map(Some)
                .ok_or_else(|| D::Error::custom(format!("invalid node id {t:?}"))),
            Some(OptRepr::Pair([a, b])) => Ok(Some(Guid(a, b))),
        }
    }
}

pub mod vec {
    use super::*;

    pub fn serialize<S: Serializer>(list: &[Guid], s: S) -> Result<S::Ok, S::Error> {
        let strs: Vec<String> = list.iter().map(Guid::to_string).collect();
        strs.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Guid>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Item {
            Text(String),
            Pair([u32; 2]),
        }
        let raw = Vec::<Item>::deserialize(d)?;
        raw.into_iter()
            .map(|it| match it {
                Item::Text(t) => parse_flex_guid(&t)
                    .ok_or_else(|| D::Error::custom(format!("invalid node id {t:?}"))),
                Item::Pair([a, b]) => Ok(Guid(a, b)),
            })
            .collect()
    }
}

pub mod opt_vec {
    use super::*;

    pub fn serialize<S: Serializer>(list: &Option<Vec<Guid>>, s: S) -> Result<S::Ok, S::Error> {
        match list {
            Some(v) => {
                let strs: Vec<String> = v.iter().map(Guid::to_string).collect();
                s.serialize_some(&strs)
            }
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<Guid>>, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Item {
            Text(String),
            Pair([u32; 2]),
        }
        let raw = Option::<Vec<Item>>::deserialize(d)?;
        match raw {
            None => Ok(None),
            Some(items) => items
                .into_iter()
                .map(|it| match it {
                    Item::Text(t) => parse_flex_guid(&t)
                        .ok_or_else(|| D::Error::custom(format!("invalid node id {t:?}"))),
                    Item::Pair([a, b]) => Ok(Guid(a, b)),
                })
                .collect::<Result<Vec<_>, _>>()
                .map(Some),
        }
    }
}
