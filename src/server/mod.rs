//! Authenticated loopback HTTP surface.
//!
//! Owns route construction, constant-time bearer authentication, host and origin
//! guards, body and event limits, the canonical error envelope, and Server-Sent
//! Events. Provider DTOs never appear here.
