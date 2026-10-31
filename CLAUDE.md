# CLAUDE.md

San Mateo: a Rust integration auth platform with the Nango public API.
Style for all docs and comments: ASD-STE100.

## Layout

| Path | Contents |
|---|---|
| `crates/san-mateo/src/policy.rs` | Verified core: token expiry, refresh gate, failure count, retry decision, backoff. Pure. |
| `verus/policy.rs` | Verus spec and proofs for `policy.rs`. Make the same change in both files. |
| `crates/san-mateo/src/crypto.rs` | `KeyRing`: AES-256-GCM with row AAD, lookup HMAC. |
| `crates/san-mateo/src/provider.rs`, `providers/` | Nango-format catalog and the built-in providers. |
| `crates/san-mateo/src/transport.rs` | `HttpTransport`, `ReqwestTransport`, `MemoryTransport` (fake). |
| `crates/san-mateo/src/proxy.rs` | Upstream URL, auth headers, header rules, SSRF checks. |
| `crates/san-mateo/src/store/` | `Store` trait, `MemoryStore`, `conformance` suite. |
| `crates/san-mateo/src/service/` | `SanMateo` facade: connect, integrations, connections and refresh, proxy, records, functions. |
| `crates/san-mateo/src/api/` | axum router, connect page, OAuth result page. |
| `crates/san-mateo-postgres` | `PgStore` (diesel-async) and SQL migrations. |
| `crates/san-mateo-server` | The `san-mateo` binary. Env config. |
| `crates/autumn-plugin-san-mateo` | autumn-web 0.9 plugin. |
| `crates/san-mateo-harvest` | autumn-harvest 0.7 activities and workflows. |
| `crates/san-mateo-camel` | rust-camel 0.57 `san-mateo:` component. |
| `crates/*/examples/` | Runnable examples: `embedded`, `autumn_app`, `harvest_workflow`. |
| `examples/harvest-camel` | The `harvest_camel` example. A separate workspace (ADR 0006). |
| `scripts/smoke-autumn-example.sh` | Boots the `autumn_app` binary and checks it. CI runs it. |
| `docs/` | `plan.md`, `examples-plan.md`, `ac-evidence.md`, ADRs. |

## Commands

```bash
git config core.hooksPath .githooks   # pre-commit: fmt, clippy, test
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
SAN_MATEO_TEST_DATABASE_URL=postgres://sm:sm@localhost/sm_test cargo test --workspace
cargo llvm-cov --workspace --summary-only
verus verus/policy.rs
cargo run --locked -p san-mateo --example embedded
bash scripts/smoke-autumn-example.sh
DATABASE_URL=postgres://sm:sm@localhost/sm_test cargo run --locked -p san-mateo-harvest --example harvest_workflow
DATABASE_URL=postgres://sm:sm@localhost/sm_test cargo run --locked --manifest-path examples/harvest-camel/Cargo.toml
# The separate workspace: the root commands do not see it.
cargo fmt --manifest-path examples/harvest-camel/Cargo.toml --check
cargo clippy --locked --manifest-path examples/harvest-camel/Cargo.toml --all-targets -- -D warnings
SAN_MATEO_TEST_DATABASE_URL=postgres://sm:sm@localhost/sm_test cargo test --locked --manifest-path examples/harvest-camel/Cargo.toml
```

## Rules

- Write the test first. See it fail. Then write the code.
- A change to `policy.rs` needs the same change in `verus/policy.rs`. Run
  Verus. The test `verus_bodies_match` compares the two. Do not accept a
  clippy auto-fix in `policy.rs`. Add an `allow` with a reason.
- Only `transport.rs` does network I/O. Other code in `src/` uses
  `HttpTransport`.
- Seal each secret with the AAD of its row. Never store a plaintext secret.
- Store tokens, secret keys and OAuth state only as lookup hashes.
- Never follow a redirect in the proxy or the OAuth client.
- Never retry a non-idempotent call after a 5xx or a network error, unless
  the caller opts in.
- A new `Store` method needs a case in `store/conformance.rs`. It runs on
  `MemoryStore` and `PgStore`.
- Postgres tests: one pool for each test, unique ids, far-future times for
  rows that the purge must not touch.
- No `unwrap` or `expect` in `src/` outside tests.
- No `unsafe`.
- Do not log bodies, tokens or secrets. Use `Secret` for secret values.
- Do not copy Nango code or data. Read only the Nango file format.
- Do mutation tests on a `git clone` in the scratchpad. Never in the repo.
- Webhooks are at least once. Do not claim exactly once.
- A new example needs `test = true`, a CI step, a README row, a
  `RUN_TIMEOUT`, a `check` function and a failure test. The test
  `examples_wired` checks these items by text.
- After a dependency change in `san-mateo` or `san-mateo-camel`, update
  `examples/harvest-camel/Cargo.lock`:
  `cargo update --manifest-path examples/harvest-camel/Cargo.toml -p san-mateo -p san-mateo-camel`.
