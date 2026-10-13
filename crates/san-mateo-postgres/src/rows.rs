//! Database rows and their conversion to and from the store types.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use diesel::prelude::*;
use san_mateo::model::{ConnectionKey, EndUser, IntegrationDefaults, Organization, Tags};
use san_mateo::store::{
    ConnectSessionRow, ConnectionRow, IntegrationRow, OAuthSessionRow, RecordRow, SyncCounts,
    SyncStateRow, SyncStatus,
};
use san_mateo::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::schema::{
    sm_connect_sessions, sm_connections, sm_integrations, sm_oauth_sessions, sm_records,
};

fn to_json<T: Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| Error::Store(format!("serialize: {e}")))
}

fn from_json<T: serde::de::DeserializeOwned>(v: Value) -> Result<T> {
    serde_json::from_value(v)
        .map_err(|e| Error::Store(format!("a stored JSON column is not valid: {e}")))
}

#[derive(Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = sm_integrations, check_for_backend(diesel::pg::Pg), treat_none_as_null = true)]
pub struct IntegrationDb {
    pub environment: String,
    pub unique_key: String,
    pub provider: String,
    pub display_name: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scopes: Value,
    pub custom: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl IntegrationDb {
    pub fn from_row(r: &IntegrationRow) -> Result<Self> {
        Ok(Self {
            environment: r.environment.clone(),
            unique_key: r.unique_key.clone(),
            provider: r.provider.clone(),
            display_name: r.display_name.clone(),
            client_id: r.client_id.clone(),
            client_secret: r.client_secret.clone(),
            scopes: to_json(&r.scopes)?,
            custom: to_json(&r.custom)?,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
    }

    pub fn into_row(self) -> Result<IntegrationRow> {
        Ok(IntegrationRow {
            environment: self.environment,
            unique_key: self.unique_key,
            provider: self.provider,
            display_name: self.display_name,
            client_id: self.client_id,
            client_secret: self.client_secret,
            scopes: from_json(self.scopes)?,
            custom: from_json::<BTreeMap<String, String>>(self.custom)?,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

#[derive(Queryable, Selectable, Insertable, AsChangeset)]
#[diesel(table_name = sm_connections, check_for_backend(diesel::pg::Pg), treat_none_as_null = true)]
pub struct ConnectionDb {
    pub id: Uuid,
    pub environment: String,
    pub provider_config_key: String,
    pub connection_id: String,
    pub provider: String,
    pub credentials: String,
    pub credentials_expires_at: Option<DateTime<Utc>>,
    pub connection_config: Value,
    pub metadata: Value,
    pub tags: Value,
    pub end_user: Option<Value>,
    pub end_user_id: Option<String>,
    pub webhook_url_override: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_fetched_at: Option<DateTime<Utc>>,
    pub last_refresh_success: Option<DateTime<Utc>>,
    pub last_refresh_failure: Option<DateTime<Utc>>,
    pub refresh_attempts: i32,
    pub refresh_exhausted: bool,
    pub version: i64,
}

/// The columns that a write changes. `id`, `created_at` and `version` stay.
#[derive(AsChangeset)]
#[diesel(table_name = sm_connections, treat_none_as_null = true)]
pub struct ConnectionChange {
    pub environment: String,
    pub provider_config_key: String,
    pub connection_id: String,
    pub provider: String,
    pub credentials: String,
    pub credentials_expires_at: Option<DateTime<Utc>>,
    pub connection_config: Value,
    pub metadata: Value,
    pub tags: Value,
    pub end_user: Option<Value>,
    pub end_user_id: Option<String>,
    pub webhook_url_override: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub last_fetched_at: Option<DateTime<Utc>>,
    pub last_refresh_success: Option<DateTime<Utc>>,
    pub last_refresh_failure: Option<DateTime<Utc>>,
    pub refresh_attempts: i32,
    pub refresh_exhausted: bool,
}

impl ConnectionDb {
    pub fn from_row(r: &ConnectionRow) -> Result<Self> {
        Ok(Self {
            id: r.id,
            environment: r.environment.clone(),
            provider_config_key: r.provider_config_key.clone(),
            connection_id: r.connection_id.clone(),
            provider: r.provider.clone(),
            credentials: r.credentials.clone(),
            credentials_expires_at: r.credentials_expires_at,
            connection_config: r.connection_config.clone(),
            metadata: r.metadata.clone(),
            tags: to_json(&r.tags)?,
            end_user: r.end_user.as_ref().map(to_json).transpose()?,
            end_user_id: r.end_user.as_ref().map(|u| u.id.clone()),
            webhook_url_override: r.webhook_url_override.clone(),
            created_at: r.created_at,
            updated_at: r.updated_at,
            last_fetched_at: r.last_fetched_at,
            last_refresh_success: r.last_refresh_success,
            last_refresh_failure: r.last_refresh_failure,
            refresh_attempts: i32::try_from(r.refresh_attempts).unwrap_or(i32::MAX),
            refresh_exhausted: r.refresh_exhausted,
            version: r.version,
        })
    }

    pub fn change(&self) -> ConnectionChange {
        ConnectionChange {
            environment: self.environment.clone(),
            provider_config_key: self.provider_config_key.clone(),
            connection_id: self.connection_id.clone(),
            provider: self.provider.clone(),
            credentials: self.credentials.clone(),
            credentials_expires_at: self.credentials_expires_at,
            connection_config: self.connection_config.clone(),
            metadata: self.metadata.clone(),
            tags: self.tags.clone(),
            end_user: self.end_user.clone(),
            end_user_id: self.end_user_id.clone(),
            webhook_url_override: self.webhook_url_override.clone(),
            updated_at: self.updated_at,
            last_fetched_at: self.last_fetched_at,
            last_refresh_success: self.last_refresh_success,
            last_refresh_failure: self.last_refresh_failure,
            refresh_attempts: self.refresh_attempts,
            refresh_exhausted: self.refresh_exhausted,
        }
    }

    pub fn into_row(self) -> Result<ConnectionRow> {
        Ok(ConnectionRow {
            id: self.id,
            environment: self.environment,
            provider_config_key: self.provider_config_key,
            connection_id: self.connection_id,
            provider: self.provider,
            credentials: self.credentials,
            credentials_expires_at: self.credentials_expires_at,
            connection_config: self.connection_config,
            metadata: self.metadata,
            tags: from_json::<Tags>(self.tags)?,
            end_user: self.end_user.map(from_json::<EndUser>).transpose()?,
            webhook_url_override: self.webhook_url_override,
            created_at: self.created_at,
            updated_at: self.updated_at,
            last_fetched_at: self.last_fetched_at,
            last_refresh_success: self.last_refresh_success,
            last_refresh_failure: self.last_refresh_failure,
            refresh_attempts: u32::try_from(self.refresh_attempts).unwrap_or(0),
            refresh_exhausted: self.refresh_exhausted,
            version: self.version,
        })
    }
}

/// The JSON part of a connect session row.
#[derive(Serialize, Deserialize)]
struct SessionData {
    end_user: Option<EndUser>,
    organization: Option<Organization>,
    allowed_integrations: Vec<String>,
    integrations_config_defaults: BTreeMap<String, IntegrationDefaults>,
    tags: Tags,
    webhook_url_override: Option<String>,
    reconnect: Option<ConnectionKey>,
}

#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = sm_connect_sessions, check_for_backend(diesel::pg::Pg))]
pub struct SessionDb {
    pub id: Uuid,
    pub environment: String,
    pub token_hash: String,
    pub data: Value,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl SessionDb {
    pub fn from_row(r: &ConnectSessionRow) -> Result<Self> {
        Ok(Self {
            id: r.id,
            environment: r.environment.clone(),
            token_hash: r.token_hash.clone(),
            data: to_json(&SessionData {
                end_user: r.end_user.clone(),
                organization: r.organization.clone(),
                allowed_integrations: r.allowed_integrations.clone(),
                integrations_config_defaults: r.integrations_config_defaults.clone(),
                tags: r.tags.clone(),
                webhook_url_override: r.webhook_url_override.clone(),
                reconnect: r.reconnect.clone(),
            })?,
            created_at: r.created_at,
            expires_at: r.expires_at,
        })
    }

    pub fn into_row(self) -> Result<ConnectSessionRow> {
        let d: SessionData = from_json(self.data)?;
        Ok(ConnectSessionRow {
            id: self.id,
            environment: self.environment,
            token_hash: self.token_hash,
            end_user: d.end_user,
            organization: d.organization,
            allowed_integrations: d.allowed_integrations,
            integrations_config_defaults: d.integrations_config_defaults,
            tags: d.tags,
            webhook_url_override: d.webhook_url_override,
            reconnect: d.reconnect,
            created_at: self.created_at,
            expires_at: self.expires_at,
        })
    }
}

#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = sm_oauth_sessions, check_for_backend(diesel::pg::Pg))]
pub struct OAuthDb {
    pub state_hash: String,
    pub environment: String,
    pub provider_config_key: String,
    pub connection_id: String,
    pub connect_session_id: Option<Uuid>,
    pub code_verifier: Option<String>,
    pub connection_config: Value,
    pub redirect_uri: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl OAuthDb {
    pub fn from_row(r: OAuthSessionRow) -> Self {
        Self {
            state_hash: r.state_hash,
            environment: r.environment,
            provider_config_key: r.provider_config_key,
            connection_id: r.connection_id,
            connect_session_id: r.connect_session_id,
            code_verifier: r.code_verifier,
            connection_config: r.connection_config,
            redirect_uri: r.redirect_uri,
            created_at: r.created_at,
            expires_at: r.expires_at,
        }
    }

    pub fn into_row(self) -> OAuthSessionRow {
        OAuthSessionRow {
            state_hash: self.state_hash,
            environment: self.environment,
            provider_config_key: self.provider_config_key,
            connection_id: self.connection_id,
            connect_session_id: self.connect_session_id,
            code_verifier: self.code_verifier,
            connection_config: self.connection_config,
            redirect_uri: self.redirect_uri,
            created_at: self.created_at,
            expires_at: self.expires_at,
        }
    }
}

#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = sm_records, check_for_backend(diesel::pg::Pg))]
pub struct RecordDb {
    pub connection: Uuid,
    pub model: String,
    pub id: Uuid,
    pub external_id: String,
    pub data: String,
    pub data_hash: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

impl RecordDb {
    pub fn into_row(self) -> RecordRow {
        RecordRow {
            id: self.id,
            external_id: self.external_id,
            data: self.data,
            data_hash: self.data_hash,
            created_at: self.created_at,
            updated_at: self.updated_at,
            deleted_at: self.deleted_at,
        }
    }
}

/// The JSON part of a sync state row.
#[derive(Serialize, Deserialize)]
struct SyncData {
    status: SyncStatus,
    frequency_ms: u64,
    paused: bool,
    last_started_at: Option<DateTime<Utc>>,
    last_finished_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
    last_counts: SyncCounts,
}

pub fn sync_data(r: &SyncStateRow) -> Result<Value> {
    to_json(&SyncData {
        status: r.status,
        frequency_ms: r.frequency_ms,
        paused: r.paused,
        last_started_at: r.last_started_at,
        last_finished_at: r.last_finished_at,
        last_success_at: r.last_success_at,
        last_error: r.last_error.clone(),
        last_counts: r.last_counts,
    })
}

pub fn sync_row(connection: Uuid, sync_name: String, data: Value) -> Result<SyncStateRow> {
    let d: SyncData = from_json(data)?;
    Ok(SyncStateRow {
        connection,
        sync_name,
        status: d.status,
        frequency_ms: d.frequency_ms,
        paused: d.paused,
        last_started_at: d.last_started_at,
        last_finished_at: d.last_finished_at,
        last_success_at: d.last_success_at,
        last_error: d.last_error,
        last_counts: d.last_counts,
    })
}
