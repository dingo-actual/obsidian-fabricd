//! Bounded context handling.
//!
//! Owns field limits measured in Unicode scalar values, logical path validation,
//! normalisation, secret redaction, and the `RedactedContext` egress projection.
//! Every outbound adapter takes a redacted context; a raw context envelope has no
//! provider serialisation.
