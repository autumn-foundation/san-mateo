-- Removes the San Mateo schema, version 1. This deletes all San Mateo data.
DROP TABLE IF EXISTS sm_sync_states;
DROP TABLE IF EXISTS sm_records;
DROP TABLE IF EXISTS sm_oauth_sessions;
DROP TABLE IF EXISTS sm_connect_sessions;
DROP TABLE IF EXISTS sm_leases;
DROP TABLE IF EXISTS sm_connections;
DROP TABLE IF EXISTS sm_integrations;
