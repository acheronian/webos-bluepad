//! Bounded per-trigger backoff (Strategy as a closed enum).

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Backoff cadence for one recovery trigger: exponential from `min` to `max`,
/// for at most `max_attempts` attempts — then the pad parks in `Paired`
/// instead of retrying forever (spec: Per-pad lifecycle: 1–30 s, 5 attempts).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackoffPolicy {
    pub min: Duration,
    pub max: Duration,
    pub max_attempts: u32,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        BackoffPolicy {
            min: Duration::from_secs(1),
            max: Duration::from_secs(30),
            max_attempts: 5,
        }
    }
}

/// The state of one trigger's backoff sequence.
#[derive(Clone, Copy, Debug, Default)]
pub struct Backoff {
    policy: BackoffPolicy,
    attempts: u32,
}

impl Backoff {
    pub fn new(policy: BackoffPolicy) -> Self {
        Backoff {
            policy,
            attempts: 0,
        }
    }

    /// Delay before the next retry, or `None` once the attempt budget is
    /// spent. The budget counts *attempts*, not retries: with
    /// `max_attempts = 5`, the fifth failed attempt parks the pad.
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.attempts + 1 >= self.policy.max_attempts {
            // This call follows the last permitted attempt.
            self.attempts = self.policy.max_attempts;
            return None;
        }
        self.attempts += 1;
        let exp = self.policy.min.saturating_mul(1u32 << (self.attempts - 1).min(5));
        Some(exp.min(self.policy.max))
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }

    /// A trigger that succeeds resets its own budget.
    pub fn reset(&mut self) {
        self.attempts = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backoff_is_one_to_thirty_seconds_five_attempts() {
        let p = BackoffPolicy::default();
        assert_eq!(p.min, Duration::from_secs(1));
        assert_eq!(p.max, Duration::from_secs(30));
        assert_eq!(p.max_attempts, 5);
    }

    #[test]
    fn delays_grow_exponentially_and_stop_at_the_budget() {
        // Five attempts total: the first four failures schedule retries
        // (1, 2, 4, 8 s), the fifth parks the pad.
        let mut b = Backoff::new(BackoffPolicy::default());
        ;
        assert_eq!(b.next_delay(), Some(Duration::from_secs(1)));
        assert_eq!(b.next_delay(), Some(Duration::from_secs(2)));
        assert_eq!(b.next_delay(), Some(Duration::from_secs(4)));
        assert_eq!(b.next_delay(), Some(Duration::from_secs(8)));
        assert_eq!(b.next_delay(), None, "budget spent — park in Paired");
    }

    #[test]
    fn delays_are_capped_at_max() {
        let mut b = Backoff::new(BackoffPolicy {
            min: Duration::from_secs(4),
            max: Duration::from_secs(30),
            max_attempts: 5,
        });
        let mut seen = vec![];
        while let Some(d) = b.next_delay() {
            seen.push(d);
        }
        assert_eq!(
            seen,
            vec![
                Duration::from_secs(4),
                Duration::from_secs(8),
                Duration::from_secs(16),
                Duration::from_secs(30),
            ]
        );
    }

    #[test]
    fn success_resets_the_budget() {
        let mut b = Backoff::new(BackoffPolicy::default());
        b.next_delay();
        b.next_delay();
        b.reset();
        assert_eq!(b.next_delay(), Some(Duration::from_secs(1)));
    }
}
