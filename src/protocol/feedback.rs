//! Interaction feedback.
//!
//! Feedback never reaches Fabric or an inference provider, and never mutates the
//! vault. It records only that a client handed a link to the browser, or that the
//! user dismissed a card, against a result set the session actually delivered.

use serde::{Deserialize, Serialize};

use crate::protocol::newtypes::{Generation, ResultSetId, SessionId, VaultId};

/// One idempotent interaction with a delivered recommendation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FeedbackRequest {
    /// Wire protocol version.
    pub protocol_version: u32,
    /// Session that received the result.
    pub session_id: SessionId,
    /// Vault the session is bound to.
    pub vault_id: VaultId,
    /// Generation the result was produced for.
    pub generation: Generation,
    /// Delivered result set the resource belonged to.
    pub result_set_id: ResultSetId,
    /// Resource the user interacted with.
    pub resource_id: String,
    /// Client-chosen identity, so a retry has no additional effect.
    pub feedback_id: uuid::Uuid,
    /// What happened.
    pub event: FeedbackEvent,
}

/// What the user did.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackEvent {
    /// The client handed a URL to the browser.
    Opened,
    /// The user dismissed the recommendation.
    Dismissed,
}

#[cfg(test)]
mod tests {
    use super::FeedbackEvent;

    #[test]
    fn both_events_round_trip() {
        for (event, spelling) in [
            (FeedbackEvent::Opened, "opened"),
            (FeedbackEvent::Dismissed, "dismissed"),
        ] {
            let encoded = serde_json::to_string(&event).expect("serialises");
            assert_eq!(encoded, format!("\"{spelling}\""));
            assert_eq!(
                serde_json::from_str::<FeedbackEvent>(&encoded).expect("decodes"),
                event
            );
        }
    }
}
