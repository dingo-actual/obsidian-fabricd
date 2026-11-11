//! One session's state, behind one lock.
//!
//! A session binds to exactly one vault for its whole life, and to the one
//! authenticated principal this process serves. Everything mutable sits behind a
//! single mutex: generations, activity, and anything later phases add. The
//! cancellation token is separate because it is shared by value with the work that
//! must observe it.
//!
//! No lock here is ever held across an `await`. Every method is synchronous, and
//! [`crate::session`] denies the lint that would let an async caller break that.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::protocol::{Generation, SessionId, VaultId};

/// One live session.
#[derive(Debug)]
pub struct Session {
    id: SessionId,
    vault_id: VaultId,
    created_at: Instant,
    cancellation: CancellationToken,
    inner: Mutex<Mutable>,
}

#[derive(Debug)]
struct Mutable {
    latest_generation: Generation,
    last_activity: Instant,
}

impl Session {
    /// Creates a session with generation zero and a fresh cancellation token.
    pub(crate) fn new(id: SessionId, vault_id: VaultId, now: Instant) -> Self {
        Self {
            id,
            vault_id,
            created_at: now,
            cancellation: CancellationToken::new(),
            inner: Mutex::new(Mutable {
                latest_generation: Generation::try_new(0).expect("zero is a safe integer"),
                last_activity: now,
            }),
        }
    }

    /// The session identity.
    pub fn id(&self) -> SessionId {
        self.id
    }

    /// The vault this session is bound to. It can never change.
    pub fn vault_id(&self) -> VaultId {
        self.vault_id
    }

    /// When the session was created.
    pub fn created_at(&self) -> Instant {
        self.created_at
    }

    /// The highest accepted generation.
    pub fn latest_generation(&self) -> Generation {
        self.inner().latest_generation
    }

    /// When the session was last used for anything.
    pub fn last_activity(&self) -> Instant {
        self.inner().last_activity
    }

    /// Installs a generation and returns the one it replaced.
    ///
    /// The rules that decide which generation may be installed belong to the
    /// context acceptance path; this is only the write side.
    pub fn install_generation(&self, generation: Generation, now: Instant) -> Generation {
        let mut inner = self.inner();

        let previous = inner.latest_generation;
        inner.latest_generation = generation;
        inner.last_activity = now;

        previous
    }

    /// Marks the session as used, delaying expiry.
    pub(crate) fn touch(&self, now: Instant) {
        self.inner().last_activity = now;
    }

    /// Reports whether the session has been idle for longer than `timeout`.
    pub(crate) fn is_idle_for(&self, now: Instant, timeout: Duration) -> bool {
        now.saturating_duration_since(self.last_activity()) >= timeout
    }

    /// A token that is cancelled when the session expires or is deleted.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Cancels all work for this session.
    pub(crate) fn cancel(&self) {
        self.cancellation.cancel();
    }

    /// Reports whether the session has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Mutable> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::Session;
    use crate::protocol::{Generation, SessionId, VaultId};

    fn session(now: Instant) -> Session {
        Session::new(
            SessionId::from_uuid(uuid::Uuid::new_v4()),
            VaultId::from_uuid(uuid::Uuid::new_v4()),
            now,
        )
    }

    #[test]
    fn a_new_session_starts_at_generation_zero_and_is_not_cancelled() {
        let now = Instant::now();
        let session = session(now);

        assert_eq!(session.latest_generation().get(), 0);
        assert!(!session.is_cancelled());
        assert_eq!(session.last_activity(), now);
    }

    #[test]
    fn installing_a_generation_returns_the_previous_one_and_counts_as_activity() {
        let now = Instant::now();
        let session = session(now);
        let later = now + Duration::from_secs(60);

        let previous = session.install_generation(Generation::try_new(7).expect("in range"), later);

        assert_eq!(previous.get(), 0);
        assert_eq!(session.latest_generation().get(), 7);
        assert_eq!(session.last_activity(), later);
    }

    #[test]
    fn idleness_is_measured_from_the_last_activity() {
        let now = Instant::now();
        let session = session(now);
        let timeout = Duration::from_secs(900);

        session.touch(now + Duration::from_secs(500));

        assert!(!session.is_idle_for(now + Duration::from_secs(899), timeout));
        assert!(!session.is_idle_for(now + Duration::from_secs(1_399), timeout));
        assert!(session.is_idle_for(now + Duration::from_secs(1_400), timeout));
    }

    #[test]
    fn cancelling_is_visible_to_every_clone_of_the_token() {
        let session = session(Instant::now());
        let token = session.cancellation();

        session.cancel();

        assert!(token.is_cancelled());
        assert!(session.is_cancelled());
    }
}
