//! Pure policy functions: token expiry, refresh gate, failure count, retry
//! decision and backoff.
//!
//! `verus/policy.rs` holds the spec and the proofs. Keep the bodies the same.
//! The test `verus_bodies_match` compares them.

/// Default refresh margin: 15 minutes.
pub const DEFAULT_REFRESH_BUFFER_MS: u64 = 900_000;
/// Wait after a failed refresh before the next refresh.
pub const REFRESH_COOLDOWN_MS: u64 = 30_000;
/// Number of days with a failed refresh that makes a connection exhausted.
pub const MAX_FAILED_REFRESH_DAYS: u32 = 4;
/// Largest number of attempts for one proxy call.
pub const MAX_ATTEMPTS: u32 = 10;
/// Shortest sync frequency: 30 seconds.
pub const MIN_SYNC_FREQUENCY_MS: u64 = 30_000;
/// Milliseconds in one day.
pub const MS_PER_DAY: u64 = 86_400_000;

/// Returns `true` when the token must be refreshed now.
///
/// A token is expired when it expires before `now + buffer`.
#[must_use]
pub const fn token_expired(now_ms: u64, expires_at_ms: u64, buffer_ms: u64) -> bool {
    let _ = (now_ms, expires_at_ms, buffer_ms);
    todo!()
}

/// Converts seconds to ms. It saturates and does not overflow.
#[must_use]
pub const fn secs_to_ms(secs: u64) -> u64 {
    let _ = secs;
    todo!()
}

/// Returns the expiry instant for `expires_in` (seconds, or ms if `in_ms`).
#[must_use]
pub const fn expires_at_ms(now_ms: u64, expires_in: u64, in_ms: bool) -> u64 {
    let _ = (now_ms, expires_in, in_ms);
    todo!()
}

/// Returns the wait until `at_ms`. A past instant gives zero.
#[must_use]
pub const fn wait_until_ms(now_ms: u64, at_ms: u64) -> u64 {
    let _ = (now_ms, at_ms);
    todo!()
}

/// Returns `true` while a session is live. It ends at `expires_at_ms`.
#[must_use]
pub const fn session_live(now_ms: u64, expires_at_ms: u64) -> bool {
    let _ = (now_ms, expires_at_ms);
    todo!()
}

/// What the refresh code does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// Do the refresh.
    Proceed,
    /// A refresh failed a short time ago. Wait.
    Cooldown,
    /// The refresh failed on too many days. Stop.
    Exhausted,
}

/// Decides if a refresh can start. `force` skips the checks.
#[must_use]
pub const fn refresh_gate(
    force: bool,
    exhausted: bool,
    last_failure_ms: Option<u64>,
    now_ms: u64,
    cooldown_ms: u64,
) -> Gate {
    let _ = (force, exhausted, last_failure_ms, now_ms, cooldown_ms);
    todo!()
}

/// Counts a refresh failure. The count goes up at most one time each day.
/// Returns the new count and `true` when the connection is exhausted.
#[must_use]
pub const fn record_failure(
    attempts: u32,
    last_failure_day: Option<u64>,
    today: u64,
    max_days: u32,
) -> (u32, bool) {
    let _ = (attempts, last_failure_day, today, max_days);
    todo!()
}

/// Returns the sync frequency, raised to the minimum.
#[must_use]
pub const fn sync_frequency_ms(requested_ms: u64) -> u64 {
    let _ = requested_ms;
    todo!()
}

/// Returns the wait in ms before retry number `attempt` (1-based).
///
/// The wait is `initial_ms * 2^(attempt - 1)`, capped at `cap_ms`.
#[must_use]
pub const fn backoff_ms(initial_ms: u64, attempt: u32, cap_ms: u64) -> u64 {
    let _ = (initial_ms, attempt, cap_ms);
    todo!()
}

/// The class of one upstream attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// HTTP 1xx, 2xx or 3xx.
    Success,
    /// HTTP 429, or a provider header that says no calls remain.
    RateLimited,
    /// HTTP 5xx.
    ServerError,
    /// No HTTP reply: connect error or timeout.
    NetworkError,
    /// Other HTTP 4xx.
    ClientError,
}

/// What the proxy does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Return the reply as it is.
    Done,
    /// Wait this many ms, then send again.
    Retry(u64),
    /// Stop. Return the last reply or error.
    GiveUp,
}

/// Retry rules for one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryRule {
    /// Attempts in total. Values above `MAX_ATTEMPTS` count as `MAX_ATTEMPTS`.
    pub max_attempts: u32,
    /// First backoff.
    pub initial_backoff_ms: u64,
    /// Largest wait before a retry.
    pub max_wait_ms: u64,
}

/// Decides the next step after attempt number `attempt` (1-based).
///
/// `opted_in` is `true` when the caller or the provider lists the status.
#[must_use]
pub const fn decide(
    outcome: Outcome,
    attempt: u32,
    idempotent: bool,
    opted_in: bool,
    header_wait_ms: Option<u64>,
    rule: &RetryRule,
) -> Step {
    let _ = (outcome, attempt, idempotent, opted_in, header_wait_ms, rule);
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Returns the body of `pub fn name`, from `first` to the closing brace,
    /// with no comments and no white space.
    fn body(text: &str, name: &str, first: &str) -> String {
        let at = text
            .find(&format!("pub fn {name}("))
            .or_else(|| text.find(&format!("pub const fn {name}(")))
            .unwrap_or_else(|| panic!("no fn {name}"));
        let start = at + text[at..].find(first).unwrap_or_else(|| panic!("no {first}"));
        let mut depth = 1_i32;
        let mut end = text.len();
        for (i, c) in text[start..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        text[start..end]
            .lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .collect::<String>()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }

    #[test]
    fn verus_bodies_match() {
        let spec = include_str!("../../../verus/policy.rs");
        let code = include_str!("policy.rs");
        for (name, first) in [
            ("token_expired", "if expires_at_ms <= now_ms"),
            ("secs_to_ms", "if secs <= u64::MAX / 1000"),
            ("expires_at_ms", "let d = if in_ms"),
            ("wait_until_ms", "if at_ms > now_ms"),
            ("session_live", "now_ms < expires_at_ms"),
            ("refresh_gate", "if force"),
            ("record_failure", "let same_day"),
            ("sync_frequency_ms", "if requested_ms < MIN_SYNC_FREQUENCY_MS"),
            ("decide", "let retryable"),
        ] {
            assert_eq!(body(spec, name, first), body(code, name, first), "{name}");
        }
    }

    #[test]
    fn expiry_edges() {
        assert!(token_expired(1_000, 1_000, 0));
        assert!(token_expired(1_000, 999, 0));
        assert!(!token_expired(1_000, 1_001, 0));
        assert!(token_expired(1_000, 1_500, 501));
        assert!(!token_expired(1_000, 1_500, 500));
        assert!(token_expired(u64::MAX, 0, 0));
        assert!(!token_expired(0, u64::MAX, u64::MAX - 1));
        assert!(token_expired(0, u64::MAX, u64::MAX));
    }

    #[test]
    fn seconds_and_expiry_saturate() {
        assert_eq!(secs_to_ms(0), 0);
        assert_eq!(secs_to_ms(3_600), 3_600_000);
        assert_eq!(secs_to_ms(u64::MAX / 1000), (u64::MAX / 1000) * 1000);
        assert_eq!(secs_to_ms(u64::MAX / 1000 + 1), u64::MAX);
        assert_eq!(expires_at_ms(1_000, 60, false), 61_000);
        assert_eq!(expires_at_ms(1_000, 60, true), 1_060);
        assert_eq!(expires_at_ms(u64::MAX - 5, 60, true), u64::MAX);
        assert_eq!(expires_at_ms(5, u64::MAX, false), u64::MAX);
    }

    #[test]
    fn wait_and_session_edges() {
        assert_eq!(wait_until_ms(1_000, 4_000), 3_000);
        assert_eq!(wait_until_ms(4_000, 1_000), 0);
        assert_eq!(wait_until_ms(4_000, 4_000), 0);
        assert!(session_live(999, 1_000));
        assert!(!session_live(1_000, 1_000));
    }

    #[test]
    fn gate_rules() {
        let c = REFRESH_COOLDOWN_MS;
        assert_eq!(refresh_gate(true, true, Some(10), 10, c), Gate::Proceed);
        assert_eq!(refresh_gate(false, true, None, 10, c), Gate::Exhausted);
        assert_eq!(refresh_gate(false, false, None, 10, c), Gate::Proceed);
        assert_eq!(refresh_gate(false, false, Some(0), c - 1, c), Gate::Cooldown);
        assert_eq!(refresh_gate(false, false, Some(0), c, c), Gate::Proceed);
        // A failure time in the future (clock skew) is inside the cooldown.
        assert_eq!(refresh_gate(false, false, Some(50), 10, c), Gate::Cooldown);
    }

    #[test]
    fn failure_counts_once_per_day() {
        assert_eq!(record_failure(0, None, 5, 4), (1, false));
        assert_eq!(record_failure(1, Some(5), 5, 4), (1, false));
        assert_eq!(record_failure(1, Some(5), 6, 4), (2, false));
        assert_eq!(record_failure(3, Some(6), 7, 4), (4, true));
        assert_eq!(record_failure(4, Some(7), 7, 4), (4, true));
        assert_eq!(record_failure(u32::MAX, Some(1), 2, 4), (u32::MAX, true));
    }

    #[test]
    fn sync_frequency_has_a_floor() {
        assert_eq!(sync_frequency_ms(0), MIN_SYNC_FREQUENCY_MS);
        assert_eq!(sync_frequency_ms(29_999), MIN_SYNC_FREQUENCY_MS);
        assert_eq!(sync_frequency_ms(60_000), 60_000);
    }

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff_ms(100, 0, 1_000), 100);
        assert_eq!(backoff_ms(100, 1, 1_000), 100);
        assert_eq!(backoff_ms(100, 2, 1_000), 200);
        assert_eq!(backoff_ms(100, 4, 1_000), 800);
        assert_eq!(backoff_ms(100, 5, 1_000), 1_000);
        assert_eq!(backoff_ms(5_000, 1, 1_000), 1_000);
        assert_eq!(backoff_ms(0, 9, 1_000), 0);
        assert_eq!(backoff_ms(u64::MAX, 60, u64::MAX), u64::MAX);
        assert_eq!(backoff_ms(3, u32::MAX, u64::MAX), u64::MAX);
    }

    const RULE: RetryRule = RetryRule {
        max_attempts: 3,
        initial_backoff_ms: 100,
        max_wait_ms: 1_000,
    };

    #[test]
    fn decide_examples() {
        use Outcome::{ClientError, NetworkError, RateLimited, ServerError, Success};
        assert_eq!(decide(Success, 1, true, true, None, &RULE), Step::Done);
        assert_eq!(decide(RateLimited, 1, false, false, None, &RULE), Step::Retry(100));
        assert_eq!(decide(RateLimited, 1, false, false, Some(700), &RULE), Step::Retry(700));
        assert_eq!(decide(RateLimited, 1, false, false, Some(1_001), &RULE), Step::GiveUp);
        assert_eq!(decide(RateLimited, 3, true, false, None, &RULE), Step::GiveUp);
        assert_eq!(decide(ServerError, 2, true, false, None, &RULE), Step::Retry(200));
        assert_eq!(decide(ServerError, 1, false, false, None, &RULE), Step::Done);
        assert_eq!(decide(ServerError, 1, false, true, None, &RULE), Step::Retry(100));
        assert_eq!(decide(NetworkError, 1, true, false, None, &RULE), Step::Retry(100));
        assert_eq!(decide(NetworkError, 1, false, false, None, &RULE), Step::Done);
        assert_eq!(decide(ClientError, 1, true, false, None, &RULE), Step::Done);
        assert_eq!(decide(ClientError, 1, false, true, Some(5), &RULE), Step::Retry(5));
        let big = RetryRule {
            max_attempts: u32::MAX,
            ..RULE
        };
        assert_eq!(decide(RateLimited, MAX_ATTEMPTS, true, true, None, &big), Step::GiveUp);
        assert_eq!(decide(RateLimited, MAX_ATTEMPTS - 1, true, true, None, &big), Step::Retry(1_000));
    }

    fn any_outcome() -> impl Strategy<Value = Outcome> {
        prop_oneof![
            Just(Outcome::Success),
            Just(Outcome::RateLimited),
            Just(Outcome::ServerError),
            Just(Outcome::NetworkError),
            Just(Outcome::ClientError),
        ]
    }

    /// Closed form of the backoff, with 128-bit math.
    fn closed_form(initial: u64, attempt: u32, cap: u64) -> u64 {
        let exp = attempt.max(1) - 1;
        if exp >= 64 {
            return if initial == 0 { 0 } else { cap };
        }
        let raw = u128::from(initial) << exp;
        u64::try_from(raw.min(u128::from(cap))).unwrap_or(cap)
    }

    proptest! {
        #[test]
        fn backoff_matches_closed_form(initial in 0_u64..1_000_000, attempt in 0_u32..80, cap in 0_u64..10_000_000) {
            prop_assert_eq!(backoff_ms(initial, attempt, cap), closed_form(initial, attempt, cap));
        }

        #[test]
        fn decide_invariants(
            outcome in any_outcome(),
            attempt in 1_u32..20,
            idempotent: bool,
            opted_in: bool,
            header in proptest::option::of(0_u64..5_000),
            max_attempts in 0_u32..20,
        ) {
            let rule = RetryRule { max_attempts, initial_backoff_ms: 50, max_wait_ms: 2_000 };
            let step = decide(outcome, attempt, idempotent, opted_in, header, &rule);
            let retryable = match outcome {
                Outcome::Success => false,
                Outcome::RateLimited => true,
                Outcome::ServerError | Outcome::NetworkError => idempotent || opted_in,
                Outcome::ClientError => opted_in,
            };
            if let Step::Retry(wait) = step {
                prop_assert!(retryable);
                prop_assert!(attempt < max_attempts.min(MAX_ATTEMPTS));
                prop_assert!(wait <= rule.max_wait_ms);
                if let Some(h) = header {
                    prop_assert_eq!(wait, h);
                }
            }
            prop_assert_eq!(step == Step::Done, !retryable);
        }

        #[test]
        fn expired_is_monotonic_in_time(now in 0_u64..1_000_000, later in 0_u64..1_000_000, exp in 0_u64..2_000_000, buf in 0_u64..100_000) {
            if token_expired(now, exp, buf) {
                prop_assert!(token_expired(now.saturating_add(later), exp, buf));
            }
        }
    }
}
