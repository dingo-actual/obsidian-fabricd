//! A bounded JSON value for note properties.
//!
//! Properties are untrusted frontmatter. This subset has no arbitrary nesting:
//! the strict decoder bounds depth before a value exists, and the typed form keeps
//! numbers in a shape that cannot be non-finite. Objects keep keys sorted so a
//! re-encoded value is deterministic.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A JSON value with no non-finite number and no unbounded depth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonValue {
    /// JSON `null`.
    Null,
    /// JSON `true` or `false`.
    Bool(bool),
    /// A finite JSON number, kept in its original integer or float form.
    Number(serde_json::Number),
    /// A string.
    String(String),
    /// An array of values.
    Array(Vec<JsonValue>),
    /// An object with sorted keys.
    Object(BTreeMap<String, JsonValue>),
}

impl JsonValue {
    /// Reports whether this is JSON `null`.
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Returns the string, when this value is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// Counts the scalar characters this value contributes as text.
    ///
    /// Used when semantic budgets are measured: only strings carry text, and
    /// object keys do not.
    pub fn text_scalars(&self) -> usize {
        match self {
            Self::String(text) => text.chars().count(),
            Self::Array(items) => items.iter().map(Self::text_scalars).sum(),
            Self::Object(entries) => entries.values().map(Self::text_scalars).sum(),
            Self::Null | Self::Bool(_) | Self::Number(_) => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::JsonValue;

    #[test]
    fn a_property_tree_round_trips() {
        let source = r#"{"tags":["a","b"],"rating":5,"nested":{"k":"v"},"flag":true,"empty":null}"#;

        let decoded: JsonValue = serde_json::from_str(source).expect("a property tree decodes");
        let encoded = serde_json::to_string(&decoded).expect("a property tree serialises");

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&encoded).expect("valid JSON"),
            serde_json::from_str::<serde_json::Value>(source).expect("valid JSON")
        );
    }

    #[test]
    fn an_integer_stays_an_integer() {
        let decoded: JsonValue = serde_json::from_str("5").expect("a number decodes");

        assert!(matches!(decoded, JsonValue::Number(ref number) if number.is_u64()));
        assert_eq!(serde_json::to_string(&decoded).expect("serialises"), "5");
    }

    #[test]
    fn text_scalars_count_strings_only() {
        let value: JsonValue =
            serde_json::from_str(r#"{"a":"héllo","b":[1,2,"ab"],"c":{"d":null}}"#)
                .expect("a property tree decodes");

        assert_eq!(value.text_scalars(), 7);
    }

    #[test]
    fn null_is_distinguished_from_an_empty_string() {
        assert!(JsonValue::Null.is_null());
        assert!(!JsonValue::String(String::new()).is_null());
    }
}
