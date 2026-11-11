//! Wire DTOs shared with the Obsidian plugin.
//!
//! Keys are `snake_case` on the wire and the Rust types mirror the wire names.
//! Serialisation is the only supported reading of these types: a request is
//! decoded through the strict decoder in `protocol::strict`, which rejects invalid
//! UTF-8, duplicate object keys, non-finite numbers, and excessive nesting.
//!
//! The specification names a single `protocol.rs`; this becomes a `protocol/`
//! directory once the DTO set outgrows the module size limits.

/// The only wire protocol version this build speaks.
pub const PROTOCOL_VERSION: u32 = 1;
