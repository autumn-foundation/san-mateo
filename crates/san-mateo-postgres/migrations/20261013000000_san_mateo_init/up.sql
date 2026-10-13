-- San Mateo schema, version 1.
-- Secrets are sealed by the service before a write (see ADR 0003).

CREATE TABLE IF NOT EXISTS sm_integrations (
    environment TEXT NOT NULL,
    unique_key TEXT NOT NULL,
    provider TEXT NOT NULL,
    display_name TEXT,
    client_id TEXT,
    client_secret TEXT,
    scopes JSONB NOT NULL DEFAULT '[]'::jsonb,
    custom JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (environment, unique_key)
);

CREATE TABLE IF NOT EXISTS sm_connections (
    id UUID PRIMARY KEY,
    environment TEXT NOT NULL,
    provider_config_key TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    credentials TEXT NOT NULL,
    credentials_expires_at TIMESTAMPTZ,
    connection_config JSONB NOT NULL,
    metadata JSONB NOT NULL,
    tags JSONB NOT NULL,
    end_user JSONB,
    end_user_id TEXT,
    webhook_url_override TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    last_fetched_at TIMESTAMPTZ,
    last_refresh_success TIMESTAMPTZ,
    last_refresh_failure TIMESTAMPTZ,
    refresh_attempts INTEGER NOT NULL DEFAULT 0,
    refresh_exhausted BOOLEAN NOT NULL DEFAULT FALSE,
    version BIGINT NOT NULL DEFAULT 0,
    CONSTRAINT sm_connections_key UNIQUE (environment, provider_config_key, connection_id)
);

CREATE INDEX IF NOT EXISTS sm_connections_expiry
    ON sm_connections (credentials_expires_at, id)
    WHERE NOT refresh_exhausted AND credentials_expires_at IS NOT NULL;

CREATE INDEX IF NOT EXISTS sm_connections_end_user
    ON sm_connections (environment, end_user_id);

CREATE TABLE IF NOT EXISTS sm_leases (
    name TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    until TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS sm_connect_sessions (
    id UUID PRIMARY KEY,
    environment TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS sm_connect_sessions_expiry ON sm_connect_sessions (expires_at);

CREATE TABLE IF NOT EXISTS sm_oauth_sessions (
    state_hash TEXT PRIMARY KEY,
    environment TEXT NOT NULL,
    provider_config_key TEXT NOT NULL,
    connection_id TEXT NOT NULL,
    connect_session_id UUID,
    code_verifier TEXT,
    connection_config JSONB NOT NULL,
    redirect_uri TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS sm_oauth_sessions_expiry ON sm_oauth_sessions (expires_at);

CREATE TABLE IF NOT EXISTS sm_records (
    connection UUID NOT NULL,
    model TEXT NOT NULL,
    id UUID NOT NULL,
    external_id TEXT NOT NULL,
    data TEXT NOT NULL,
    data_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    deleted_at TIMESTAMPTZ,
    PRIMARY KEY (connection, model, id)
);

CREATE INDEX IF NOT EXISTS sm_records_cursor ON sm_records (connection, model, updated_at, id);

CREATE TABLE IF NOT EXISTS sm_sync_states (
    connection UUID NOT NULL,
    sync_name TEXT NOT NULL,
    data JSONB NOT NULL,
    PRIMARY KEY (connection, sync_name)
);
