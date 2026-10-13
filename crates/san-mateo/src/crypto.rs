//! AES-256-GCM seal and open with AAD, and keyed lookup hashes.
//!
//! A sealed value is `sm1.<key_id>.<nonce>.<ciphertext>`. The nonce and the
//! ciphertext (with the tag) are base64url with no padding. See ADR 0003.

use std::collections::BTreeMap;

use crate::error::{Error, Result};

/// The prefix of a sealed value.
pub const SEALED_PREFIX: &str = "sm1";

/// One 256-bit key.
#[derive(Clone)]
struct Key([u8; 32]);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(<redacted>)")
    }
}

/// Keys with ids. One key is active.
///
/// `seal` uses the active key. `open` uses the key that the sealed value
/// names. The lookup key for [`KeyRing::lookup_hash`] comes from the key with
/// the lookup id. It stays the same when the active key changes.
#[derive(Clone)]
pub struct KeyRing {
    keys: BTreeMap<String, Key>,
    active: String,
    lookup_id: String,
}

impl std::fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl KeyRing {
    /// Makes a ring with one key. The key is active and gives the lookup key.
    ///
    /// # Errors
    ///
    /// The id must be 1 to 32 chars of `[A-Za-z0-9_-]`.
    pub fn new(id: &str, key: [u8; 32]) -> Result<Self> {
        let _ = (id, key);
        todo!()
    }

    /// Makes a ring from a base64 (standard or URL-safe) 32-byte key.
    ///
    /// # Errors
    ///
    /// The value must decode to exactly 32 bytes. The id must be valid.
    pub fn from_base64(id: &str, key_b64: &str) -> Result<Self> {
        let _ = (id, key_b64);
        todo!()
    }

    /// Adds a key that can open data. The active key does not change.
    ///
    /// # Errors
    ///
    /// The id must be valid and new.
    pub fn with_key(self, id: &str, key: [u8; 32]) -> Result<Self> {
        let _ = (id, key);
        todo!()
    }

    /// Adds a base64 key that can open data.
    ///
    /// # Errors
    ///
    /// The value must decode to exactly 32 bytes. The id must be valid and new.
    pub fn with_base64_key(self, id: &str, key_b64: &str) -> Result<Self> {
        let _ = (id, key_b64);
        todo!()
    }

    /// Makes the key with `id` the active key.
    ///
    /// # Errors
    ///
    /// The ring must hold the key.
    pub fn activate(self, id: &str) -> Result<Self> {
        let _ = id;
        todo!()
    }

    /// Uses the key with `id` for lookup hashes.
    ///
    /// # Errors
    ///
    /// The ring must hold the key.
    pub fn lookup_with(self, id: &str) -> Result<Self> {
        let _ = id;
        todo!()
    }

    /// Returns the id of the active key.
    #[must_use]
    pub fn active_key_id(&self) -> &str {
        todo!()
    }

    /// Encrypts `plaintext`. `aad` binds the value to its row.
    ///
    /// # Errors
    ///
    /// Fails only when the cipher fails.
    pub fn seal(&self, plaintext: &[u8], aad: &[u8]) -> Result<String> {
        let _ = (plaintext, aad);
        todo!()
    }

    /// Decrypts a sealed value.
    ///
    /// # Errors
    ///
    /// [`Error::Crypto`] when the format, the key id, the AAD or the tag is wrong.
    pub fn open(&self, sealed: &str, aad: &[u8]) -> Result<Vec<u8>> {
        let _ = (sealed, aad);
        todo!()
    }

    /// Seals a value as JSON.
    ///
    /// # Errors
    ///
    /// Fails when the value does not serialize.
    pub fn seal_json<T: serde::Serialize>(&self, value: &T, aad: &[u8]) -> Result<String> {
        let _ = (value, aad);
        todo!()
    }

    /// Opens a JSON value.
    ///
    /// # Errors
    ///
    /// [`Error::Crypto`] when the value does not open or does not parse.
    pub fn open_json<T: serde::de::DeserializeOwned>(&self, sealed: &str, aad: &[u8]) -> Result<T> {
        let _ = (sealed, aad);
        todo!()
    }

    /// Returns the hex HMAC-SHA256 of `value` with the lookup key.
    ///
    /// Use it to find high-entropy tokens. Do not use it for passwords.
    #[must_use]
    pub fn lookup_hash(&self, value: &str) -> String {
        let _ = value;
        todo!()
    }

    /// Returns the hex HMAC-SHA256 of `data` with a key for `purpose`.
    ///
    /// The key comes from the lookup key and `purpose`, so each purpose has
    /// its own key.
    #[must_use]
    pub fn keyed_hash(&self, purpose: &str, data: &[u8]) -> String {
        let _ = (purpose, data);
        todo!()
    }
}

/// Returns `true` when `a` and `b` are equal. The time does not depend on the
/// position of the first difference.
#[must_use]
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let _ = (a, b);
    todo!()
}

/// Builds an AAD from parts. Each part has a length prefix, so different
/// parts never give the same AAD.
#[must_use]
pub fn aad(parts: &[&str]) -> Vec<u8> {
    let _ = parts;
    todo!()
}

#[allow(dead_code)]
fn invalid(msg: &str) -> Error {
    Error::Invalid(msg.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ring() -> KeyRing {
        KeyRing::new("k1", [7; 32]).unwrap()
    }

    #[test]
    fn seal_then_open() {
        let r = ring();
        let s = r.seal(b"hello", b"row-1").unwrap();
        assert!(s.starts_with("sm1.k1."));
        assert_eq!(s.split('.').count(), 4);
        assert_eq!(r.open(&s, b"row-1").unwrap(), b"hello");
    }

    #[test]
    fn wrong_aad_fails() {
        let r = ring();
        let s = r.seal(b"hello", b"row-1").unwrap();
        assert_eq!(r.open(&s, b"row-2"), Err(Error::Crypto));
    }

    #[test]
    fn tampered_value_fails() {
        let r = ring();
        let s = r.seal(b"hello", b"a").unwrap();
        let mut parts: Vec<String> = s.split('.').map(str::to_owned).collect();
        let mut ct = parts[3].clone().into_bytes();
        ct[0] = if ct[0] == b'A' { b'B' } else { b'A' };
        parts[3] = String::from_utf8(ct).unwrap();
        assert_eq!(r.open(&parts.join("."), b"a"), Err(Error::Crypto));
    }

    #[test]
    fn malformed_values_fail() {
        let r = ring();
        for bad in ["", "sm1", "sm1.k1", "sm1.k1.x", "sm2.k1.AAAA.AAAA", "sm1.k1.!!.AAAA", "sm1.k1.AAAAAAAAAAAAAAAA.AAAA"] {
            assert_eq!(r.open(bad, b""), Err(Error::Crypto), "{bad}");
        }
    }

    #[test]
    fn unknown_key_fails() {
        let s = ring().seal(b"x", b"").unwrap();
        let other = KeyRing::new("k2", [7; 32]).unwrap();
        assert_eq!(other.open(&s, b""), Err(Error::Crypto));
    }

    #[test]
    fn nonces_differ() {
        let r = ring();
        assert_ne!(r.seal(b"x", b"").unwrap(), r.seal(b"x", b"").unwrap());
    }

    #[test]
    fn rotation_keeps_old_data_readable() {
        let old = ring();
        let sealed_old = old.seal(b"old", b"r").unwrap();
        let rotated = old.with_key("k2", [9; 32]).unwrap().activate("k2").unwrap();
        assert_eq!(rotated.active_key_id(), "k2");
        assert_eq!(rotated.open(&sealed_old, b"r").unwrap(), b"old");
        let sealed_new = rotated.seal(b"new", b"r").unwrap();
        assert!(sealed_new.starts_with("sm1.k2."));
        assert_eq!(rotated.open(&sealed_new, b"r").unwrap(), b"new");
        // The lookup key stays on k1 after the rotation.
        assert_eq!(rotated.lookup_hash("t"), ring().lookup_hash("t"));
    }

    #[test]
    fn key_ids_are_checked() {
        for bad in ["", "a.b", "has space", "x".repeat(33).as_str()] {
            assert!(KeyRing::new(bad, [1; 32]).is_err(), "{bad}");
        }
        assert!(ring().with_key("k1", [2; 32]).is_err());
        assert!(ring().activate("nope").is_err());
        assert!(ring().lookup_with("nope").is_err());
    }

    #[test]
    fn base64_keys() {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode([3_u8; 32]);
        let url = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([3_u8; 32]);
        let a = KeyRing::from_base64("a", &b64).unwrap();
        let b = KeyRing::from_base64("a", &url).unwrap();
        assert_eq!(a.lookup_hash("x"), b.lookup_hash("x"));
        assert!(KeyRing::from_base64("a", "c2hvcnQ=").is_err());
        assert!(KeyRing::from_base64("a", "not base64 !").is_err());
        assert!(a.with_base64_key("b", &url).is_ok());
    }

    #[test]
    fn lookup_hash_is_keyed_and_stable() {
        let r = ring();
        let h = r.lookup_hash("token");
        assert_eq!(h.len(), 64);
        assert_eq!(h, r.lookup_hash("token"));
        assert_ne!(h, r.lookup_hash("token2"));
        assert_ne!(h, KeyRing::new("k1", [8; 32]).unwrap().lookup_hash("token"));
        assert_ne!(r.keyed_hash("records", b"token"), r.keyed_hash("other", b"token"));
        let moved = r.clone().with_key("k9", [5; 32]).unwrap().lookup_with("k9").unwrap();
        assert_ne!(moved.lookup_hash("token"), h);
    }

    #[test]
    fn json_round_trip() {
        let r = ring();
        let s = r.seal_json(&serde_json::json!({"a": 1}), b"j").unwrap();
        let v: serde_json::Value = r.open_json(&s, b"j").unwrap();
        assert_eq!(v["a"], 1);
        let not_json = r.seal(b"{", b"j").unwrap();
        assert_eq!(r.open_json::<serde_json::Value>(&not_json, b"j"), Err(Error::Crypto));
    }

    #[test]
    fn debug_has_no_key_bytes() {
        let d = format!("{:?}", ring());
        assert!(d.contains("k1"));
        assert!(!d.contains("7, 7"));
    }

    #[test]
    fn aad_parts_do_not_collide() {
        assert_ne!(aad(&["ab", "c"]), aad(&["a", "bc"]));
        assert_eq!(aad(&["a", "b"]), aad(&["a", "b"]));
    }

    #[test]
    fn constant_time_compare() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    proptest! {
        #[test]
        fn any_value_round_trips(data in proptest::collection::vec(any::<u8>(), 0..512), aad_bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
            let r = ring();
            let s = r.seal(&data, &aad_bytes).unwrap();
            prop_assert_eq!(r.open(&s, &aad_bytes).unwrap(), data);
        }
    }
}
