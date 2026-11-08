-- San Mateo schema for SQLite, version 1. It has the same tables as the
-- Postgres schema.
-- Secrets are sealed by the service before a write (see ADR 0003).
-- Times are INTEGER microseconds since the Unix epoch, as in Postgres
-- TIMESTAMPTZ. So the order and the cursors are the same in both stores.
-- UUIDs are TEXT in the hyphenated lower-case form. Text order is then the
-- byte order that Postgres uses for UUID.
-- JSON values are TEXT.

CREATE TABLE IF NOT EXISTS sm_integrations (
    environment TEXT NOT NULL,
    unique_key TEXT NOT NULL,
    provider TEXT NOT NULL,
    display_name TEXT,
    client_id TEXT,
    client_secret TEXT,
    scopes TEXT NOT NULL DEFAULT '[]',
    custom TEXT NOT NULL DEFAULT '{}',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (environment, unique_key)
) STRICT;

CREATE TABLE IF NOT EXISTS sm_connections (
    id TEXT PRIMARY KEY NOT NULL,
    environment TEXT NOT NULL,
    provider_config_key TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    credentials TEXT NOT NULL,
    credentials_expires_at INTEGER,
    connection_config TEXT NOT NULL,
    metadata TEXT NOT NULL,
    tags TEXT NOT NULL,
    end_user TEXT,
    end_user_id TEXT,
    webhook_url_override TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_fetched_at INTEGER,
    last_refresh_success INTEGER,
    last_refresh_failure INTEGER,
    refresh_attempts INTEGER NOT NULL DEFAULT 0,
    refresh_exhausted INTEGER NOT NULL DEFAULT 0 CHECK (refresh_exhausted IN (0, 1)),
    version INTEGER NOT NULL DEFAULT 0,
    CONSTRAINT sm_connections_key UNIQUE (environment, provider_config_key, connection_id)
) STRICT;

CREATE INDEX IF NOT EXISTS sm_connections_expiry
    ON sm_connections (credentials_expires_at, id)
    WHERE refresh_exhausted = 0 AND credentials_expires_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS sm_connections_end_user
    ON sm_connections (environment, end_user_id);

CREATE TABLE IF NOT EXISTS sm_leases (
    name TEXT PRIMARY KEY NOT NULL,
    owner TEXT NOT NULL,
    until INTEGER NOT NULL
) STRICT;

CREATE TABLE IF NOT EXISTS sm_connect_sessions (
    id TEXT PRIMARY KEY NOT NULL,
    environment TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    data TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
) STRICT;

CREATE INDEX IF NOT EXISTS sm_connect_sessions_expiry ON sm_connect_sessions (expires_at);

CREATE TABLE IF NOT EXISTS sm_oauth_sessions (
    state_hash TEXT PRIMARY KEY NOT NULL,
    environment TEXT NOT NULL,
    provider_config_key TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    connect_session_id TEXT,
    code_verifier TEXT,
    connection_config TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL
) STRICT;

CREATE INDEX IF NOT EXISTS sm_oauth_sessions_expiry ON sm_oauth_sessions (expires_at);

-- Records and sync states go with their connection. SQLite checks these
-- keys only when a connection sets `PRAGMA foreign_keys = ON`. The store
-- sets it on each connection that it opens.
CREATE TABLE IF NOT EXISTS sm_records (
    connection TEXT NOT NULL REFERENCES sm_connections (id) ON DELETE CASCADE,
    model TEXT NOT NULL,
    id TEXT NOT NULL,
    external_id TEXT NOT NULL,
    data TEXT NOT NULL,
    data_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    PRIMARY KEY (connection, model, id)
) STRICT;

CREATE INDEX IF NOT EXISTS sm_records_cursor ON sm_records (connection, model, updated_at, id);

CREATE TABLE IF NOT EXISTS sm_sync_states (
    connection TEXT NOT NULL REFERENCES sm_connections (id) ON DELETE CASCADE,
    sync_name TEXT NOT NULL,
    data TEXT NOT NULL,
    PRIMARY KEY (connection, sync_name)
) STRICT;
