//! JSON decoding that rejects duplicate object keys before they are discarded.

use crate::Result;
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::fmt;

struct Unique(Value);

struct UniqueJson;

impl<'de> Visitor<'de> for UniqueJson {
    type Value = Unique;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JSON with unique object keys")
    }

    fn visit_bool<E: serde::de::Error>(self, value: bool) -> std::result::Result<Unique, E> {
        Ok(Unique(value.into()))
    }

    fn visit_i64<E: serde::de::Error>(self, value: i64) -> std::result::Result<Unique, E> {
        Ok(Unique(value.into()))
    }

    fn visit_u64<E: serde::de::Error>(self, value: u64) -> std::result::Result<Unique, E> {
        Ok(Unique(value.into()))
    }

    fn visit_f64<E: serde::de::Error>(self, value: f64) -> std::result::Result<Unique, E> {
        Ok(Unique(
            serde_json::Number::from_f64(value)
                .ok_or_else(|| E::custom("invalid number"))?
                .into(),
        ))
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> std::result::Result<Unique, E> {
        Ok(Unique(value.into()))
    }

    fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Unique, E> {
        Ok(Unique(Value::Null))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Unique, A::Error> {
        let mut values = Vec::new();
        while let Some(Unique(value)) = seq.next_element()? {
            values.push(value);
        }
        Ok(Unique(Value::Array(values)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Unique, A::Error> {
        let mut values = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom("duplicate JSON object key"));
            }
            let Unique(value) = map.next_value()?;
            values.insert(key, value);
        }
        Ok(Unique(Value::Object(values)))
    }
}

impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(UniqueJson)
    }
}

pub fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let Unique(value) = serde_json::from_slice(bytes)?;
    Ok(serde_json::from_value(value)?)
}
