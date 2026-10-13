//! A secret string. Its `Debug` output never shows the value.

use serde::{Deserialize, Serialize};

/// A secret string, for example a token or a password.
///
/// `Debug` shows `Secret(<redacted>)`. There is no `Display`. Use
/// [`Secret::expose`] to read the value.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    /// Makes a secret.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the value. Do not log it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Returns `true` when the value is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_value() {
        let s = Secret::new("tok-123");
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        assert_eq!(s.expose(), "tok-123");
        assert!(!s.is_empty());
        assert!(Secret::default().is_empty());
    }

    #[test]
    fn serde_is_transparent() {
        let s: Secret = serde_json::from_str("\"abc\"").unwrap();
        assert_eq!(s.expose(), "abc");
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"abc\"");
    }
}
