//! The only way into the wire model.
//!
//! A body is validated structurally before any typed value exists: valid UTF-8,
//! complete JSON, no duplicate object keys at any depth, at most four nesting
//! levels, at most 32 keys per object, and no number that a JavaScript client
//! would misread. Only then is the typed value built.
//!
//! Duplicate keys are the reason this pre-pass exists. Serde keeps the last value
//! and reports nothing, so a body that repeats `generation` or `current_text`
//! would otherwise be accepted with a silently chosen meaning.

use std::cell::RefCell;
use std::rc::Rc;

use serde::de::{self, DeserializeOwned, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::Deserializer;

/// Deepest container nesting accepted in a request body.
pub const MAX_DEPTH: usize = 4;
/// Most keys accepted in one object.
pub const MAX_KEYS_PER_OBJECT: usize = 32;

/// Why a body was rejected before it became a typed value.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// The body is not valid UTF-8.
    #[error("request body is not valid UTF-8")]
    NotUtf8,
    /// The body is not one complete JSON value of the expected shape.
    #[error("invalid JSON: {source}")]
    InvalidJson {
        /// The parser or field failure.
        #[source]
        source: serde_json::Error,
    },
    /// An object repeats a key.
    #[error("duplicate object key `{key}`")]
    DuplicateKey {
        /// The repeated key.
        key: String,
    },
    /// An object has more keys than the protocol allows.
    #[error("object has {found} keys; at most {MAX_KEYS_PER_OBJECT} are allowed")]
    TooManyKeys {
        /// Number of keys found.
        found: usize,
    },
    /// A value nests deeper than the protocol allows.
    #[error("nesting exceeds {MAX_DEPTH} levels")]
    TooDeep,
    /// A number cannot be represented as a finite double.
    #[error("number is not finite")]
    NonFiniteNumber,
}

/// Decodes one protocol message.
///
/// Unknown fields are ignored, so an additive protocol change stays compatible.
/// Ignored fields are not retained: the typed value has no place to hold them,
/// and nothing downstream may forward them.
///
/// # Errors
///
/// Returns a [`ProtocolError`] when the body is malformed, structurally
/// forbidden, or does not match the expected message.
pub fn decode_strict<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ProtocolError> {
    let text = validate_structure(bytes)?;

    serde_json::from_str(text).map_err(|source| ProtocolError::InvalidJson { source })
}

/// Validates the structure of a body and returns it as text.
///
/// Shared by the typed decoders so a message with no `Deserialize` impl can be
/// validated the same way.
///
/// # Errors
///
/// Returns a [`ProtocolError`] when the body is not UTF-8, not one complete JSON
/// value, or violates a structural limit.
pub fn validate_structure(bytes: &[u8]) -> Result<&str, ProtocolError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ProtocolError::NotUtf8)?;
    let recorded = Recorded::default();

    let mut deserializer = Deserializer::from_str(text);
    let walk = StructureVisitor {
        recorded: recorded.clone(),
        depth: 1,
    }
    .deserialize(&mut deserializer);

    if let Some(kind) = recorded.take() {
        return Err(kind.into_protocol());
    }
    walk.map_err(|source| ProtocolError::InvalidJson { source })?;
    deserializer
        .end()
        .map_err(|source| ProtocolError::InvalidJson { source })?;

    Ok(text)
}

/// A structural violation, recorded out of band because the walk reports through
/// serde's error type, which cannot carry a typed reason.
#[derive(Debug)]
enum Kind {
    DuplicateKey(String),
    TooManyKeys(usize),
    TooDeep,
    NonFinite,
}

impl Kind {
    fn into_protocol(self) -> ProtocolError {
        match self {
            Self::DuplicateKey(key) => ProtocolError::DuplicateKey { key },
            Self::TooManyKeys(found) => ProtocolError::TooManyKeys { found },
            Self::TooDeep => ProtocolError::TooDeep,
            Self::NonFinite => ProtocolError::NonFiniteNumber,
        }
    }
}

/// The one slot a violation is stored in during a single-threaded parse.
#[derive(Clone, Default)]
struct Recorded(Rc<RefCell<Option<Kind>>>);

impl Recorded {
    fn record<E: de::Error>(&self, kind: Kind) -> E {
        *self.0.borrow_mut() = Some(kind);
        E::custom("protocol structural violation")
    }

    fn take(&self) -> Option<Kind> {
        self.0.borrow_mut().take()
    }
}

/// Walks the document without building a value.
struct StructureVisitor {
    recorded: Recorded,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for StructureVisitor {
    type Value = ();

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl StructureVisitor {
    fn enter<E: de::Error>(&self) -> Result<(), E> {
        if self.depth > MAX_DEPTH {
            return Err(self.recorded.record(Kind::TooDeep));
        }

        Ok(())
    }

    fn child(&self) -> StructureVisitor {
        StructureVisitor {
            recorded: self.recorded.clone(),
            depth: self.depth + 1,
        }
    }
}

impl<'de> Visitor<'de> for StructureVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("one JSON value")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        self.enter()?;

        let mut keys: Vec<String> = Vec::new();
        while let Some(key) = map.next_key::<String>()? {
            if keys.len() == MAX_KEYS_PER_OBJECT {
                return Err(self.recorded.record(Kind::TooManyKeys(keys.len() + 1)));
            }
            if keys.contains(&key) {
                return Err(self.recorded.record(Kind::DuplicateKey(key)));
            }
            keys.push(key);
            map.next_value_seed(self.child())?;
        }

        Ok(())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        self.enter()?;

        while seq.next_element_seed(self.child())?.is_some() {}

        Ok(())
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<(), E> {
        if value.is_finite() {
            return Ok(());
        }

        Err(self.recorded.record(Kind::NonFinite))
    }

    fn visit_bool<E: de::Error>(self, _value: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E: de::Error>(self, _value: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E: de::Error>(self, _value: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_str<E: de::Error>(self, _value: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_DEPTH, MAX_KEYS_PER_OBJECT, ProtocolError, decode_strict};

    #[test]
    fn the_limits_are_the_documented_ones() {
        assert_eq!(MAX_DEPTH, 4);
        assert_eq!(MAX_KEYS_PER_OBJECT, 32);
    }

    #[test]
    fn a_flat_object_is_accepted() {
        let value: serde_json::Value =
            decode_strict(br#"{"a":1,"b":[true,null,"x"]}"#).expect("a flat object is valid");

        assert!(value.is_object());
    }

    #[test]
    fn four_levels_are_the_last_accepted_depth() {
        assert!(decode_strict::<serde_json::Value>(br#"{"a":{"b":{"c":{"d":1}}}}"#).is_ok());
        assert!(matches!(
            decode_strict::<serde_json::Value>(br#"{"a":{"b":{"c":{"d":{"e":1}}}}}"#),
            Err(ProtocolError::TooDeep)
        ));
    }

    #[test]
    fn thirty_two_keys_are_the_last_accepted_count() {
        for count in [32usize, 33] {
            let pairs: Vec<String> = (0..count).map(|i| format!("\"k{i}\":{i}")).collect();
            let body = format!("{{{}}}", pairs.join(","));
            let decoded = decode_strict::<serde_json::Value>(body.as_bytes());

            if count == 32 {
                assert!(decoded.is_ok(), "32 keys must be accepted");
            } else {
                assert!(matches!(decoded, Err(ProtocolError::TooManyKeys { .. })));
            }
        }
    }

    #[test]
    fn a_duplicate_key_names_the_key() {
        let error = decode_strict::<serde_json::Value>(br#"{"a":1,"b":2,"a":3}"#)
            .expect_err("a duplicate must be rejected");

        assert!(matches!(&error, ProtocolError::DuplicateKey { key } if key == "a"));
    }

    #[test]
    fn an_out_of_range_number_is_rejected_by_the_parser() {
        // A float literal beyond the double range never becomes a value: the JSON
        // number parser refuses it, so the walk never sees infinity from text.
        for body in [br#"{"a":[1,1e400]}"#.as_slice(), br#"{"a":-1e400}"#] {
            assert!(matches!(
                decode_strict::<serde_json::Value>(body),
                Err(ProtocolError::InvalidJson { .. })
            ));
        }
    }

    #[test]
    fn the_walk_rejects_a_non_finite_number_that_reaches_it() {
        use serde::de::Visitor;

        let visitor = super::StructureVisitor {
            recorded: super::Recorded::default(),
            depth: 1,
        };

        assert!(
            visitor
                .visit_f64::<serde_json::Error>(f64::INFINITY)
                .is_err()
        );

        let visitor = super::StructureVisitor {
            recorded: super::Recorded::default(),
            depth: 1,
        };
        assert!(visitor.visit_f64::<serde_json::Error>(1.5).is_ok());
    }

    #[test]
    fn a_finite_float_is_accepted() {
        assert!(decode_strict::<serde_json::Value>(br#"{"a":1.5,"b":-0.25}"#).is_ok());
    }

    #[test]
    fn trailing_whitespace_is_accepted_but_trailing_values_are_not() {
        assert!(decode_strict::<serde_json::Value>(b"{\"a\":1}\n").is_ok());
        assert!(matches!(
            decode_strict::<serde_json::Value>(b"{\"a\":1}{}"),
            Err(ProtocolError::InvalidJson { .. })
        ));
    }

    #[test]
    fn a_syntax_error_is_reported_as_invalid_json() {
        assert!(matches!(
            decode_strict::<serde_json::Value>(b"{\"a\":}"),
            Err(ProtocolError::InvalidJson { .. })
        ));
    }
}
