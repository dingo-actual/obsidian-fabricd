//! Idle expiry.
//!
//! A session that nobody touches for the configured timeout stops existing, and its
//! outstanding work is cancelled rather than left running for a window that is
//! gone. Expiry is driven by an explicit clock so the timing rules can be tested
//! without waiting: [`crate::session::Registry::expire_due`] takes the current
//! instant, and the sweeper below supplies it in a running process.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;

use super::registry::Registry;

/// Shortest sweep interval.
const MIN_SWEEP: Duration = Duration::from_secs(1);
/// Longest sweep interval, so a session never lingers much past its timeout.
const MAX_SWEEP: Duration = Duration::from_secs(30);

/// How often to sweep, derived from the idle timeout.
///
/// One tenth of the timeout keeps the worst-case overrun at ten percent, and the
/// bounds stop a tiny timeout from spinning or a huge one from lagging.
pub fn sweep_interval(idle_timeout: Duration) -> Duration {
    (idle_timeout / 10).clamp(MIN_SWEEP, MAX_SWEEP)
}

/// Starts a task that collects expired sessions until the runtime stops.
///
/// The task holds the registry, so it keeps expiring sessions for the lifetime of
/// the process and stops when the runtime is dropped.
pub fn spawn_sweeper(registry: Arc<Registry>) -> JoinHandle<()> {
    let interval = sweep_interval(registry.limits().idle_timeout);

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick completes immediately; expire nothing before then.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            let expired = registry.expire_due(std::time::Instant::now());
            if !expired.is_empty() {
                tracing::info!(count = expired.len(), "idle sessions expired");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{MAX_SWEEP, MIN_SWEEP, sweep_interval};

    #[test]
    fn a_short_timeout_sweeps_at_the_minimum_interval() {
        assert_eq!(sweep_interval(Duration::from_secs(5)), MIN_SWEEP);
    }

    #[test]
    fn the_default_timeout_sweeps_every_ninety_seconds_worth_of_work_in_slices() {
        assert_eq!(sweep_interval(Duration::from_secs(900)), MAX_SWEEP);
    }

    #[test]
    fn a_middle_timeout_sweeps_at_a_tenth_of_itself() {
        assert_eq!(
            sweep_interval(Duration::from_secs(120)),
            Duration::from_secs(12)
        );
    }

    #[tokio::test]
    async fn the_sweeper_collects_a_session_whose_timeout_has_passed() {
        use std::sync::Arc;
        use std::time::Instant;

        use super::spawn_sweeper;
        use crate::protocol::{SessionId, VaultId};
        use crate::session::registry::{Limits, Registry};

        let registry = Arc::new(Registry::new(Limits {
            max_sessions: 4,
            idle_timeout: Duration::from_secs(1),
        }));
        let session = match registry
            .admit(
                SessionId::from_uuid(uuid::Uuid::new_v4()),
                VaultId::from_uuid(uuid::Uuid::new_v4()),
                Instant::now() - Duration::from_secs(2),
            )
            .expect("admit")
        {
            crate::session::registry::Admit::Created(session) => session,
            crate::session::registry::Admit::Existing(_) => panic!("created"),
        };

        let sweeper = spawn_sweeper(registry.clone());
        tokio::time::sleep(Duration::from_millis(1_200)).await;
        sweeper.abort();

        assert!(session.is_cancelled());
        assert!(registry.is_empty());
    }
}
