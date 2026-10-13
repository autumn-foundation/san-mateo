//! Records that syncs save: stable ids, keyed content hashes, change
//! classes and cursors.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::crypto::KeyRing;
use crate::error::{Error, Result};

/// The largest page of records.
pub const MAX_LIMIT: u32 = 10_000;
/// The default page of records.
pub const DEFAULT_LIMIT: u32 = 100;
/// The largest number of ids in one query.
pub const MAX_IDS: usize = 100;

/// The last change of a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LastAction {
    /// The record is new, or it came back after a delete.
    Added,
    /// The record changed.
    Updated,
    /// The record is deleted.
    Deleted,
}

impl LastAction {
    /// Parses `added`, `updated` or `deleted`. Case does not matter.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let _ = s;
        todo!()
    }
}

/// The result of one write to one record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// A new record, or a record that came back after a delete.
    Added,
    /// The content changed.
    Updated,
    /// The content is the same.
    Unchanged,
    /// The record is now deleted.
    Deleted,
    /// A delete of a record that is missing or already deleted.
    Ignored,
}

/// The stored state of a record that a write compares with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordState {
    /// The keyed content hash.
    pub data_hash: String,
    /// `true` when the record is deleted.
    pub deleted: bool,
}

/// Classifies an upsert.
#[must_use]
pub fn classify_upsert(existing: Option<&RecordState>, new_hash: &str) -> Change {
    let _ = (existing, new_hash);
    todo!()
}

/// Classifies a delete.
#[must_use]
pub fn classify_delete(existing: Option<&RecordState>) -> Change {
    let _ = existing;
    todo!()
}

/// Returns the stable id of a record: a UUID v5 from the connection, the
/// model and the external id.
#[must_use]
pub fn record_id(connection: Uuid, model: &str, external_id: &str) -> Uuid {
    let _ = (connection, model, external_id);
    todo!()
}

/// Returns the external id of a record: its `id` field (a string or a number).
///
/// # Errors
///
/// [`Error::Invalid`] when the field is missing, empty or of another type.
pub fn external_id(record: &Value) -> Result<String> {
    let _ = record;
    todo!()
}

/// Returns JSON text with sorted object keys and no white space.
#[must_use]
pub fn canonical_json(value: &Value) -> String {
    let _ = value;
    todo!()
}

/// Returns the keyed content hash of a record.
#[must_use]
pub fn data_hash(keys: &KeyRing, value: &Value) -> String {
    let _ = (keys, value);
    todo!()
}

/// A position in a record list: after this `(updated_at, id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cursor {
    /// The change time of the last record, in microseconds.
    pub updated_at_us: i64,
    /// The id of the last record.
    pub id: Uuid,
}

impl Cursor {
    /// Encodes the cursor as base64url text.
    #[must_use]
    pub fn encode(&self) -> String {
        todo!()
    }

    /// Decodes a cursor.
    ///
    /// # Errors
    ///
    /// [`Error::Invalid`] when the text is not a cursor.
    pub fn decode(s: &str) -> Result<Self> {
        let _ = s;
        todo!()
    }
}

/// Truncates a time to whole microseconds. Stores keep microseconds.
#[must_use]
pub fn to_micros(t: DateTime<Utc>) -> i64 {
    t.timestamp_micros()
}

/// Counts of a batch write, by external id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchOutcome {
    /// Ids of added records.
    pub added: Vec<String>,
    /// Ids of updated records.
    pub updated: Vec<String>,
    /// Ids of deleted records.
    pub deleted: Vec<String>,
    /// Ids of records with no change.
    pub unchanged: Vec<String>,
}

impl BatchOutcome {
    /// Adds one result.
    pub fn push(&mut self, external_id: String, change: Change) {
        let _ = (external_id, change);
        todo!()
    }

    /// Adds the counts of `other`.
    pub fn extend(&mut self, other: Self) {
        let _ = other;
        todo!()
    }
}

/// A query for records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordQuery {
    /// The model name.
    pub model: String,
    /// Return records after this position.
    pub cursor: Option<Cursor>,
    /// Return records changed at or after this time, in microseconds.
    pub modified_after_us: Option<i64>,
    /// The page size, 1 to [`MAX_LIMIT`].
    pub limit: u32,
    /// Return only records with these last actions. Empty means all.
    pub filter: Vec<LastAction>,
    /// Return only records with these external ids. Empty means all.
    pub external_ids: Vec<String>,
}

/// Returns the page size: the default when `None`, else checked.
///
/// # Errors
///
/// [`Error::Invalid`] when the value is 0 or above [`MAX_LIMIT`].
pub fn check_limit(limit: Option<u32>) -> Result<u32> {
    let _ = limit;
    todo!()
}

/// Parses a filter such as `added,updated`.
///
/// # Errors
///
/// [`Error::Invalid`] for an unknown action.
pub fn parse_filter(s: &str) -> Result<Vec<LastAction>> {
    let _ = s;
    todo!()
}

#[allow(dead_code)]
fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use serde_json::json;

    #[test]
    fn upsert_classes() {
        let live = RecordState { data_hash: "h1".into(), deleted: false };
        let dead = RecordState { data_hash: "h1".into(), deleted: true };
        assert_eq!(classify_upsert(None, "h1"), Change::Added);
        assert_eq!(classify_upsert(Some(&live), "h1"), Change::Unchanged);
        assert_eq!(classify_upsert(Some(&live), "h2"), Change::Updated);
        assert_eq!(classify_upsert(Some(&dead), "h1"), Change::Added);
        assert_eq!(classify_delete(Some(&live)), Change::Deleted);
        assert_eq!(classify_delete(Some(&dead)), Change::Ignored);
        assert_eq!(classify_delete(None), Change::Ignored);
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        let c = Uuid::from_u128(1);
        let a = record_id(c, "contacts", "42");
        assert_eq!(a, record_id(c, "contacts", "42"));
        assert_eq!(a.get_version_num(), 5);
        assert_ne!(a, record_id(c, "contacts", "43"));
        assert_ne!(a, record_id(c, "deals", "42"));
        assert_ne!(a, record_id(Uuid::from_u128(2), "contacts", "42"));
        // The parts have length prefixes: no collision when text moves.
        assert_ne!(record_id(c, "ab", "c"), record_id(c, "a", "bc"));
    }

    #[test]
    fn external_ids() {
        assert_eq!(external_id(&json!({"id": "a1"})).unwrap(), "a1");
        assert_eq!(external_id(&json!({"id": 7})).unwrap(), "7");
        for bad in [json!({}), json!({"id": ""}), json!({"id": null}), json!({"id": [1]}), json!(5)] {
            assert!(external_id(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn canonical_json_sorts_keys() {
        assert_eq!(canonical_json(&json!({"b": 1, "a": {"d": [2, {"z": 1, "y": 2}], "c": null}})), r#"{"a":{"c":null,"d":[2,{"y":2,"z":1}]},"b":1}"#);
        assert_eq!(canonical_json(&json!("x\"y")), r#""x\"y""#);
    }

    #[test]
    fn hashes_ignore_key_order_and_use_the_key() {
        let k = KeyRing::new("k", [1; 32]).unwrap();
        let h1 = data_hash(&k, &json!({"a": 1, "b": 2}));
        assert_eq!(h1, data_hash(&k, &json!({"b": 2, "a": 1})));
        assert_ne!(h1, data_hash(&k, &json!({"a": 1, "b": 3})));
        assert_ne!(h1, data_hash(&KeyRing::new("k", [2; 32]).unwrap(), &json!({"a": 1, "b": 2})));
    }

    #[test]
    fn cursor_round_trip_and_errors() {
        let c = Cursor { updated_at_us: 1_700_000_000_123_456, id: Uuid::from_u128(99) };
        assert_eq!(Cursor::decode(&c.encode()).unwrap(), c);
        assert!(!c.encode().contains('='));
        for bad in ["", "!!!", "YWJj", "MTI6bm90LWEtdXVpZA"] {
            assert!(Cursor::decode(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn batch_counts() {
        let mut b = BatchOutcome::default();
        b.push("1".into(), Change::Added);
        b.push("2".into(), Change::Updated);
        b.push("3".into(), Change::Deleted);
        b.push("4".into(), Change::Unchanged);
        b.push("5".into(), Change::Ignored);
        assert_eq!((b.added.len(), b.updated.len(), b.deleted.len(), b.unchanged.len()), (1, 1, 1, 1));
        let mut c = BatchOutcome::default();
        c.extend(b);
        assert_eq!(c.added, vec!["1"]);
    }

    #[test]
    fn limits_and_filters() {
        assert_eq!(check_limit(None).unwrap(), DEFAULT_LIMIT);
        assert_eq!(check_limit(Some(1)).unwrap(), 1);
        assert_eq!(check_limit(Some(MAX_LIMIT)).unwrap(), MAX_LIMIT);
        assert!(check_limit(Some(0)).is_err());
        assert!(check_limit(Some(MAX_LIMIT + 1)).is_err());
        assert_eq!(parse_filter("added,UPDATED").unwrap(), vec![LastAction::Added, LastAction::Updated]);
        assert_eq!(parse_filter("deleted").unwrap(), vec![LastAction::Deleted]);
        assert!(parse_filter("added,nope").is_err());
        assert_eq!(LastAction::parse("Deleted"), Some(LastAction::Deleted));
        assert_eq!(serde_json::to_value(LastAction::Added).unwrap(), json!("ADDED"));
    }

    proptest! {
        #[test]
        fn any_cursor_round_trips(us in any::<i64>(), id in any::<u128>()) {
            let c = Cursor { updated_at_us: us, id: Uuid::from_u128(id) };
            prop_assert_eq!(Cursor::decode(&c.encode()).unwrap(), c);
        }
    }
}
