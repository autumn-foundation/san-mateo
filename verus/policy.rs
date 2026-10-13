//! Verus spec and proof for `crates/san-mateo/src/policy.rs`.
//!
//! Keep this file in step with `src/policy.rs`. The test
//! `policy::verus_bodies_match` compares the function bodies.
//! Run: `verus verus/policy.rs`.

use vstd::prelude::*;

verus! {

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

// ------------------------------------------------------------------- expiry

/// Spec: a token is expired when it expires before `now + buffer`.
/// A token that expires at `now` is expired, also with a zero buffer.
pub open spec fn spec_expired(now: int, expires_at: int, buffer: int) -> bool {
    expires_at <= now || expires_at - now < buffer
}

/// Returns `true` when the token must be refreshed now.
pub fn token_expired(now_ms: u64, expires_at_ms: u64, buffer_ms: u64) -> (r: bool)
    ensures
        r == spec_expired(now_ms as int, expires_at_ms as int, buffer_ms as int),
{
    if expires_at_ms <= now_ms {
        true
    } else {
        expires_at_ms - now_ms < buffer_ms
    }
}

/// A larger buffer marks all tokens that a smaller buffer marks.
proof fn lemma_expired_monotonic_in_buffer(now: int, exp: int, b1: int, b2: int)
    requires
        b1 <= b2,
        spec_expired(now, exp, b1),
    ensures
        spec_expired(now, exp, b2),
{
}

/// Time moves forward: an expired token stays expired.
proof fn lemma_expired_stays_expired(t1: int, t2: int, exp: int, buffer: int)
    requires
        t1 <= t2,
        spec_expired(t1, exp, buffer),
    ensures
        spec_expired(t2, exp, buffer),
{
}

/// Converts seconds to ms. It saturates and does not overflow.
pub fn secs_to_ms(secs: u64) -> (r: u64)
    ensures
        secs <= u64::MAX / 1000 ==> r == secs * 1000,
        secs > u64::MAX / 1000 ==> r == u64::MAX,
{
    if secs <= u64::MAX / 1000 {
        secs * 1000
    } else {
        u64::MAX
    }
}

pub open spec fn spec_min(a: int, b: int) -> int {
    if a <= b {
        a
    } else {
        b
    }
}

/// Spec: the expiry instant, capped at `u64::MAX`.
pub open spec fn spec_expires_at(now: int, expires_in: int, in_ms: bool) -> int {
    let d = if in_ms {
        expires_in
    } else {
        spec_min(expires_in * 1000, u64::MAX as int)
    };
    spec_min(now + d, u64::MAX as int)
}

/// Returns the expiry instant for `expires_in` (seconds, or ms if `in_ms`).
pub fn expires_at_ms(now_ms: u64, expires_in: u64, in_ms: bool) -> (r: u64)
    ensures
        r == spec_expires_at(now_ms as int, expires_in as int, in_ms),
        r >= now_ms,
{
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
pub fn wait_until_ms(now_ms: u64, at_ms: u64) -> (r: u64)
    ensures
        at_ms > now_ms ==> r == at_ms - now_ms,
        at_ms <= now_ms ==> r == 0,
{
    if at_ms > now_ms {
        at_ms - now_ms
    } else {
        0
    }
}

/// Returns `true` while a session is live. It ends at `expires_at_ms`.
pub fn session_live(now_ms: u64, expires_at_ms: u64) -> (r: bool)
    ensures
        r == (now_ms < expires_at_ms),
{
    now_ms < expires_at_ms
}

// ----------------------------------------------------------- refresh gate

/// What the refresh code does next.
pub enum Gate {
    /// Do the refresh.
    Proceed,
    /// A refresh failed a short time ago. Wait.
    Cooldown,
    /// The refresh failed on too many days. Stop.
    Exhausted,
}

/// Spec: the last failure is inside the cooldown window.
pub open spec fn spec_in_cooldown(last_failure: Option<u64>, now: int, cooldown: int) -> bool {
    match last_failure {
        Some(t) => now - (t as int) < cooldown,
        None => false,
    }
}

/// Decides if a refresh can start. `force` skips the checks.
pub fn refresh_gate(
    force: bool,
    exhausted: bool,
    last_failure_ms: Option<u64>,
    now_ms: u64,
    cooldown_ms: u64,
) -> (g: Gate)
    ensures
        // Each gate value holds exactly in its case.
        g is Proceed <==> (force || (!exhausted && !spec_in_cooldown(
            last_failure_ms,
            now_ms as int,
            cooldown_ms as int,
        ))),
        g is Exhausted <==> (!force && exhausted),
        g is Cooldown <==> (!force && !exhausted && spec_in_cooldown(
            last_failure_ms,
            now_ms as int,
            cooldown_ms as int,
        )),
{
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

/// Spec: the failure count after one more failure on day `today`.
pub open spec fn spec_failure_count(attempts: int, last_day: Option<u64>, today: int) -> int {
    match last_day {
        Some(d) => if d as int == today {
            attempts
        } else {
            spec_min(attempts + 1, u32::MAX as int)
        }
        None => spec_min(attempts + 1, u32::MAX as int),
    }
}

/// Counts a refresh failure. The count goes up at most one time each day.
/// Returns the new count and `true` when the connection is exhausted.
pub fn record_failure(attempts: u32, last_failure_day: Option<u64>, today: u64, max_days: u32) -> (r: (
    u32,
    bool,
))
    ensures
        r.0 == spec_failure_count(attempts as int, last_failure_day, today as int),
        r.0 >= attempts,
        r.0 <= attempts + 1,
        r.1 == (r.0 >= max_days),
{
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

/// Failures on `max_days` different days make the connection exhausted.
proof fn lemma_exhausted_after_max_days(max_days: u32)
    ensures
        spec_failure_count(max_days as int - 1, None, 0) >= max_days as int || max_days == 0,
{
}

/// Spec: the failure count after one failure on each day in `days`, in order.
pub open spec fn spec_count_over(days: Seq<u64>) -> int
    decreases days.len(),
{
    if days.len() == 0 {
        0
    } else {
        let before = days.drop_last();
        let last_day = if before.len() == 0 {
            None
        } else {
            Some(before.last())
        };
        spec_failure_count(spec_count_over(before), last_day, days.last() as int)
    }
}

/// Failures on days that go up count one each. So failures on
/// `MAX_FAILED_REFRESH_DAYS` different days give an exhausted connection.
proof fn lemma_failures_on_rising_days(days: Seq<u64>)
    requires
        days.len() <= u32::MAX,
        forall|i: int| 0 < i < days.len() ==> days[i - 1] < #[trigger] days[i],
    ensures
        spec_count_over(days) == days.len(),
        days.len() >= MAX_FAILED_REFRESH_DAYS ==> spec_count_over(days) >= MAX_FAILED_REFRESH_DAYS,
    decreases days.len(),
{
    if days.len() > 0 {
        let before = days.drop_last();
        assert forall|i: int| 0 < i < before.len() implies before[i - 1] < #[trigger] before[i] by {
            assert(before[i - 1] == days[i - 1]);
            assert(before[i] == days[i]);
        }
        lemma_failures_on_rising_days(before);
        let n = before.len() as int;
        assert(spec_count_over(before) == n);
        if before.len() > 0 {
            let prev = before.last();
            assert(prev == days[days.len() - 2]);
            assert(prev < days.last());
            assert(spec_failure_count(n, Some(prev), days.last() as int) == n + 1);
        } else {
            assert(spec_failure_count(0, None, days.last() as int) == 1);
        }
    }
}

/// More failures on one day do not change the count.
proof fn lemma_same_day_counts_once(attempts: int, day: u64)
    ensures
        spec_failure_count(attempts, Some(day), day as int) == attempts,
{
}

/// Returns the sync frequency, raised to the minimum.
pub fn sync_frequency_ms(requested_ms: u64) -> (r: u64)
    ensures
        r >= MIN_SYNC_FREQUENCY_MS,
        requested_ms >= MIN_SYNC_FREQUENCY_MS ==> r == requested_ms,
{
    if requested_ms < MIN_SYNC_FREQUENCY_MS {
        MIN_SYNC_FREQUENCY_MS
    } else {
        requested_ms
    }
}

// ------------------------------------------------------------------ backoff

/// Spec: `initial * 2^(attempt - 1)`, capped at `cap`.
pub open spec fn spec_backoff(initial: int, attempt: int, cap: int) -> int
    decreases attempt,
{
    if attempt <= 1 {
        spec_min(initial, cap)
    } else {
        spec_min(spec_backoff(initial, attempt - 1, cap) * 2, cap)
    }
}

proof fn lemma_backoff_bounds(initial: int, attempt: int, cap: int)
    requires
        initial >= 0,
        cap >= 0,
    ensures
        0 <= spec_backoff(initial, attempt, cap) <= cap,
    decreases attempt,
{
    if attempt > 1 {
        lemma_backoff_bounds(initial, attempt - 1, cap);
    }
}

/// The backoff does not decrease from one attempt to the next.
proof fn lemma_backoff_monotonic(initial: int, attempt: int, cap: int)
    requires
        initial >= 0,
        cap >= 0,
        attempt >= 1,
    ensures
        spec_backoff(initial, attempt, cap) <= spec_backoff(initial, attempt + 1, cap),
{
    lemma_backoff_bounds(initial, attempt, cap);
}

/// Once the backoff is at the cap, it stays at the cap.
proof fn lemma_backoff_stays_at_cap(initial: int, i: int, j: int, cap: int)
    requires
        cap >= 0,
        1 <= i <= j,
        spec_backoff(initial, i, cap) == cap,
    ensures
        spec_backoff(initial, j, cap) == cap,
    decreases j - i,
{
    if j > i {
        lemma_backoff_stays_at_cap(initial, i, j - 1, cap);
    }
}

/// Once the backoff is zero, it stays zero.
proof fn lemma_backoff_stays_at_zero(initial: int, i: int, j: int, cap: int)
    requires
        cap >= 0,
        1 <= i <= j,
        spec_backoff(initial, i, cap) == 0,
    ensures
        spec_backoff(initial, j, cap) == 0,
    decreases j - i,
{
    if j > i {
        lemma_backoff_stays_at_zero(initial, i, j - 1, cap);
    }
}

/// Returns the wait in ms before retry number `attempt` (1-based).
pub fn backoff_ms(initial_ms: u64, attempt: u32, cap_ms: u64) -> (r: u64)
    ensures
        r <= cap_ms,
        r == spec_backoff(initial_ms as int, attempt as int, cap_ms as int),
{
    let mut b: u64 = if initial_ms <= cap_ms {
        initial_ms
    } else {
        cap_ms
    };
    let mut i: u32 = 1;
    while i < attempt
        invariant
            1 <= i,
            attempt == 0 ==> i == 1,
            attempt >= 1 ==> i <= attempt,
            b <= cap_ms,
            b == spec_backoff(initial_ms as int, i as int, cap_ms as int),
        decreases attempt - i,
    {
        if b == cap_ms {
            proof {
                lemma_backoff_stays_at_cap(initial_ms as int, i as int, attempt as int, cap_ms as int);
            }
            return b;
        }
        if b == 0 {
            proof {
                lemma_backoff_stays_at_zero(initial_ms as int, i as int, attempt as int, cap_ms as int);
            }
            return b;
        }
        b = if cap_ms - b <= b {
            cap_ms
        } else {
            b * 2
        };
        i = i + 1;
    }
    assert(attempt == 0 ==> spec_backoff(initial_ms as int, attempt as int, cap_ms as int)
        == spec_backoff(initial_ms as int, 1, cap_ms as int));
    b
}

// ------------------------------------------------------------ retry decision

/// The class of one upstream attempt.
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
pub enum Step {
    /// Return the reply as it is.
    Done,
    /// Wait this many ms, then send again.
    Retry(u64),
    /// Stop. Return the last reply or error.
    GiveUp,
}

/// Retry rules for one call.
pub struct RetryRule {
    /// Attempts in total. Values above `MAX_ATTEMPTS` count as `MAX_ATTEMPTS`.
    pub max_attempts: u32,
    /// First backoff.
    pub initial_backoff_ms: u64,
    /// Largest wait before a retry.
    pub max_wait_ms: u64,
}

/// Spec: a retry is allowed for this outcome.
/// `opted_in` is `true` when the caller or the provider lists the status.
pub open spec fn spec_retryable(outcome: Outcome, idempotent: bool, opted_in: bool) -> bool {
    match outcome {
        Outcome::Success => false,
        Outcome::RateLimited => true,
        Outcome::ServerError | Outcome::NetworkError => idempotent || opted_in,
        Outcome::ClientError => opted_in,
    }
}

pub open spec fn spec_cap_attempts(max_attempts: u32) -> int {
    spec_min(max_attempts as int, MAX_ATTEMPTS as int)
}

/// Decides the next step after attempt number `attempt` (1-based).
pub fn decide(
    outcome: Outcome,
    attempt: u32,
    idempotent: bool,
    opted_in: bool,
    header_wait_ms: Option<u64>,
    rule: &RetryRule,
) -> (s: Step)
    ensures
        // Bounded attempts, also for a large `max_attempts`.
        s is Retry ==> attempt < spec_cap_attempts(rule.max_attempts),
        // Bounded wait.
        s is Retry ==> s->Retry_0 <= rule.max_wait_ms,
        // A write call never repeats after a 5xx or a network error,
        // unless the caller opts in.
        s is Retry ==> spec_retryable(outcome, idempotent, opted_in),
        // An outcome that is not retryable ends the call with `Done`.
        !spec_retryable(outcome, idempotent, opted_in) ==> s is Done,
        s is Done ==> !spec_retryable(outcome, idempotent, opted_in),
        // A retry with a header waits exactly as long as the header says.
        (s is Retry && header_wait_ms is Some) ==> s->Retry_0 == header_wait_ms->Some_0,
        // A retry with no header waits exactly the capped backoff.
        (s is Retry && header_wait_ms is None) ==> s->Retry_0 == spec_backoff(
            rule.initial_backoff_ms as int,
            attempt as int,
            rule.max_wait_ms as int,
        ),
        // A retryable outcome with attempts left and a wait in the cap retries.
        (spec_retryable(outcome, idempotent, opted_in) && attempt < spec_cap_attempts(
            rule.max_attempts,
        ) && (match header_wait_ms {
            Some(w) => w <= rule.max_wait_ms,
            None => true,
        })) ==> s is Retry,
{
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

/// One step of the proxy loop: a retry at `attempt < max` gives a next
/// attempt that is still `<= max`. The loop is in `src/service/proxying.rs`. Tests check it.
proof fn lemma_attempts_bounded(attempt: u32, max_attempts: u32)
    requires
        attempt < spec_cap_attempts(max_attempts),
    ensures
        attempt + 1 <= MAX_ATTEMPTS,
{
}

fn main() {
}

} // verus!
