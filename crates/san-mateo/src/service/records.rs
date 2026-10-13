//! Records: batch save, batch delete and cursor reads.

use chrono::{DateTime, SecondsFormat, Utc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{RecordPage, RecordsRequest, SanMateo, invalid};
use crate::error::Result;
use crate::model::ConnectionKey;
use crate::records::{self, BatchOutcome, Cursor, RecordQuery};
use crate::store::{RecordRow, RecordWrite};

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn record_aad(connection: Uuid, model: &str, id: Uuid) -> Vec<u8> {
    crate::crypto::aad(&["record", &connection.to_string(), model, &id.to_string()])
}

fn check_model(model: &str) -> Result<()> {
    if model.is_empty() || model.len() > 255 {
        return Err(invalid("the model name must be 1 to 255 chars"));
    }
    Ok(())
}

/// Returns the records with one entry for each external id. A later record
/// replaces an earlier one. The order of first use stays.
fn dedupe(records: Vec<Value>) -> Result<Vec<(String, Value)>> {
    let mut out: Vec<(String, Value)> = Vec::with_capacity(records.len());
    for r in records {
        let id = records::external_id(&r)?;
        match out.iter_mut().find(|(e, _)| *e == id) {
            Some(slot) => slot.1 = r,
            None => out.push((id, r)),
        }
    }
    Ok(out)
}

impl SanMateo {
    /// Saves records. Each record must have an `id`. A later record with the
    /// same `id` in one batch replaces an earlier one.
    ///
    /// # Errors
    ///
    /// [`crate::Error::UnknownConnection`], [`crate::Error::Invalid`].
    pub async fn save_records(
        &self,
        env: &str,
        key: &ConnectionKey,
        model: &str,
        records: Vec<Value>,
    ) -> Result<BatchOutcome> {
        check_model(model)?;
        let conn = self.connection_row(env, key).await?;
        let mut writes = Vec::new();
        for (external_id, record) in dedupe(records)? {
            let id = records::record_id(conn.id, model, &external_id);
            writes.push(RecordWrite {
                data_hash: records::data_hash(&self.inner.keys, &record),
                data: self
                    .inner
                    .keys
                    .seal_json(&record, &record_aad(conn.id, model, id))?,
                id,
                external_id,
            });
        }
        let changes = self
            .inner
            .store
            .upsert_records(conn.id, model, writes, self.now())
            .await?;
        let mut out = BatchOutcome::default();
        for (id, change) in changes {
            out.push(id, change);
        }
        Ok(out)
    }

    /// Marks records as deleted.
    ///
    /// # Errors
    ///
    /// [`crate::Error::UnknownConnection`], [`crate::Error::Invalid`].
    pub async fn delete_records(
        &self,
        env: &str,
        key: &ConnectionKey,
        model: &str,
        records: Vec<Value>,
    ) -> Result<BatchOutcome> {
        check_model(model)?;
        let conn = self.connection_row(env, key).await?;
        let ids = dedupe(records)?
            .into_iter()
            .map(|(external, _)| (records::record_id(conn.id, model, &external), external))
            .collect();
        let changes = self
            .inner
            .store
            .delete_records(conn.id, model, ids, self.now())
            .await?;
        let mut out = BatchOutcome::default();
        for (id, change) in changes {
            out.push(id, change);
        }
        Ok(out)
    }

    fn record_json(&self, connection: Uuid, model: &str, row: &RecordRow) -> Result<Value> {
        let mut data: Value = self
            .inner
            .keys
            .open_json(&row.data, &record_aad(connection, model, row.id))?;
        let cursor = Cursor {
            updated_at_us: row.updated_at.timestamp_micros(),
            id: row.id,
        };
        let meta = json!({
            "first_seen_at": rfc3339(row.created_at),
            "last_modified_at": rfc3339(row.updated_at),
            "last_action": row.last_action(),
            "deleted_at": row.deleted_at.map(rfc3339),
            "pruned_at": Value::Null,
            "cursor": cursor.encode(),
        });
        match &mut data {
            Value::Object(m) => {
                m.insert("_nango_metadata".to_owned(), meta);
            }
            other => {
                *other = json!({ "id": row.external_id, "value": other.clone(), "_nango_metadata": meta });
            }
        }
        Ok(data)
    }

    /// Reads a page of records.
    ///
    /// # Errors
    ///
    /// [`crate::Error::UnknownConnection`], [`crate::Error::Invalid`].
    pub async fn list_records(
        &self,
        env: &str,
        key: &ConnectionKey,
        request: RecordsRequest,
    ) -> Result<RecordPage> {
        check_model(&request.model)?;
        let limit = records::check_limit(request.limit)?;
        if request.ids.len() > records::MAX_IDS {
            return Err(invalid(format!("at most {} ids", records::MAX_IDS)));
        }
        let cursor = request.cursor.as_deref().map(Cursor::decode).transpose()?;
        let modified_after_us = request
            .modified_after
            .as_deref()
            .map(|s| {
                DateTime::parse_from_rfc3339(s)
                    .map(|t| t.timestamp_micros())
                    .map_err(|_| invalid("modified_after must be an RFC 3339 time"))
            })
            .transpose()?;
        let filter = request
            .filter
            .as_deref()
            .map(records::parse_filter)
            .transpose()?
            .unwrap_or_default();
        let conn = self.connection_row(env, key).await?;
        let query = RecordQuery {
            model: request.model.clone(),
            cursor,
            modified_after_us,
            limit,
            filter,
            external_ids: request.ids,
        };
        let mut rows = self.inner.store.list_records(conn.id, &query).await?;
        let more = rows.len() > limit as usize;
        rows.truncate(limit as usize);
        let next_cursor = more.then(|| rows.last()).flatten().map(|r| {
            Cursor {
                updated_at_us: r.updated_at.timestamp_micros(),
                id: r.id,
            }
            .encode()
        });
        let records = rows
            .iter()
            .map(|r| self.record_json(conn.id, &request.model, r))
            .collect::<Result<Vec<_>>>()?;
        Ok(RecordPage {
            records,
            next_cursor,
        })
    }
}
