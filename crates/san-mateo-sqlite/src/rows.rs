//! Values for SQL parameters, and the JSON parts of rows.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rusqlite::types::ToSql;
use san_mateo::model::{ConnectionKey, EndUser, IntegrationDefaults, Organization, Tags};
use san_mateo::store::{ConnectSessionRow, ConnectionRow, SyncCounts, SyncStateRow, SyncStatus};
use san_mateo::{Error, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Returns a value as JSON text.
pub fn json<T: Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|e| Error::Store(format!("serialize: {e}")))
}

/// The columns that a connection write changes, with their parameter
/// numbers in [`ConnectionDb::params`]. A write keeps `id` (`?1`),
/// `created_at` (`?14`) and `version` (`?21`).
pub const CHANGE_COLUMNS: [(&str, u8); 18] = [
    ("environment", 2),
    ("provider_config_key", 3),
    ("connection_id", 4),
    ("provider", 5),
    ("credentials", 6),
    ("credentials_expires_at", 7),
    ("connection_config", 8),
    ("metadata", 9),
    ("tags", 10),
    ("end_user", 11),
    ("end_user_id", 12),
    ("webhook_url_override", 13),
    ("updated_at", 15),
    ("last_fetched_at", 16),
    ("last_refresh_success", 17),
    ("last_refresh_failure", 18),
    ("refresh_attempts", 19),
    ("refresh_exhausted", 20),
];

/// Returns the `SET` list of a connection write. With `excluded`, the
/// values come from the row of an `INSERT ... ON CONFLICT DO UPDATE`.
/// Otherwise they come from the parameters of [`ConnectionDb::params`].
pub fn connection_change(excluded: bool) -> String {
    CHANGE_COLUMNS
        .iter()
        .map(|(column, n)| {
            if excluded {
                format!("{column} = excluded.{column}")
            } else {
                format!("{column} = ?{n}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A connection as SQL values.
pub struct ConnectionDb {
    pub id: String,
    pub environment: String,
    pub provider_config_key: String,
    pub connection_id: String,
    pub provider: String,
    pub credentials: String,
    pub credentials_expires_at: Option<i64>,
    pub connection_config: String,
    pub metadata: String,
    pub tags: String,
    pub end_user: Option<String>,
    pub end_user_id: Option<String>,
    pub webhook_url_override: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_fetched_at: Option<i64>,
    pub last_refresh_success: Option<i64>,
    pub last_refresh_failure: Option<i64>,
    pub refresh_attempts: u32,
    pub refresh_exhausted: bool,
    pub version: i64,
}

fn micros(t: Option<DateTime<Utc>>) -> Option<i64> {
    t.map(|t| t.timestamp_micros())
}

impl ConnectionDb {
    pub fn from_row(r: &ConnectionRow) -> Result<Self> {
        Ok(Self {
            id: r.id.to_string(),
            environment: r.environment.clone(),
            provider_config_key: r.provider_config_key.clone(),
            connection_id: r.connection_id.clone(),
            provider: r.provider.clone(),
            credentials: r.credentials.clone(),
            credentials_expires_at: micros(r.credentials_expires_at),
            connection_config: json(&r.connection_config)?,
            metadata: json(&r.metadata)?,
            tags: json(&r.tags)?,
            end_user: r.end_user.as_ref().map(json).transpose()?,
            end_user_id: r.end_user.as_ref().map(|u| u.id.clone()),
            webhook_url_override: r.webhook_url_override.clone(),
            created_at: r.created_at.timestamp_micros(),
            updated_at: r.updated_at.timestamp_micros(),
            last_fetched_at: micros(r.last_fetched_at),
            last_refresh_success: micros(r.last_refresh_success),
            last_refresh_failure: micros(r.last_refresh_failure),
            refresh_attempts: r.refresh_attempts,
            refresh_exhausted: r.refresh_exhausted,
            version: r.version,
        })
    }

    /// Returns the values `?1` to `?21`, in the column order of
    /// `CONNECTION_COLUMNS` in `lib.rs`.
    pub fn params(&self) -> Vec<&dyn ToSql> {
        vec![
            &self.id,
            &self.environment,
            &self.provider_config_key,
            &self.connection_id,
            &self.provider,
            &self.credentials,
            &self.credentials_expires_at,
            &self.connection_config,
            &self.metadata,
            &self.tags,
            &self.end_user,
            &self.end_user_id,
            &self.webhook_url_override,
            &self.created_at,
            &self.updated_at,
            &self.last_fetched_at,
            &self.last_refresh_success,
            &self.last_refresh_failure,
            &self.refresh_attempts,
            &self.refresh_exhausted,
            &self.version,
        ]
    }
}

/// The JSON part of a connect session row. It is the same as in the
/// Postgres store.
#[derive(Serialize, Deserialize)]
pub struct SessionData {
    end_user: Option<EndUser>,
    organization: Option<Organization>,
    allowed_integrations: Vec<String>,
    integrations_config_defaults: BTreeMap<String, IntegrationDefaults>,
    tags: Tags,
    webhook_url_override: Option<String>,
    reconnect: Option<ConnectionKey>,
}

impl SessionData {
    pub fn of(r: &ConnectSessionRow) -> Self {
        Self {
            end_user: r.end_user.clone(),
            organization: r.organization.clone(),
            allowed_integrations: r.allowed_integrations.clone(),
            integrations_config_defaults: r.integrations_config_defaults.clone(),
            tags: r.tags.clone(),
            webhook_url_override: r.webhook_url_override.clone(),
            reconnect: r.reconnect.clone(),
        }
    }

    pub fn into_row(
        self,
        id: Uuid,
        environment: String,
        token_hash: String,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> ConnectSessionRow {
        ConnectSessionRow {
            id,
            environment,
            token_hash,
            end_user: self.end_user,
            organization: self.organization,
            allowed_integrations: self.allowed_integrations,
            integrations_config_defaults: self.integrations_config_defaults,
            tags: self.tags,
            webhook_url_override: self.webhook_url_override,
            reconnect: self.reconnect,
            created_at,
            expires_at,
        }
    }
}

/// The JSON part of a sync state row. It is the same as in the Postgres
/// store.
#[derive(Serialize, Deserialize)]
pub struct SyncData {
    pub status: SyncStatus,
    pub frequency_ms: u64,
    pub paused: bool,
    pub last_started_at: Option<DateTime<Utc>>,
    pub last_finished_at: Option<DateTime<Utc>>,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub last_counts: SyncCounts,
}

impl SyncData {
    pub fn of(r: &SyncStateRow) -> Self {
        Self {
            status: r.status,
            frequency_ms: r.frequency_ms,
            paused: r.paused,
            last_started_at: r.last_started_at,
            last_finished_at: r.last_finished_at,
            last_success_at: r.last_success_at,
            last_error: r.last_error.clone(),
            last_counts: r.last_counts,
        }
    }

    pub fn into_row(self, connection: Uuid, sync_name: String) -> SyncStateRow {
        SyncStateRow {
            connection,
            sync_name,
            status: self.status,
            frequency_ms: self.frequency_ms,
            paused: self.paused,
            last_started_at: self.last_started_at,
            last_finished_at: self.last_finished_at,
            last_success_at: self.last_success_at,
            last_error: self.last_error,
            last_counts: self.last_counts,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_change_column_has_its_own_parameter() {
        let mut seen: Vec<u8> = CHANGE_COLUMNS.iter().map(|(_, n)| *n).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), CHANGE_COLUMNS.len());
        // `id`, `created_at` and `version` are not in the list.
        assert!(!seen.contains(&1) && !seen.contains(&14) && !seen.contains(&21));
        assert!(seen.iter().all(|n| (2..=20).contains(n)));
    }

    #[test]
    fn the_change_list_names_each_column_once() {
        let set = connection_change(false);
        assert!(set.starts_with("environment = ?2, "));
        assert!(set.ends_with("refresh_exhausted = ?20"));
        let ex = connection_change(true);
        assert!(ex.contains("credentials = excluded.credentials"));
        assert!(!ex.contains("created_at"));
    }
}
