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
    if expires_at_ms <= now_ms {
        true
    } else {
        expires_at_ms - now_ms < buffer_ms
    }
}

/// Converts seconds to ms. It saturates and does not overflow.
#[must_use]
pub const fn secs_to_ms(secs: u64) -> u64 {
    if secs <= u64::MAX / 1000 {
        secs * 1000
    } else {
        u64::MAX
    }
}

/// Returns the expiry instant for `expires_in` (seconds, or ms if `in_ms`).
#[must_use]
pub const fn expires_at_ms(now_ms: u64, expires_in: u64, in_ms: bool) -> u64 {
    let d = if in_ms {
        expires_in
    } else {
        secs_to_ms(expires_in)
    };
    if now_ms <= u64::MAX - d {
        now_ms + d
    } else {
        u64::MAX
    }
}

/// Returns the wait until `at_ms`. A past instant gives zero.
#[must_use]
// The body is the same text as in the Verus spec.
#[allow(clippy::implicit_saturating_sub)]
pub const fn wait_until_ms(now_ms: u64, at_ms: u64) -> u64 {
    if at_ms > now_ms { at_ms - now_ms } else { 0 }
}

/// Returns `true` while a session is live. It ends at `expires_at_ms`.
#[must_use]
pub const fn session_live(now_ms: u64, expires_at_ms: u64) -> bool {
    now_ms < expires_at_ms
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
    if force {
        return Gate::Proceed;
    }
    if exhausted {
        return Gate::Exhausted;
    }
    match last_failure_ms {
        Some(t) => {
            if now_ms < t || now_ms - t < cooldown_ms {
                Gate::Cooldown
            } else {
                Gate::Proceed
            }
        }
        None => Gate::Proceed,
    }
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
    let same_day = match last_failure_day {
        Some(d) => d == today,
        None => false,
    };
    let next = if same_day || attempts == u32::MAX {
        attempts
    } else {
        attempts + 1
    };
    (next, next >= max_days)
}

/// Returns the sync frequency, raised to the minimum.
#[must_use]
pub const fn sync_frequency_ms(requested_ms: u64) -> u64 {
    if requested_ms < MIN_SYNC_FREQUENCY_MS {
        MIN_SYNC_FREQUENCY_MS
    } else {
        requested_ms
    }
}

/// Returns the wait in ms before retry number `attempt` (1-based).
///
/// The wait is `initial_ms * 2^(attempt - 1)`, capped at `cap_ms`.
#[must_use]
pub const fn backoff_ms(initial_ms: u64, attempt: u32, cap_ms: u64) -> u64 {
    let mut b: u64 = if initial_ms <= cap_ms {
        initial_ms
    } else {
        cap_ms
    };
    let mut i: u32 = 1;
    while i < attempt {
        // At the cap or at zero, the value stays. A large `attempt` costs no time.
        if b == cap_ms || b == 0 {
            return b;
        }
        // Compare with no overflow: `cap - b <= b` is `2b >= cap`.
        b = if cap_ms - b <= b { cap_ms } else { b * 2 };
        i += 1;
    }
    b
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
    let retryable = match outcome {
        Outcome::Success => false,
        Outcome::RateLimited => true,
        Outcome::ServerError | Outcome::NetworkError => idempotent || opted_in,
        Outcome::ClientError => opted_in,
    };
    if !retryable {
        return Step::Done;
    }
    let max = if rule.max_attempts < MAX_ATTEMPTS {
        rule.max_attempts
    } else {
        MAX_ATTEMPTS
    };
    if attempt >= max {
        return Step::GiveUp;
    }
    let wait = match header_wait_ms {
        Some(w) => w,
        None => backoff_ms(rule.initial_backoff_ms, attempt, rule.max_wait_ms),
    };
    if wait <= rule.max_wait_ms {
        Step::Retry(wait)
    } else {
        Step::GiveUp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Returns the body of `pub fn name` with no comments and no white space.
    /// In the spec, the body starts at a line with only `{` (after `ensures`).
    /// Removes `/* ... */` comments.
    fn strip_block_comments(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find("/*") {
            out.push_str(&rest[..at]);
            rest = rest[at..]
                .find("*/")
                .map_or("", |end| &rest[at + end + 2..]);
        }
        out.push_str(rest);
        out
    }

    /// Returns each `pub const NAME: T = V;` line, with no spaces.
    fn constants(text: &str) -> std::collections::BTreeMap<String, String> {
        text.lines()
            .filter_map(|l| l.trim().strip_prefix("pub const "))
            .filter(|l| !l.contains("fn "))
            .filter_map(|l| {
                let (name, rest) = l.split_once(':')?;
                Some((
                    name.trim().to_owned(),
                    rest.chars().filter(|c| !c.is_whitespace()).collect(),
                ))
            })
            .collect()
    }

    fn body(text: &str, name: &str, spec: bool) -> String {
        let at = text
            .find(&format!("pub fn {name}("))
            .or_else(|| text.find(&format!("pub const fn {name}(")))
            .unwrap_or_else(|| panic!("no fn {name}"));
        let open = if spec { "\n{\n" } else { " {\n" };
        let start = at
            + text[at..]
                .find(open)
                .unwrap_or_else(|| panic!("no body for {name}"))
            + open.len();
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
        strip_block_comments(&text[start..end])
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
        // `backoff_ms` has proof code in its loop. A proptest checks it.
        for name in [
            "token_expired",
            "secs_to_ms",
            "expires_at_ms",
            "wait_until_ms",
            "session_live",
            "refresh_gate",
            "record_failure",
            "sync_frequency_ms",
            "decide",
        ] {
            assert_eq!(body(spec, name, true), body(code, name, false), "{name}");
        }
        // The constants that the proofs use must have the same values.
        let spec_consts = constants(spec);
        let code_consts = constants(code);
        assert!(spec_consts.len() >= 5, "{spec_consts:?}");
        for (name, value) in &spec_consts {
            assert_eq!(code_consts.get(name), Some(value), "{name}");
        }
    }

    #[test]
    fn the_drift_check_sees_changes() {
        let code =
            "pub const A: u64 = 1;\npub fn f(x: u64) -> u64 {\n    x /* note */ + 1 // tail\n}\n";
        assert_eq!(body(code, "f", false), "x+1");
        assert_eq!(constants(code).get("A").map(String::as_str), Some("u64=1;"));
        assert_eq!(strip_block_comments("a/*b*/c/*d"), "ac");
    }

    #[test]
    fn constants_match_the_plan() {
        // docs/plan.md AC9, AC11 and AC15 give these values.
        assert_eq!(DEFAULT_REFRESH_BUFFER_MS, 15 * 60 * 1000);
        assert_eq!(REFRESH_COOLDOWN_MS, 30 * 1000);
        assert_eq!(MAX_FAILED_REFRESH_DAYS, 4);
        assert_eq!(MAX_ATTEMPTS, 10);
        assert_eq!(MIN_SYNC_FREQUENCY_MS, 30 * 1000);
        assert_eq!(MS_PER_DAY, 24 * 60 * 60 * 1000);
    }

    #[test]
    fn expiry_at_a_real_time() {
        // 2023-11-14T22:13:20Z in ms. Tests with small times miss overflow
        // and operator mistakes.
        let now = 1_700_000_000_000_u64;
        assert_eq!(expires_at_ms(now, 4 * 3_600, false), now + 14_400_000);
        assert_eq!(expires_at_ms(now, 30 * 86_400, false), now + 2_592_000_000);
        assert_eq!(expires_at_ms(now, 14_400_000, true), now + 14_400_000);
        assert!(!token_expired(
            now,
            now + 14_400_000,
            DEFAULT_REFRESH_BUFFER_MS
        ));
        assert!(token_expired(now, now + 600_000, DEFAULT_REFRESH_BUFFER_MS));
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
        assert!(!token_expired(0, u64::MAX, u64::MAX));
        assert!(token_expired(1, u64::MAX, u64::MAX));
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
        assert_eq!(
            refresh_gate(false, false, Some(0), c - 1, c),
            Gate::Cooldown
        );
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
        assert_eq!(
            decide(RateLimited, 1, false, false, None, &RULE),
            Step::Retry(100)
        );
        assert_eq!(
            decide(RateLimited, 1, false, false, Some(700), &RULE),
            Step::Retry(700)
        );
        assert_eq!(
            decide(RateLimited, 1, false, false, Some(1_001), &RULE),
            Step::GiveUp
        );
        assert_eq!(
            decide(RateLimited, 3, true, false, None, &RULE),
            Step::GiveUp
        );
        assert_eq!(
            decide(ServerError, 2, true, false, None, &RULE),
            Step::Retry(200)
        );
        assert_eq!(
            decide(ServerError, 1, false, false, None, &RULE),
            Step::Done
        );
        assert_eq!(
            decide(ServerError, 1, false, true, None, &RULE),
            Step::Retry(100)
        );
        assert_eq!(
            decide(NetworkError, 1, true, false, None, &RULE),
            Step::Retry(100)
        );
        assert_eq!(
            decide(NetworkError, 1, false, false, None, &RULE),
            Step::Done
        );
        assert_eq!(decide(ClientError, 1, true, false, None, &RULE), Step::Done);
        assert_eq!(
            decide(ClientError, 1, false, true, Some(5), &RULE),
            Step::Retry(5)
        );
        let big = RetryRule {
            max_attempts: u32::MAX,
            ..RULE
        };
        assert_eq!(
            decide(RateLimited, MAX_ATTEMPTS, true, true, None, &big),
            Step::GiveUp
        );
        assert_eq!(
            decide(RateLimited, MAX_ATTEMPTS - 1, true, true, None, &big),
            Step::Retry(1_000)
        );
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
