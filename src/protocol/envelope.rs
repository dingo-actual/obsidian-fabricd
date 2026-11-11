//! Content-bearing requests.
//!
//! These carry note text, so they are the messages where a mis-read field hurts:
//! a duplicated `current_text` or `generation` must be a rejection, never a
//! silent choice. `ContextEnvelope` therefore has no `Deserialize` impl at all —
//! [`decode_envelope`] is the only way in, and it validates structure first.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::protocol::json::JsonValue;
use crate::protocol::newtypes::{Generation, SessionId, UnixMillis, VaultId};
use crate::protocol::strict::{self, ProtocolError};

/// One accepted editing context.
///
/// Every field is untrusted data. Nothing here is a filesystem instruction:
/// `document_path` is a logical vault-relative identifier, never opened.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContextEnvelope {
    /// Wire protocol version.
    pub protocol_version: u32,
    /// Session this context belongs to.
    pub session_id: SessionId,
    /// Vault the session is bound to.
    pub vault_id: VaultId,
    /// Monotonic generation of this context.
    pub generation: Generation,
    /// Vault-relative, slash-separated logical path.
    pub document_path: String,
    /// Displayed filename stem.
    pub document_title: String,
    /// Text at the cursor.
    pub current_text: String,
    /// Meaningful selection, duplicated in `current_text` when present.
    pub selected_text: Option<String>,
    /// Nearest enclosing heading.
    pub section_title: Option<String>,
    /// Active heading path, root to deepest.
    pub heading_path: Vec<String>,
    /// Previous text, oldest first.
    pub preceding_text: Vec<String>,
    /// Following text, nearest first.
    pub following_text: Vec<String>,
    /// Note frontmatter, restricted to the bounded JSON subset.
    pub properties: BTreeMap<String, JsonValue>,
    /// Diagnostic timestamp; never used for ordering.
    pub timestamp_ms: UnixMillis,
    /// What produced this context.
    pub trigger: Trigger,
    /// Whether the client prefers a cached answer.
    pub cache_policy: CachePolicy,
}

impl ContextEnvelope {
    /// Distinct semantic text, counting a duplicated selection once.
    ///
    /// Budgets are measured in Unicode scalar values, so this is an approximation
    /// of the assembled text rather than a byte count.
    pub fn semantic_scalars(&self) -> usize {
        let selection = self.selected_text.as_deref().unwrap_or_default();
        let redundant = self.current_text == selection;

        let scalar = |text: &str| text.chars().count();
        let current = if redundant {
            0
        } else {
            scalar(&self.current_text)
        };
        let properties: usize = self.properties.values().map(JsonValue::text_scalars).sum();

        scalar(&self.document_title)
            + current
            + scalar(selection)
            + self.section_title.as_deref().map_or(0, scalar)
            + self.heading_path.iter().map(|h| scalar(h)).sum::<usize>()
            + self.preceding_text.iter().map(|t| scalar(t)).sum::<usize>()
            + self.following_text.iter().map(|t| scalar(t)).sum::<usize>()
            + properties
    }
}

/// What produced a context submission.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// Submitted after the client's own debounce.
    Automatic,
    /// Requested by the user.
    Manual,
}

/// Whether the client prefers a cached answer for this context.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CachePolicy {
    /// Reuse a matching entry while it is fresh.
    PreferCache,
    /// Skip the cache and run the pipeline again.
    Refresh,
}

/// A cancellation fence. Carries no note text and no path.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CancelRequest {
    /// Wire protocol version.
    pub protocol_version: u32,
    /// Session to advance.
    pub session_id: SessionId,
    /// Vault the session is bound to.
    pub vault_id: VaultId,
    /// New generation; older work is cancelled.
    pub generation: Generation,
    /// Why work was cancelled.
    pub reason: CancelReason,
}

/// Why a client cancelled.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    /// The user moved to a different passage.
    ContextChanged,
    /// The user paused suggestions.
    Paused,
    /// The note or vault is excluded from retrieval.
    Excluded,
    /// The window went inactive.
    Inactive,
    /// The plugin is unloading.
    Unload,
}

/// The wire form of [`ContextEnvelope`], private to this module so the strict
/// decoder stays the only entry point.
#[derive(Deserialize)]
struct EnvelopeWire {
    protocol_version: u32,
    session_id: SessionId,
    vault_id: VaultId,
    generation: Generation,
    document_path: String,
    document_title: String,
    current_text: String,
    selected_text: Option<String>,
    section_title: Option<String>,
    heading_path: Vec<String>,
    preceding_text: Vec<String>,
    following_text: Vec<String>,
    properties: BTreeMap<String, JsonValue>,
    timestamp_ms: UnixMillis,
    trigger: Trigger,
    cache_policy: CachePolicy,
}

impl From<EnvelopeWire> for ContextEnvelope {
    fn from(wire: EnvelopeWire) -> Self {
        Self {
            protocol_version: wire.protocol_version,
            session_id: wire.session_id,
            vault_id: wire.vault_id,
            generation: wire.generation,
            document_path: wire.document_path,
            document_title: wire.document_title,
            current_text: wire.current_text,
            selected_text: wire.selected_text,
            section_title: wire.section_title,
            heading_path: wire.heading_path,
            preceding_text: wire.preceding_text,
            following_text: wire.following_text,
            properties: wire.properties,
            timestamp_ms: wire.timestamp_ms,
            trigger: wire.trigger,
            cache_policy: wire.cache_policy,
        }
    }
}

/// Decodes a context envelope after structural validation.
///
/// # Errors
///
/// Returns a [`ProtocolError`] when the body is malformed, structurally
/// forbidden, or missing a required field.
pub fn decode_envelope(bytes: &[u8]) -> Result<ContextEnvelope, ProtocolError> {
    let text = strict::validate_structure(bytes)?;
    let wire: EnvelopeWire =
        serde_json::from_str(text).map_err(|source| ProtocolError::InvalidJson { source })?;

    Ok(wire.into())
}

#[cfg(test)]
mod tests {
    use super::decode_envelope;

    const FIXTURE: &str = include_str!("../../tests/fixtures/protocol/context-envelope.json");

    #[test]
    fn a_duplicated_selection_is_counted_once() {
        let mut envelope = decode_envelope(FIXTURE.as_bytes()).expect("the fixture decodes");
        envelope.current_text = "abcdef".to_owned();
        envelope.selected_text = Some("abcdef".to_owned());
        envelope.preceding_text.clear();
        envelope.following_text.clear();
        envelope.heading_path.clear();
        envelope.section_title = None;
        envelope.document_title = "T".to_owned();

        assert_eq!(envelope.semantic_scalars(), 1 + 6);
    }

    #[test]
    fn a_selection_inside_other_text_is_counted_twice() {
        let mut envelope = decode_envelope(FIXTURE.as_bytes()).expect("the fixture decodes");
        envelope.current_text = "abcdefgh".to_owned();
        envelope.selected_text = Some("abc".to_owned());
        envelope.preceding_text.clear();
        envelope.following_text.clear();
        envelope.heading_path.clear();
        envelope.section_title = None;
        envelope.properties.clear();
        envelope.document_title.clear();

        assert_eq!(envelope.semantic_scalars(), 8 + 3);
    }

    #[test]
    fn property_text_counts_toward_the_semantic_budget() {
        let text = FIXTURE.replace(
            r#""properties": {}"#,
            r#""properties": {"tags": ["ab", "cde"], "n": 7}"#,
        );
        let envelope = decode_envelope(text.as_bytes()).expect("the modified fixture decodes");

        assert_eq!(envelope.properties.len(), 2);
        assert!(envelope.semantic_scalars() > 5);
    }

    #[test]
    fn an_unknown_inner_property_name_is_kept_as_data_only() {
        let text = FIXTURE.replace(r#""properties": {}"#, r#""properties": {"__proto__": "x"}"#);
        let envelope = decode_envelope(text.as_bytes()).expect("the modified fixture decodes");

        assert_eq!(
            envelope.properties.keys().collect::<Vec<_>>(),
            ["__proto__"]
        );
    }
}
