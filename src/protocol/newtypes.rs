//! Wire newtypes.
//!
//! Each identifier and integer in the protocol has one distinct meaning, so it
//! gets its own type. A bare `u64` or `String` would let a generation be used as
//! a timestamp and a session be used as a vault.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// Largest integer that survives a JavaScript round trip.
///
/// Every integer the plugin sees must stay exact in a double-precision float, so
/// the protocol forbids anything above this bound.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// A value that cannot be represented exactly as a JavaScript number.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValueOutOfRange {
    /// The rejected value.
    pub value: u64,
    /// The largest accepted value.
    pub maximum: u64,
}

impl fmt::Display for ValueOutOfRange {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} exceeds the safe integer maximum {}",
            self.value, self.maximum
        )
    }
}

impl std::error::Error for ValueOutOfRange {}

macro_rules! safe_integer {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(u64);

        impl $name {
            /// Creates the value if it is a safe integer.
            ///
            /// # Errors
            ///
            /// Returns [`ValueOutOfRange`] when the value would lose precision in
            /// a JavaScript number.
            pub fn try_new(value: u64) -> Result<Self, ValueOutOfRange> {
                if value <= MAX_SAFE_INTEGER {
                    Ok(Self(value))
                } else {
                    Err(ValueOutOfRange {
                        value,
                        maximum: MAX_SAFE_INTEGER,
                    })
                }
            }

            /// Returns the inner number.
            pub const fn get(self) -> u64 {
                self.0
            }

            /// Returns the next value, or `None` at the safe integer maximum.
            pub fn checked_next(self) -> Option<Self> {
                self.0
                    .checked_add(1)
                    .and_then(|next| Self::try_new(next).ok())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_u64(self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = u64::deserialize(deserializer)?;
                Self::try_new(raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

safe_integer!(
    Generation,
    "One monotonic step of a session's context sequence. Zero means no context has been accepted yet, which is only valid in a response that reports session state."
);
safe_integer!(
    UnixMillis,
    "A diagnostic timestamp: nonnegative Unix milliseconds. Never determines ordering or expiry; generations do that."
);

/// A length measured in Unicode scalar values rather than bytes or UTF-16 units.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScalarCount(u32);

impl ScalarCount {
    /// A zero count.
    pub const ZERO: Self = Self(0);

    /// Creates a count.
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the inner count.
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Applies a limit, returning whichever value is smaller.
    pub const fn clamp_to(self, limit: Self) -> Self {
        if self.0 <= limit.0 { self } else { limit }
    }
}

impl Serialize for ScalarCount {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.0)
    }
}

impl<'de> Deserialize<'de> for ScalarCount {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u32::deserialize(deserializer).map(Self)
    }
}

macro_rules! uuid_newtype {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub struct $name(Uuid);

        impl $name {
            /// Wraps an already validated UUID.
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Borrows the inner UUID.
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// Consumes the newtype and returns the UUID.
            pub const fn into_uuid(self) -> Uuid {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                Uuid::from_str(text).map(Self)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                self.0.serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                Uuid::deserialize(deserializer).map(Self)
            }
        }
    };
}

uuid_newtype!(
    SessionId,
    "A plugin runtime or window. Fresh for each runtime; scopes generations, events, cancellation, and reconnect state."
);
uuid_newtype!(
    VaultId,
    "A plugin vault configuration. Partitions cache and feedback without transmitting an absolute directory."
);
uuid_newtype!(
    ResultSetId,
    "One delivered outcome. A cache hit receives a fresh identity."
);
uuid_newtype!(
    ServerInstanceId,
    "One connector process lifetime. Fresh on every start, so a client can detect that volatile sessions are gone."
);

#[cfg(test)]
mod tests {
    use super::{
        Generation, MAX_SAFE_INTEGER, ScalarCount, SessionId, UnixMillis, ValueOutOfRange,
    };

    #[test]
    fn the_safe_integer_maximum_is_the_documented_bound() {
        assert_eq!(MAX_SAFE_INTEGER, 2_u64.pow(53) - 1);
        assert!(Generation::try_new(MAX_SAFE_INTEGER).is_ok());
        assert!(Generation::try_new(MAX_SAFE_INTEGER + 1).is_err());
    }

    #[test]
    fn the_range_error_names_the_value_and_the_bound() {
        let error = UnixMillis::try_new(MAX_SAFE_INTEGER + 8).expect_err("out of range");

        assert_eq!(
            error,
            ValueOutOfRange {
                value: MAX_SAFE_INTEGER + 8,
                maximum: MAX_SAFE_INTEGER
            }
        );
        assert!(
            error
                .to_string()
                .contains(&(MAX_SAFE_INTEGER + 8).to_string())
        );
    }

    #[test]
    fn a_generation_advances_without_leaving_the_safe_range() {
        let generation = Generation::try_new(11).expect("in range");

        assert_eq!(generation.checked_next().map(Generation::get), Some(12));
        assert_eq!(
            Generation::try_new(MAX_SAFE_INTEGER)
                .expect("in range")
                .checked_next(),
            None
        );
    }

    #[test]
    fn a_scalar_count_clamps_to_a_smaller_limit() {
        assert_eq!(
            ScalarCount::new(9).clamp_to(ScalarCount::new(4)),
            ScalarCount::new(4)
        );
        assert_eq!(
            ScalarCount::new(3).clamp_to(ScalarCount::new(4)),
            ScalarCount::new(3)
        );
    }

    #[test]
    fn identifiers_serialise_as_hyphenated_lowercase() {
        let id: SessionId = "5B6F4992-9158-4B22-AA77-601F771B6907"
            .parse()
            .expect("a UUID");

        assert_eq!(id.to_string(), "5b6f4992-9158-4b22-aa77-601f771b6907");
        assert_eq!(
            serde_json::to_string(&id).expect("serialises"),
            "\"5b6f4992-9158-4b22-aa77-601f771b6907\""
        );
    }

    #[test]
    fn an_identifier_rejects_a_non_uuid() {
        assert!("not-a-uuid".parse::<SessionId>().is_err());
        assert!(serde_json::from_str::<SessionId>("\"not-a-uuid\"").is_err());
    }
}
