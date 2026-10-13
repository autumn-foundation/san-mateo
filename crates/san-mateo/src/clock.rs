//! Time and randomness seams. Tests use fixed versions.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Utc};

/// A source of the current time.
pub trait Clock: Send + Sync + 'static {
    /// Returns the current time.
    fn now(&self) -> DateTime<Utc>;

    /// Returns the current time as ms since the Unix epoch. Times before the
    /// epoch give zero.
    fn now_ms(&self) -> u64 {
        to_ms(self.now())
    }
}

/// Converts a time to ms since the Unix epoch. Times before the epoch give zero.
#[must_use]
pub fn to_ms(t: DateTime<Utc>) -> u64 {
    let _ = t;
    todo!()
}

/// Converts ms since the Unix epoch to a time. Values out of range give the
/// largest time that `chrono` supports.
#[must_use]
pub fn from_ms(ms: u64) -> DateTime<Utc> {
    let _ = ms;
    todo!()
}

/// Ms values at or above this are epoch ms. Smaller values are epoch seconds.
/// It is 1972-01-01 in ms.
pub const EPOCH_MS_THRESHOLD: f64 = 63_072_000_000.0;

/// Parses an instant from JSON: a number (epoch seconds or epoch ms), or a
/// string (a number, RFC 3339, or an HTTP date).
#[must_use]
pub fn parse_instant(v: &serde_json::Value) -> Option<DateTime<Utc>> {
    let _ = v;
    todo!()
}

/// Parses an instant from a string: a number (epoch seconds or epoch ms),
/// RFC 3339, or an HTTP date (RFC 2822 form).
#[must_use]
pub fn parse_instant_str(s: &str) -> Option<DateTime<Utc>> {
    let _ = s;
    todo!()
}

/// The system clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        todo!()
    }
}

/// A clock that moves only when a test moves it.
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<DateTime<Utc>>,
}

impl ManualClock {
    /// Makes a clock at `start`.
    #[must_use]
    pub const fn new(start: DateTime<Utc>) -> Self {
        Self {
            now: Mutex::new(start),
        }
    }

    /// Sets the time.
    pub fn set(&self, t: DateTime<Utc>) {
        let _ = t;
        todo!()
    }

    /// Moves the time forward.
    pub fn advance(&self, by: chrono::Duration) {
        let _ = by;
        todo!()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        todo!()
    }
}

/// A source of random bytes.
pub trait Entropy: Send + Sync + 'static {
    /// Fills `buf` with random bytes.
    fn fill(&self, buf: &mut [u8]);

    /// Returns `bytes` random bytes as lower-case hex.
    fn hex(&self, bytes: usize) -> String {
        let _ = bytes;
        todo!()
    }

    /// Returns a random UUID v4.
    fn uuid(&self) -> uuid::Uuid {
        todo!()
    }
}

/// Random bytes from the operating system.
#[derive(Debug, Clone, Copy, Default)]
pub struct OsEntropy;

impl Entropy for OsEntropy {
    fn fill(&self, buf: &mut [u8]) {
        let _ = buf;
        todo!()
    }
}

/// Bytes from a counter. Each call gives new bytes. Only for tests.
#[derive(Debug, Default)]
pub struct SeqEntropy {
    next: AtomicU64,
}

impl Entropy for SeqEntropy {
    fn fill(&self, buf: &mut [u8]) {
        let _ = (buf, &self.next, Ordering::Relaxed);
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn ms_conversions() {
        let t = Utc.timestamp_millis_opt(1_700_000_000_123).unwrap();
        assert_eq!(to_ms(t), 1_700_000_000_123);
        assert_eq!(from_ms(1_700_000_000_123), t);
        let before = Utc.timestamp_millis_opt(-5).unwrap();
        assert_eq!(to_ms(before), 0);
        assert_eq!(from_ms(u64::MAX), DateTime::<Utc>::MAX_UTC);
    }

    #[test]
    fn instants() {
        use serde_json::json;
        let t = Utc.timestamp_opt(1_700_000_000, 0).unwrap();
        assert_eq!(parse_instant(&json!(1_700_000_000)), Some(t));
        assert_eq!(parse_instant(&json!(1_700_000_000_000_i64)), Some(t));
        assert_eq!(parse_instant(&json!(1_700_000_000.5)), Some(t + chrono::Duration::milliseconds(500)));
        assert_eq!(parse_instant(&json!("1700000000")), Some(t));
        assert_eq!(parse_instant(&json!("2023-11-14T22:13:20Z")), Some(t));
        assert_eq!(parse_instant(&json!("2023-11-14T23:13:20+01:00")), Some(t));
        assert_eq!(parse_instant_str("Tue, 14 Nov 2023 22:13:20 GMT"), Some(t));
        assert_eq!(parse_instant(&json!("soon")), None);
        assert_eq!(parse_instant(&json!(-5)), None);
        assert_eq!(parse_instant(&json!(null)), None);
        assert_eq!(parse_instant(&json!({"a": 1})), None);
        assert_eq!(parse_instant(&json!(f64::MAX)), None);
    }

    #[test]
    fn manual_clock_moves_only_on_request() {
        let start = Utc.timestamp_millis_opt(1_000).unwrap();
        let c = ManualClock::new(start);
        assert_eq!(c.now(), start);
        c.advance(chrono::Duration::seconds(2));
        assert_eq!(c.now_ms(), 3_000);
        c.set(Utc.timestamp_millis_opt(10).unwrap());
        assert_eq!(c.now_ms(), 10);
    }

    #[test]
    fn system_clock_is_after_2024() {
        assert!(SystemClock.now_ms() > 1_704_067_200_000);
    }

    #[test]
    fn entropy_gives_new_values() {
        let s = SeqEntropy::default();
        let a = s.hex(32);
        let b = s.hex(32);
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(s.uuid(), s.uuid());
        assert_eq!(s.uuid().get_version_num(), 4);
        let os = OsEntropy;
        assert_ne!(os.hex(32), os.hex(32));
    }
}
