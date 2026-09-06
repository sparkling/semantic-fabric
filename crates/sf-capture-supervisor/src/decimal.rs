// SPDX-License-Identifier: MIT OR Apache-2.0

fn parse<E: serde::de::Error>(value: &str) -> Result<u64, E> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || value.bytes().any(|byte| !byte.is_ascii_digit())
    {
        return Err(E::custom("noncanonical uint64 decimal string"));
    }
    value
        .parse()
        .map_err(|_| E::custom("uint64 decimal string exceeds range"))
}

pub(crate) mod u64_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u64, D::Error>
    where
        D: Deserializer<'de>,
    {
        super::parse(&String::deserialize(deserializer)?)
    }
}

pub(crate) mod option_u64_string {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(value) => serializer.serialize_some(&value.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer)?
            .map(|value| super::parse(&value))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Value {
        #[serde(with = "super::u64_string")]
        count: u64,
    }

    #[test]
    fn only_canonical_decimal_strings_are_accepted() {
        assert_eq!(
            serde_json::to_string(&Value { count: 42 }).unwrap(),
            "{\"count\":\"42\"}"
        );
        assert_eq!(
            serde_json::from_str::<Value>("{\"count\":\"42\"}").unwrap(),
            Value { count: 42 }
        );
        for invalid in ["{\"count\":42}", "{\"count\":\"01\"}", "{\"count\":\"-1\"}"] {
            assert!(serde_json::from_str::<Value>(invalid).is_err());
        }
    }
}
