//! Byte limits shared by the request and event paths.

/// Largest accepted request body, matching the shipped default configuration.
pub const DEFAULT_REQUEST_BYTES: u64 = 128 * 1024;
/// Largest emitted event, matching the shipped default configuration.
pub const DEFAULT_EVENT_BYTES: u64 = 256 * 1024;

/// The limit a request was checked against, for an error message.
pub fn describe_bytes(limit: u64) -> String {
    format!("request body exceeds {limit} bytes")
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_EVENT_BYTES, DEFAULT_REQUEST_BYTES, describe_bytes};

    #[test]
    fn the_defaults_are_the_documented_byte_counts() {
        assert_eq!(DEFAULT_REQUEST_BYTES, 131_072);
        assert_eq!(DEFAULT_EVENT_BYTES, 262_144);
        assert!(describe_bytes(DEFAULT_REQUEST_BYTES).contains("131072"));
    }
}
