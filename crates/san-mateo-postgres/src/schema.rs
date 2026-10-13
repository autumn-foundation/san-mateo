//! Diesel table definitions. They match `migrations/`.

diesel::table! {
    sm_schema_migrations (version) {
        version -> Text,
    }
}

diesel::table! {
    sm_integrations (environment, unique_key) {
        environment -> Text,
        unique_key -> Text,
        provider -> Text,
        display_name -> Nullable<Text>,
        client_id -> Nullable<Text>,
        client_secret -> Nullable<Text>,
        scopes -> Jsonb,
        custom -> Jsonb,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

diesel::table! {
    sm_connections (id) {
        id -> Uuid,
        environment -> Text,
        provider_config_key -> Text,
        connection_id -> Text,
        provider -> Text,
        credentials -> Text,
        credentials_expires_at -> Nullable<Timestamptz>,
        connection_config -> Jsonb,
        metadata -> Jsonb,
        tags -> Jsonb,
        end_user -> Nullable<Jsonb>,
        end_user_id -> Nullable<Text>,
        webhook_url_override -> Nullable<Text>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        last_fetched_at -> Nullable<Timestamptz>,
        last_refresh_success -> Nullable<Timestamptz>,
        last_refresh_failure -> Nullable<Timestamptz>,
        refresh_attempts -> Int4,
        refresh_exhausted -> Bool,
        version -> Int8,
    }
}

diesel::table! {
    sm_leases (name) {
        name -> Text,
        owner -> Text,
        until -> Timestamptz,
    }
}

diesel::table! {
    sm_connect_sessions (id) {
        id -> Uuid,
        environment -> Text,
        token_hash -> Text,
        data -> Jsonb,
        created_at -> Timestamptz,
        expires_at -> Timestamptz,
    }
}

diesel::table! {
    sm_oauth_sessions (state_hash) {
        state_hash -> Text,
        environment -> Text,
        provider_config_key -> Text,
        connection_id -> Text,
        connect_session_id -> Nullable<Uuid>,
        code_verifier -> Nullable<Text>,
        connection_config -> Jsonb,
        redirect_uri -> Text,
        created_at -> Timestamptz,
        expires_at -> Timestamptz,
    }
}

diesel::table! {
    sm_records (connection, model, id) {
        connection -> Uuid,
        model -> Text,
        id -> Uuid,
        external_id -> Text,
        data -> Text,
        data_hash -> Text,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
        deleted_at -> Nullable<Timestamptz>,
    }
}

diesel::table! {
    sm_sync_states (connection, sync_name) {
        connection -> Uuid,
        sync_name -> Text,
        data -> Jsonb,
    }
}

// Foreign keys: `ON DELETE CASCADE` from each table to `sm_connections`.
diesel::joinable!(sm_records -> sm_connections (connection));
diesel::joinable!(sm_sync_states -> sm_connections (connection));
