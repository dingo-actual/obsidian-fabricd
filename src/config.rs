//! Configuration contract.
//!
//! Holds the versioned TOML schema, secret-name resolution, platform path
//! resolution, cross-section validation, and the pipeline fingerprint that
//! partitions the cache. Unknown keys are rejected: a misspelled privacy or
//! fallback setting must fail validation rather than take effect silently.
