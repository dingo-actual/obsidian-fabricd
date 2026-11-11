//! The bounded session registry.
//!
//! The registry never evicts a live session to make room. At the cap it refuses,
//! so a client cannot lose a working window because another window opened. Expired
//! sessions are collected first, which means the cap only ever counts sessions that
//! are still alive.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::protocol::{SessionId, VaultId};

use super::state::Session;

/// Registry limits, taken from configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    /// Largest number of live sessions.
    pub max_sessions: usize,
    /// Idle time after which a session is collected.
    pub idle_timeout: Duration,
}

/// What happened when a session identity was submitted.
#[derive(Debug)]
pub enum Admit {
    /// The session was created.
    Created(Arc<Session>),
    /// The same identity and vault already exist; nothing was replaced.
    Existing(Arc<Session>),
}

/// Why a session identity was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmitError {
    /// The registry is full of live sessions.
    Capacity,
    /// The identity is already bound to a different vault.
    VaultConflict {
        /// The vault the identity is bound to.
        bound: VaultId,
    },
}

/// The live sessions of one process lifetime.
#[derive(Debug)]
pub struct Registry {
    limits: Limits,
    sessions: Mutex<HashMap<SessionId, Arc<Session>>>,
}

impl Registry {
    /// Creates an empty registry.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// The limits this registry enforces.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Number of live sessions.
    pub fn len(&self) -> usize {
        self.sessions().len()
    }

    /// Reports whether no session is live.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Submits a session identity.
    ///
    /// # Errors
    ///
    /// Returns [`AdmitError::VaultConflict`] when the identity is already bound to
    /// another vault, and [`AdmitError::Capacity`] when the registry is full of
    /// live sessions.
    pub fn admit(
        &self,
        id: SessionId,
        vault_id: VaultId,
        now: Instant,
    ) -> Result<Admit, AdmitError> {
        let mut sessions = self.sessions();

        collect_idle(&mut sessions, now, self.limits.idle_timeout);

        if let Some(existing) = sessions.get(&id) {
            if existing.vault_id() != vault_id {
                return Err(AdmitError::VaultConflict {
                    bound: existing.vault_id(),
                });
            }
            existing.touch(now);

            return Ok(Admit::Existing(existing.clone()));
        }

        if sessions.len() >= self.limits.max_sessions {
            return Err(AdmitError::Capacity);
        }

        let session = Arc::new(Session::new(id, vault_id, now));
        sessions.insert(id, session.clone());

        Ok(Admit::Created(session))
    }

    /// Finds a session and renews its activity.
    ///
    /// An expired session is collected first, so a caller never observes a session
    /// that has already timed out.
    pub fn find(&self, id: SessionId, now: Instant) -> Option<Arc<Session>> {
        let mut sessions = self.sessions();
        let timeout = self.limits.idle_timeout;

        if let Some(session) = sessions.get(&id)
            && session.is_idle_for(now, timeout)
        {
            let expired = sessions.remove(&id).expect("the session was just observed");
            expired.cancel();

            return None;
        }

        let session = sessions.get(&id)?;
        session.touch(now);

        Some(session.clone())
    }

    /// Removes a session, cancelling its work.
    ///
    /// Returns `None` when the identity is unknown, which is the normal case after
    /// a restart or a previous delete.
    pub fn remove(&self, id: SessionId) -> Option<Arc<Session>> {
        let removed = self.sessions().remove(&id);

        if let Some(session) = &removed {
            session.cancel();
        }

        removed
    }

    /// Collects every session that has been idle for the configured timeout.
    ///
    /// Each collected session is cancelled before this returns, so outstanding work
    /// stops instead of continuing for a window that no longer exists.
    pub fn expire_due(&self, now: Instant) -> Vec<SessionId> {
        let mut sessions = self.sessions();
        collect_idle(&mut sessions, now, self.limits.idle_timeout)
    }

    fn sessions(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, Arc<Session>>> {
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

/// Removes idle sessions and cancels them.
fn collect_idle(
    sessions: &mut HashMap<SessionId, Arc<Session>>,
    now: Instant,
    idle_timeout: Duration,
) -> Vec<SessionId> {
    let expired: Vec<SessionId> = sessions
        .iter()
        .filter(|(_, session)| session.is_idle_for(now, idle_timeout))
        .map(|(id, _)| *id)
        .collect();

    for id in &expired {
        if let Some(session) = sessions.remove(id) {
            session.cancel();
        }
    }

    expired
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Admit, AdmitError, Limits, Registry};
    use crate::protocol::{Generation, SessionId, VaultId};

    const IDLE: Duration = Duration::from_secs(900);

    fn registry(max_sessions: usize) -> Registry {
        Registry::new(Limits {
            max_sessions,
            idle_timeout: IDLE,
        })
    }

    fn id() -> SessionId {
        SessionId::from_uuid(uuid::Uuid::new_v4())
    }

    fn vault() -> VaultId {
        VaultId::from_uuid(uuid::Uuid::new_v4())
    }

    #[test]
    fn a_new_identity_is_created_and_a_repeat_is_existing() {
        let registry = registry(4);
        let now = Instant::now();
        let (session_id, vault) = (id(), vault());

        let Admit::Created(first) = registry.admit(session_id, vault, now).expect("admit") else {
            panic!("the first submission creates a session");
        };
        let Admit::Existing(second) = registry.admit(session_id, vault, now).expect("admit") else {
            panic!("the second submission finds the session");
        };

        assert_eq!(first.id(), second.id());
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn a_repeat_reports_the_generation_the_session_already_holds() {
        let registry = registry(4);
        let now = Instant::now();
        let (session_id, vault) = (id(), vault());
        let Admit::Created(session) = registry.admit(session_id, vault, now).expect("admit") else {
            panic!("created");
        };
        session.install_generation(Generation::try_new(12).expect("in range"), now);

        let Admit::Existing(again) = registry.admit(session_id, vault, now).expect("admit") else {
            panic!("existing");
        };

        assert_eq!(again.latest_generation().get(), 12);
    }

    #[test]
    fn rebinding_the_same_identity_to_another_vault_is_refused() {
        let registry = registry(4);
        let now = Instant::now();
        let (session_id, first_vault, second_vault) = (id(), vault(), vault());
        registry
            .admit(session_id, first_vault, now)
            .expect("the first binding is accepted");

        let error = registry
            .admit(session_id, second_vault, now)
            .expect_err("a rebind must be refused");

        assert_eq!(error, AdmitError::VaultConflict { bound: first_vault });
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn the_cap_refuses_rather_than_evicting() {
        let registry = registry(16);
        let now = Instant::now();
        let mut admitted = Vec::new();
        for _ in 0..16 {
            let Admit::Created(session) = registry.admit(id(), vault(), now).expect("admit") else {
                panic!("created");
            };
            admitted.push(session);
        }

        let error = registry
            .admit(id(), vault(), now)
            .expect_err("the seventeenth session must be refused");

        assert_eq!(error, AdmitError::Capacity);
        assert_eq!(registry.len(), 16);
        assert!(admitted.iter().all(|session| !session.is_cancelled()));
        for session in &admitted {
            assert!(registry.find(session.id(), now).is_some());
        }
    }

    #[test]
    fn a_session_survives_just_under_the_timeout_and_is_collected_at_it() {
        let registry = registry(4);
        let now = Instant::now();
        let Admit::Created(session) = registry.admit(id(), vault(), now).expect("admit") else {
            panic!("created");
        };

        assert!(
            registry
                .expire_due(now + Duration::from_secs(899))
                .is_empty(),
            "a session is live just under the timeout"
        );

        let expired = registry.expire_due(now + IDLE);

        assert_eq!(expired, vec![session.id()]);
        assert!(session.is_cancelled());
        assert!(registry.find(session.id(), now + IDLE).is_none());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn a_use_after_creation_resets_the_timer() {
        let registry = registry(4);
        let now = Instant::now();
        let Admit::Created(session) = registry.admit(id(), vault(), now).expect("admit") else {
            panic!("created");
        };
        let used_at = now + Duration::from_secs(500);

        assert!(registry.find(session.id(), used_at).is_some());
        assert!(
            registry.expire_due(now + IDLE).is_empty(),
            "the use delayed expiry past the original deadline"
        );
        assert!(
            registry
                .expire_due(now + Duration::from_secs(1_399))
                .is_empty()
        );
        assert_eq!(
            registry.expire_due(now + Duration::from_secs(1_400)).len(),
            1
        );
    }

    #[test]
    fn an_expired_session_frees_capacity_for_a_new_one() {
        let registry = registry(1);
        let now = Instant::now();
        registry
            .admit(id(), vault(), now)
            .expect("the first session fits");

        let later = now + IDLE;

        let Admit::Created(_) = registry
            .admit(id(), vault(), later)
            .expect("the expired session must not block a new one")
        else {
            panic!("created");
        };
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn removing_one_session_leaves_its_neighbour_untouched() {
        let registry = registry(4);
        let now = Instant::now();
        let vault = vault();
        let Admit::Created(first) = registry.admit(id(), vault, now).expect("admit") else {
            panic!("created");
        };
        let Admit::Created(second) = registry.admit(id(), vault, now).expect("admit") else {
            panic!("created");
        };

        let removed = registry.remove(first.id()).expect("the session is removed");

        assert_eq!(removed.id(), first.id());
        assert!(first.is_cancelled());
        assert!(!second.is_cancelled());
        assert!(registry.find(second.id(), now).is_some());
        assert!(registry.find(first.id(), now).is_none());
    }

    #[test]
    fn removing_an_unknown_session_reports_nothing() {
        let registry = registry(4);

        assert!(registry.remove(id()).is_none());
        assert!(registry.is_empty());
    }

    #[test]
    fn finding_an_unknown_session_reports_nothing() {
        let registry = registry(4);

        assert!(registry.find(id(), Instant::now()).is_none());
    }

    #[test]
    fn the_limits_are_reported_unchanged() {
        let registry = registry(3);

        assert_eq!(registry.limits().max_sessions, 3);
        assert_eq!(registry.limits().idle_timeout, IDLE);
    }
}
