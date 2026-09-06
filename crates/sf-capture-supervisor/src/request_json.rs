// SPDX-License-Identifier: MIT OR Apache-2.0

use std::{collections::BTreeSet, fmt};

use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};

use crate::{AuthorityError, MAX_REQUEST_BYTES_V2};

pub(crate) fn validate_request_bytes(bytes: &[u8]) -> Result<(), AuthorityError> {
    if bytes.is_empty()
        || bytes.len() > MAX_REQUEST_BYTES_V2
        || bytes.last() != Some(&b'\n')
        || bytes.contains(&b'\r')
        || std::str::from_utf8(bytes).is_err()
    {
        return Err(AuthorityError::InvalidInput("canonical request bytes"));
    }
    let content = &bytes[..bytes.len() - 1];
    let parsed: serde_json::Value = serde_json::from_slice(content)
        .map_err(|_| AuthorityError::InvalidInput("canonical request JSON"))?;
    if !parsed.is_object() {
        return Err(AuthorityError::InvalidInput("canonical request object"));
    }
    let mut deserializer = serde_json::Deserializer::from_slice(content);
    UniqueJsonValue
        .deserialize(&mut deserializer)
        .map_err(|_| AuthorityError::InvalidInput("duplicate request key"))?;
    deserializer
        .end()
        .map_err(|_| AuthorityError::InvalidInput("canonical request JSON"))?;
    Ok(())
}

struct UniqueJsonValue;

impl<'de> DeserializeSeed<'de> for UniqueJsonValue {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_str<E>(self, _value: &str) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_string<E>(self, _value: String) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        UniqueJsonValue.deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element_seed(UniqueJsonValue)?.is_some() {}
        Ok(())
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key) {
                return Err(serde::de::Error::custom("duplicate JSON object key"));
            }
            map.next_value_seed(UniqueJsonValue)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_multiline_json_and_rejects_nested_duplicate_keys() {
        assert!(validate_request_bytes(b"{\n  \"a\": {\"b\": 1}\n}\n").is_ok());
        assert!(validate_request_bytes(b"{\"a\":{\"b\":1,\"b\":2}}\n").is_err());
    }
}
