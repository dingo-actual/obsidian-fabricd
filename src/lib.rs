//! `obsidian-fabricd`: a loopback companion service that turns a bounded
//! Obsidian editing context into ordered Fabric resource recommendations.
//!
//! The service owns provider credentials and all outbound network access. It is
//! not a vault indexer, a synchronisation engine, or an answer generator, and it
//! never needs to read the vault directory: document paths are logical routing
//! metadata and must never be given to a filesystem API.
//!
//! Version 1 has no Fabric write path and no vault mutation.
//!
//! Module layout follows the specification:
//!
//! - [`protocol`]: wire DTOs shared with the Obsidian plugin.
//! - [`config`]: the configuration contract.
//! - [`telemetry`]: process-wide logging setup.
//! - [`server`]: the authenticated loopback HTTP surface.
//! - [`session`]: per-session state, generation fencing, and publication.
//! - [`context`]: validation, normalisation, redaction, and egress projection.
//! - [`pipeline`]: scheduling, orchestration, and result selection.
//! - [`fabric`]: the read-only Fabric search adapter.
//! - [`contextualizer`]: the optional retrieval-prefix adapter.
//! - [`reranker`]: provider-neutral reranking adapters.
//! - [`cache`]: the SQLite cache worker and migrations.

#![forbid(unsafe_code)]

pub mod cache;
pub mod cli;
pub mod config;
pub mod context;
pub mod contextualizer;
pub mod fabric;
pub mod pipeline;
pub mod protocol;
pub mod reranker;
pub mod server;
pub mod session;
pub mod telemetry;
