# Acceptance criteria: evidence

This file lists each acceptance criterion (AC) of [plan.md](plan.md), its
status and its evidence. It also records the TDD steps, the review rounds
and the mutation tests.

Paths: `core/` is `crates/san-mateo`, `pg/` is `crates/san-mateo-postgres`,
`server/` is `crates/san-mateo-server`, `plugin/` is
`crates/autumn-plugin-san-mateo`, `harvest/` is `crates/san-mateo-harvest`,
`camel/` is `crates/san-mateo-camel`. `t/` is a `tests/` folder. `u:` is a
unit test in `src/`.

## Summary

| Item | Result |
|---|---|
| ACs done | 26 of 26 (AC25 with the limits in its row) |
| Tests | {{TESTS}} |
| Line coverage | {{COVERAGE}} |
| Verus | `verus/policy.rs`: 29 verified, 0 errors |
| CI | fmt, clippy `-D warnings`, docs `-D warnings`, tests with Postgres 16, coverage ≥ 85%, MSRV 1.89, Verus. Green on the PR head. |

## Acceptance criteria

| AC | Status | Evidence |
|---|---|---|
| AC1 Catalog | Done | u: `provider::{reads_fields, alias_is_a_shallow_merge, bad_catalogs, builtin_catalog_covers_each_supported_mode, reads_the_full_nango_catalog_when_present}`. CI gets the Nango `providers.yaml` at a fixed commit and runs the last test on it (more than 1,000 providers, 80% or more with a supported mode). `core/t/api.rs::providers`. Built-in catalog: `core/providers/providers.yaml`, 36 providers. |
| AC2 Templates | Done | u: `template::{plain_values, missing_values_fail, fallback_uses_the_first_complete_alternative, functions, url_positions, host_injection_is_rejected, finds_values_that_set_the_host}`. `core/t/proxy.rs::{interpolated_base_url, optional_template_values_are_left_out}`. |
| AC3 Integrations | Done | `core/t/auth_flows.rs::integrations_crud_hides_and_seals_the_secret`, `core/t/api.rs::integrations_over_http`. |
| AC4 Connect sessions | Done | `core/t/auth_flows.rs::connect_sessions_store_only_a_hash_and_expire`, `core/t/api.rs::connect_sessions_over_http`, `core/t/hardening.rs::webhook_overrides_cannot_point_to_internal_hosts`. |
| AC5 OAuth2 | Done | `core/t/auth_flows.rs::{oauth2_flow_end_to_end, oauth_errors_are_reported, interpolated_provider_with_basic_client_auth, session_defaults_replace_end_user_values}`. `core/t/hardening.rs::{end_users_cannot_set_reserved_oauth_parameters, scopes_follow_nango_rules, the_oauth_state_ends_with_the_connect_session, callback_metadata_goes_into_the_token_url_after_a_check}`. u: `oauth2::{pkce_rfc7636_vector, token_request_json_and_basic_auth, parse_quirky_replies, parse_alternate_path, zero_expiry_means_no_expiry, header_authorization_method_uses_basic_auth}`. `core/t/api.rs::oauth_over_http`. |
| AC6 Other auth modes | Done | `core/t/auth_flows.rs::{api_key_connections_are_verified, basic_cc_and_unauthenticated_connections}`, `core/t/hardening.rs::{verification_endpoints_are_templates_and_send_data, a_client_credentials_refresh_asks_for_the_same_scopes}`, `core/t/api.rs::other_auth_modes_over_http`. |
| AC7 Encryption | Done | u: `crypto::{wrong_aad_fails, tampered_value_fails, nonces_differ, rotation_keeps_old_data_readable, debug_has_no_key_bytes}`. `core/t/auth_flows.rs::{credentials_are_sealed_and_bound_to_the_row, key_rotation_keeps_connections_readable}`. Logs: `core/t/logs.rs::no_secret_reaches_the_logs`. `Debug`: `server/t/server.rs::full_settings`, u: `secret::debug_hides_the_value`. |
| AC8 Connections API | Done | `core/t/connections.rs::{get_returns_fresh_credentials_without_a_refresh, force_refresh_and_no_refresh_token, list_delete_metadata_and_patch}`, `core/t/api.rs::{connections_over_http, get_connection_hides_refresh_tokens}`, `core/t/auth_flows.rs::import_checks_the_auth_mode`. |
| AC9 Refresh | Done | u: `policy::{expiry_edges, gate_rules, failure_counts_once_per_day, constants_match_the_plan, expiry_at_a_real_time}`. `core/t/connections.rs::{refresh_inside_the_buffer_keeps_the_old_refresh_token, refresh_failures_cool_down_and_exhaust, concurrent_gets_share_one_refresh, a_lease_of_another_process_is_respected, the_sweep_refreshes_tokens_that_expire_soon}`. `core/t/refresh_races.rs` (14 tests: lease life, fenced writes, lost callers, store errors, sweep paging, short tokens, coalesced forced and 401 refreshes, valid token kept on failure). |
| AC10 Proxy | Done | `core/t/proxy.rs::{oauth_proxy_adds_auth_and_provider_headers, api_key_and_basic_proxies, a_401_refreshes_once_and_retries, a_refresh_send_counts_toward_the_hard_limit, oauth_templates_see_the_integration_client}`. u: `proxy::header_rules`. `core/t/transport_http.rs::{never_follows_redirects, caps_the_body_and_times_out}`. |
| AC11 Retry policy | Done | u: `policy::{decide_examples, decide_invariants, backoff_matches_closed_form}`, u: `proxy::{classes, waits_from_headers}`. `core/t/proxy.rs::{rate_limits_retry_for_all_methods, server_errors_retry_only_idempotent_calls_or_opt_in, network_errors_retry_then_fail, attempts_have_a_hard_limit, a_5xx_with_no_quota_left_is_not_a_rate_limit}`. Verus proves `decide`. |
| AC12 SSRF | Done | u: `proxy::{ssrf_rules, override_host_rules}`, u: `template::finds_values_that_set_the_host`. `core/t/proxy.rs::{base_url_override_is_checked, link_pagination_stays_on_known_hosts}`. `core/t/hardening.rs::{an_end_user_cannot_pick_the_host_of_a_provider_url, an_allowed_host_can_come_from_the_end_user, callback_metadata_goes_into_the_token_url_after_a_check}`. |
| AC13 Pagination | Done | u: `paginate::{cursor_flow, link_header_flow, offset_flow, link_header_keeps_commas_and_semicolons_in_targets, a_repeated_cursor_ends_the_loop}`. `core/t/proxy.rs::{cursor_pagination_reads_all_pages, pagination_has_a_page_limit_and_checks_replies, a_repeated_cursor_stops_pagination}`. |
| AC14 Records | Done | u: `records::{upsert_classes, ids_are_stable_and_distinct, limits_and_filters}`. `core/t/records.rs::{save_classify_and_read_back, records_are_sealed_at_rest, cursor_pages_and_filters, bad_queries_and_inputs}`. `core/t/api.rs::records_over_http`. Conformance: `record_times_only_go_up`. `pg/t/postgres.rs::{concurrent_record_batches_count_each_record_once, concurrent_batches_get_distinct_times, opposite_order_batches_do_not_deadlock}`. |
| AC15 Syncs and actions | Done | `core/t/functions.rs::{a_sync_run_saves_records_and_reports, one_run_at_a_time_for_each_connection_and_sync, the_scheduler_runs_due_auto_start_syncs, actions_run_and_return_output, a_slow_sync_does_not_stall_the_background_loops, last_sync_date_is_the_start_of_the_last_successful_run, two_processes_do_not_run_one_sync_at_once}`. u: `policy::sync_frequency_has_a_floor`. `core/t/api.rs::syncs_and_actions_over_http`. |
| AC16 Webhooks | Done | u: `webhook::{auth_json_shape, signatures, delivers_signed_to_both_urls, override_replaces_both_urls, retries_5xx_and_network_errors_but_not_4xx}` {{WEBHOOK_EXTRA}}. |
| AC17 API auth and errors | Done | `core/t/auth_flows.rs::secret_keys_select_the_environment`, `core/t/api.rs::health_needs_no_auth_and_other_routes_do`, u: `error::codes_and_status`, u: `crypto::constant_time_compare`, `server/t/server.rs::errors_name_the_variable`. |
| AC18 Postgres store | Done | `core` u: `store::memory::memory_store_passes_the_conformance_suite`. `pg/t/postgres.rs::{pg_store_passes_the_conformance_suite, migrations_are_idempotent_and_concurrent_safe, only_one_concurrent_writer_wins_a_version, only_one_concurrent_owner_gets_a_lease, concurrent_upserts_of_one_key_make_one_row, the_service_runs_on_postgres, the_schema_cascades_connection_deletes}`. CI runs them on Postgres 16. |
| AC19 Standalone server | Done | `server/t/server.rs::{defaults, full_settings, errors_name_the_variable, the_app_serves_the_api, a_providers_file_adds_to_the_catalog, build_uses_postgres_when_set, build_uses_old_keys_and_the_lookup_key, serve_stops_cleanly_on_shutdown, run_starts_and_stops, main_exit_codes}`, `server/t/signal.rs::sigterm_ends_the_shutdown_future`. The background loops: `core/t/connections.rs::background_loops_start_and_stop`, `core/t/functions.rs::stop_and_flush_do_not_wait_for_ever`. |
| AC20 autumn plugin | Done | `plugin/t/install.rs::{plugin_installs_with_one_call, plugin_name_contract_and_config_section, plugin_declares_all_routes_under_the_prefix, plugin_passes_the_autumn_conformance_checks, connect_and_proxy_through_the_autumn_stack, the_api_uses_the_autumn_body_limit}`, `plugin/t/lifecycle.rs` (9), `plugin/t/security.rs` (5), `plugin/t/boot.rs` (real app in a child process). u: `plugin::{loops_start_at_startup_not_in_the_state_initializer, csrf_at_the_root_names_each_route_prefix, contract_accepts_the_compiled_autumn_web}`. |
| AC21 harvest | Done | `harvest/t/activities.rs::{proxy_retryable_statuses_are_retryable_failures, proxy_client_errors_are_not_retryable_and_hold_a_body_preview, proxy_writes_are_not_retried_after_a_possible_success, actions_are_not_retried, a_paused_sync_is_skipped}`. `harvest/t/workflows.rs` (15, with harvest `WorkflowTestEnv`, `WorkflowSimulator` and replay checks). |
| AC22 camel | Done | `camel/t/route.rs` (18, real `CamelContext` route): `a_get_adds_the_auth_and_returns_json, headers_cannot_change_the_tenant_unless_the_route_allows_it, a_non_2xx_reply_fails_the_exchange_with_the_http_error, a_401_refreshes_the_token_and_sends_again`. u: the error map equals the harvest-camel retry rule for each code. |
| AC23 Connect page | Done | `core/t/api.rs::the_connect_page_lists_integrations_and_escapes_text`, u: `api::html::escapes_markup`, `core/t/hardening.rs::{the_connect_page_never_puts_secrets_in_a_url, the_error_page_hides_internal_details}`. |
| AC24 Quality | Done | CI jobs `fmt`, `clippy` (pedantic and nursery, `-D warnings`, also `cargo doc -D warnings`), `test` (Postgres, `cargo llvm-cov --fail-under-lines 85`), `msrv`. Workspace lints: `unsafe_code = "forbid"`, `unwrap_used` and `expect_used` (tests allowed by `clippy.toml`). Proptests: `policy::{decide_invariants, backoff_matches_closed_form, expired_is_monotonic_in_time}`. |
| AC25 Verus | Done, with limits | CI job `verus`. `policy::verus_bodies_match` compares 9 function bodies and all shared constants. `policy::the_drift_check_sees_changes`. Limits: `backoff_ms` has proof code in its loop, so a proptest links it to the spec. The service code that calls the policy (for example the refresh order) is tested, not proved. |
| AC26 Docs | Done | `README.md` (Mermaid flow and sequence diagrams), `CLAUDE.md`, `docs/plan.md`, `docs/adr/0001`–`0005`, crate docs. A review pass checked ASD-STE100 wording and accuracy (commit `0d9ef52`). |

## TDD record

Each crate and each review fix has a RED commit (failing tests), a GREEN
commit (tests pass) and, when needed, a REFACTOR commit.

| Step | RED | GREEN | REFACTOR |
|---|---|---|---|
| Core | `2e34711` | `3979988` | `3b49d83` |
| Postgres | `52a51eb` | `57565fc` | `d98c3fd` |
| Server | `c0bd454` | `7a328c0` | `50ed483` |
| Camel | `729f40c` | `83047b9` | `c51cbc5` |
| Harvest | `941a7f9` | `32d8810` | `5cae46d` |
| autumn plugin | `d4da2c3` | `85120d3` | `0646cab` |
| Connect flow hardening (security, OAuth) | `516b5b9` | `a9415d6` | |
| IP forms, lookup key, camel headers | `74bfa8a` | `fdf31c7` | |
| OAuth and provider protocol | `b0a0782`, `cd6687e` | `cbf4583`, `3995630` | |
| Refresh races | `27a962a` | `35b4843` | `8b78d45` |
| Sync scheduler | `ebf3faf` | `3fe6e0d` | |
| Records | `eba0043` | `30fb7f1` | |
| Adapters (ecosystem review) | `16f89c9` | `fd4ad8a` | |
| Client vars, CC scopes | `4552902` | `011aaf2` | |
| Proxy quota and attempt bound (mutation review) | `d1dbcdb` | `94e7170` | |

## Review rounds

Agents reviewed the code from six angles. Each finding was checked in the
code. The fixes use the TDD steps above.

| Angle | Findings | Result |
|---|---|---|
| Security | 3 medium, 7 low | All fixed: host-setting end-user values, camel header overrides, gated 401 refresh, session expiry in the callback, HTML error masking, reserved OAuth params, webhook override SSRF, IP forms, lookup key on rotation, CSP `form-action` and secret forms. |
| OAuth and protocol | 1 high, 9 medium, 5 low | All fixed: coalesced 401 refresh, refresh tokens hidden, Nango scope rules, CC refresh scopes, callback metadata order, `authorization_method: header`, optional template values and client vars, verification templates and data, zero expiry, Link header commas, `error: null`, decimal `expires_in`, `ids`, `DELETE` reply; also query name case. |
| Concurrency and refresh | 3 high, 7 medium, 4 low | All fixed: lease life, fenced refresh write, tracked refresh task, `touch_connection`, paged sweep, short tokens, coalesced forced refresh, slow syncs, `last_sync_date`, record write times, sync lease, deadlock order, pause write, valid token kept, cascade deletes, cancel-safe callback. |
| Ecosystem | 1 high, 4 medium, 4 low | All fixed: harvest write retries, plugin body limit, loops at startup, harvest pause, harvest-camel note, root CSRF fix, `server_url` warning, token-like headers, empty sync key. |
| Docs and STE | 9 accuracy defects | Fixed in `0d9ef52` and later doc commits. |
| Tests and spec (mutation) | 2 bugs, spec gaps, test gaps | Bugs fixed (5xx quota rule, 11th send). Verus gate spec made exact, sequence lemma added, drift test checks constants. Missing tests added (see below). |

## Mutation tests

`cargo mutants` 27.1 on the core crate (files with the main rules):

| File | Mutants | Caught | Missed | Timeout | Unviable |
|---|---|---|---|---|---|
| policy.rs (with `--skip verus_bodies_match`) | 83 | 73 | 6 | 1 | 3 |
| crypto.rs | 46 | 37 | 2 | 0 | 7 |
| template.rs | 134 | 104 | 19 | 7 | 4 |
| oauth2.rs | 82 | 74 | 5 | 0 | 3 |
| proxy.rs | 168 | 144 | 20 | 0 | 4 |
| records.rs | 52 | 39 | 3 | 0 | 10 |
| webhook.rs | 33 | 27 | 2 | 2 | 2 |

After the run, new tests pin the missed mutants that change behaviour:
{{MUTATION_FIXES}}

The other missed mutants give the same result (for example a bound on both
sides of an equal value), change only speed, or touch `Debug` and log text.

## Known limits

- The SSRF check reads IP literals. It does not resolve DNS names. Use a
  network policy for names that resolve to private addresses.
- The host check compares the last two host labels. A catalog URL such as
  `https://${x}.co.uk` passes. Catalog URLs must use a full domain.
- harvest-camel 0.1 uses camel 0.55. Its contexts cannot hold the camel
  component yet. The error map follows its retry rules.
- A crashed process holds the refresh lease of its connections for up to
  120 s. Reads with a valid token do not wait for it.
