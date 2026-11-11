//! Wire DTOs shared with the Obsidian plugin.
//!
//! Keys are `snake_case` on the wire and the Rust types use the wire names, so
//! there is no translation layer to drift.
//!
//! Every message enters through [`strict`]: valid UTF-8, one complete JSON value,
//! no duplicate object keys at any depth, at most four nesting levels, at most 32
//! keys per object, and no number a JavaScript client would misread. A message
//! with a `Deserialize` impl can use [`decode_strict`]; a content-bearing message
//! that must never be decoded casually has its own decoder and no `Deserialize`
//! impl at all.
//!
//! Unknown additive fields are ignored for interpretation and are not retained:
//! a typed value has nowhere to hold one, so nothing downstream can forward it.
//!
//! This is a directory rather than one file because the message set outgrew the
//! module size limits.

pub mod envelope;
pub mod errors;
pub mod events;
pub mod feedback;
pub mod json;
pub mod newtypes;
pub mod session;
pub mod strict;

/// The only wire protocol version this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;

pub use envelope::{
    CachePolicy, CancelReason, CancelRequest, ContextEnvelope, Trigger, decode_envelope,
};
pub use errors::{ErrorCode, ErrorEnvelope};
pub use events::{
    CacheInfo, EmptyReason, QueryMode, Recommendation, RelevancePolicy, RerankMode, ResultsEvent,
    ScoreKind, Stage, StatusEvent,
};
pub use feedback::{FeedbackEvent, FeedbackRequest};
pub use json::JsonValue;
pub use newtypes::{
    Generation, MAX_SAFE_INTEGER, ResultSetId, ScalarCount, ServerInstanceId, SessionId,
    UnixMillis, ValueOutOfRange, VaultId,
};
pub use session::{
    ContextAcceptance, ContextualizerHealth, ContextualizerMode, FabricHealth, HealthResponse,
    Limits, ProviderObservation, RerankerHealth, ServerStatus, SessionCreateRequest,
    SessionCreateResponse,
};
pub use strict::{
    MAX_DEPTH, MAX_KEYS_PER_OBJECT, ProtocolError, decode_strict, validate_structure,
};

#[cfg(test)]
mod tests {
    use super::{PROTOCOL_VERSION, decode_strict};

    #[test]
    fn the_protocol_version_is_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[test]
    fn every_decoder_shares_the_structural_rules() {
        let duplicate = br#"{"protocol_version":1,"generation":3,"generation":4}"#;
        let too_deep = br#"{"a":{"b":{"c":{"d":{"e":1}}}}}"#;

        assert!(decode_strict::<super::ContextAcceptance>(duplicate).is_err());
        assert!(decode_strict::<super::JsonValue>(too_deep).is_err());
    }
}
