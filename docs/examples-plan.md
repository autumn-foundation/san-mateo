# Plan: runnable examples

Style: ASD-STE100. Short sentences. Active voice.

## 1. Problem

Users need code that they can run and copy for each way to use San Mateo:
embedded in an axum app, as an autumn-web plugin, in an autumn-harvest
workflow, and in a harvest-camel route. The README has short snippets, but
no build compiles them, so they can rot. The task: four runnable examples,
and CI runs each one.

## 2. Research

| Source | Finding | Use |
|---|---|---|
| harvest-camel `examples/order_flow.rs` | A cargo example with `test = true`. `main` reads `DATABASE_URL`. It checks its result and fails when it is wrong. A run timeout. CI does `cargo run --example`. | The house style for all four examples. |
| `MemoryTransport` | Replies come in order, and the last reply repeats. It logs each request. | The fake provider. No port, no dev-only proxy flags. |
| autumn-web 0.8 `run()` | No stop from code. It calls `process::exit(1)` when the boot fails. It sets a global tracing subscriber. | The test uses `TestApp`. A script boots the real binary and stops it with SIGTERM. |
| autumn-harvest 0.7 | Postgres only for async activities. `migrate::apply` is safe in parallel when the role can lock the ledger. An activity with no queue goes to `"default"`. | A real worker on a queue for each run. Activities go to `ctx.queue_name()`. |
| autumn-harvest-sqlite 0.7 | Activities are sync closures with no `ActivityContext`. | Not usable for the San Mateo activities. |
| harvest-camel main (at the start) | camel 0.55. San Mateo uses camel 0.57. | Bump harvest-camel to 0.57 (approved; PRs 5 and 6 merged). Pin a commit of `main`. |
| harvest-camel deps | `autumn-harvest-plugin` turns on the autumn-web default features (`db`, C builds of libpq and SQLite). | Put the harvest-camel example in its own cargo workspace. |

## 3. Brainstorming

Ideas for each example. The chosen ideas are in bold.

- Embedded: **your own axum app nests `api::router`**; **OAuth connect over
  the REST API with no browser and no followed redirect**; **refresh before a
  proxy call**; **your own handler calls `san.proxy`**; **a signed `auth`
  webhook**; **background loops and a clean stop**; a sync with records (left
  out: the tests cover it, and the example must stay short).
- autumn plugin: **one `.plugin(..)` call**; **a route with the
  `SanMateoService` extractor**; **an API key connect over the REST API**;
  **config in `autumn.toml` with env var names only**; **a boot of the real
  binary in the dev and prod profiles**; **SIGTERM stop**.
- harvest workflow: **your own workflow calls `san_mateo_proxy`**; **a `POST`
  that is safe to retry because it forwards the idempotency key**; **a 503
  that harvest retries**; the library sync loop (left out: it never ends).
- harvest-camel: **an activity calls a camel route with
  `call_route_from_activity`**; **the route calls `san-mateo:`**; **the route
  copies `HarvestIdempotencyKey` upstream**; **a 404 that harvest does not
  retry**; **the San Mateo error map agrees with the real harvest-camel
  `is_retryable`**; the inbound `harvest:` bridge (left out: order_flow shows
  it).

## 4. Reverse brainstorming (how to make the examples fail)

| How to fail | Counter-measure |
|---|---|
| An example compiles, but no job runs it. | CI runs each example as a user does (`cargo run`). A guard test checks the wiring. |
| An example passes when the feature is broken. | Each example checks exact values (headers, call counts, JSON). A failure test for each example removes one fake reply and expects an error. |
| An example hangs CI. | `RUN_TIMEOUT` around the scenario. `timeout-minutes` on the jobs. Stop the server, loops, worker and camel on all paths. |
| A database test skips in CI and stays green. | When `CI` is set and the URL is not set, the test fails and names the variable. |
| Two runs on one database take the tasks of each other. | A unique queue and workflow id for each run. A CI step runs two copies at the same time. |
| An example teaches an unsafe setting. | No `allow_private_hosts`, no `allow_plain_http`, no fixed key. Keys are random at run time. Config holds env var names only. |
| An example leaks a secret. | The embedded and autumn checks scan the summary (and the webhook and health replies) for each secret. The smoke script scans the app log for the keys. Failure messages hide the keys. |
| A copied route trusts an id from the URL (IDOR). | A comment says: in production, get the connection id from the signed-in user. |
| The harvest-camel pin moves or a second camel version comes in. | A full 40-hex `rev`. A CI step fails on two versions of camel or autumn-harvest. |
| The heavy harvest-camel dependencies enter the main workspace. | A separate workspace. The guard test checks the root `Cargo.lock`. |
| The separate lock goes stale when a San Mateo dependency changes. | A first CI step checks the lock and prints the fix command. |
| A new upstream release breaks a copied example. | A weekly run updates the dependencies and runs the examples. |
| A shell step hides a failure. | `set -euo pipefail`. Wait for each process and check each exit code. `shellcheck`. |
| The README snippet rots. | A doc test compiles the README Rust blocks. |

## 5. Six thinking hats

- **White (facts):** 453 tests pass now. CI has fmt, clippy, test (Postgres,
  coverage 85%), msrv and verus jobs. harvest-camel builds on camel 0.57 with
  a version bump and one lint allow. Its tests, its example and MSRV 1.89
  pass on 0.57.
- **Red (feelings):** Users want to copy one file and see it work. They do
  not want to set up provider accounts. They fear token leaks.
- **Black (risks):** autumn `run()` cannot run in a test. The harvest
  examples need Postgres. The harvest-camel job is slow on a cold cache.
  The library harvest workflows send activities to `"default"`. Two
  harvest-camel PRs make the same change.
- **Yellow (benefits):** CI proves that all four integration paths work
  together on each PR. The examples are also end-to-end tests of the
  adapters.
- **Green (ideas):** A guard test for the wiring. A failure test for each
  example. A weekly run with new dependencies. A parity test between the San
  Mateo error map and harvest-camel. A doc test for the README.
- **Blue (process):** Plan, then for each example: RED (the test, see it
  fail), GREEN (the scenario), REFACTOR (fmt, clippy, STE). Then CI, docs, a
  review from several angles, fixes, and the AC evidence.

## 6. Design

| Example | Target | Fake provider | Postgres | CI |
|---|---|---|---|---|
| `embedded` | `crates/san-mateo/examples/embedded.rs` | `MemoryTransport` | No | `examples` job: `cargo run -p san-mateo --example embedded` |
| `autumn_app` | `crates/autumn-plugin-san-mateo/examples/autumn_app/` (`main.rs`, `autumn.toml`) | `MemoryTransport` in the test | No | `examples` job: `scripts/smoke-autumn-example.sh` |
| `harvest_workflow` | `crates/san-mateo-harvest/examples/harvest_workflow.rs` | `MemoryTransport` | Yes | `examples` job: `cargo run`, and two runs at the same time |
| `harvest_camel` | `examples/harvest-camel/` (own workspace) | `MemoryTransport` | Yes | `example-harvest-camel` job: lock, fmt, clippy, test, run, MSRV, one camel version |

Rules for each example:

- One file (or one folder). A `//!` header with the flow, the run command
  and what to change for production.
- `[[example]] test = true`. The `#[cfg(test)]` test runs the same `run`.
- `run` returns a summary. A pure `check` compares exact values. `main`
  prints the summary and returns an error when `check` fails.
- `RUN_TIMEOUT` wraps the scenario. The stop runs on all paths, before the
  first `?` on the result.
- The harvest examples read `DATABASE_URL` in `main` and
  `SAN_MATEO_TEST_DATABASE_URL` in the test.
- No `unwrap` or `expect` outside tests. No `unsafe`.

## 7. Acceptance criteria

- **EX1 Embedded.** `cargo run -p san-mateo --example embedded` needs no
  database. An axum app nests `api::router`. The OAuth connect runs over the
  REST API: a 302 that the client does not follow, `state`, PKCE S256, then
  the callback. The first token expires soon, so San Mateo refreshes before
  the proxy call. The app handler returns the exact contacts JSON. The
  upstream call has `Bearer` with the new token. An upstream error gives
  502. The `auth` webhook has a valid signature and no secret. The server,
  the loops and the webhooks stop cleanly.
- **EX2 autumn plugin.** The test builds the same routes and the
  `[san_mateo]` section of the same `autumn.toml` as `main`, in `TestApp`.
  It connects an API key over the REST API. The app route with
  `SanMateoService` returns the contacts, and the API key goes upstream. An
  upstream error gives 502. The smoke script boots the real binary in the
  dev and prod profiles and checks the routes and the connect page. It
  stops the app with SIGTERM (exit code 0). It checks that a missing key
  stops the boot with the variable name. It finds no key in the log.
- **EX3 harvest workflow.** A real harvest worker on Postgres runs a
  workflow that calls `san_mateo_proxy`. A `POST` that forwards the
  idempotency key gets a 503, harvest retries it, and both sends have the
  same key and the San Mateo auth. A `GET` follows. The result is exact. Two
  runs at the same time on one database pass.
- **EX4 harvest-camel.** A harvest activity calls a camel route with
  `call_route_from_activity`. The route calls `san-mateo:` and copies
  `HarvestIdempotencyKey` upstream. With no key, the route sends no `POST`.
  harvest retries the 503 with one key. It does not retry the 404. The
  result is exact. A test shows that the San Mateo error map agrees with
  the real harvest-camel `is_retryable`.
- **EX5 Self-check.** For each example, failure tests change one fake
  reply. They expect an error that starts with `check failed:`, or a failed
  workflow with no retry. The stop still runs. A wrong result gives a
  non-zero exit code.
- **EX6 No hang.** `RUN_TIMEOUT` of 120 s or less in each example. The new
  CI jobs have `timeout-minutes`.
- **EX7 No secret leaks.** The summaries and the autumn log hold no token
  or key. No example has a fixed key. Keys are random at run time.
- **EX8 Database tests.** With no URL they skip with a notice. With `CI`
  set and no URL they fail and name the variable.
- **EX9 CI.** The `examples` job runs each main-workspace example as a user
  does. The `example-harvest-camel` job gates the separate workspace. A
  weekly run and a manual run update the dependencies and run the
  examples.
- **EX10 Gates.** fmt, clippy `-D warnings` on all targets, `cargo doc -D
  warnings` with the examples, MSRV 1.89 `--locked`, coverage 85% or more.
- **EX11 Guard.** A test fails when an example loses `test = true`, its
  `check` function, its failure test, its run timeout, its CI commands or
  its README row, when an example turns on a dev-only proxy flag, when an
  example is not in the list, or when the root `Cargo.lock` gets the
  harvest-camel dependencies.
- **EX12 Docs.** A README table with the commands that CI runs. No stale
  "camel 0.55" text. CLAUDE.md lists the paths, the commands and the rule
  for new examples. The `rust` and `rust,no_run` blocks of the README
  compile as doc tests. ADR 0006.
  The docs give the queue limit of the library workflows. ASD-STE100.
- **EX13 TDD.** For each example, the test failed before the code (RED
  commit) and passed after it (GREEN commit).
- **EX14 harvest-camel pin.** harvest-camel `main` is on camel 0.57 (PRs 5
  and 6). The example pins a full 40-hex `rev` of `main`. The example lock
  has one version each of camel and autumn-harvest. Since harvest-camel
  0.2.0 (autumn-harvest 0.8), the example uses the crates.io release.

## 8. Out of scope

- A fix to the library harvest workflows (`san_mateo_sync_loop`,
  `san_mateo_refresh_sweep`) to send activities to `ctx.queue_name()`. It
  changes behaviour, so it needs approval. This work writes the limit down.
- The inbound `harvest:` bridge. harvest-camel `order_flow` shows it.
- Real providers, `ReqwestTransport` and `PgStore` in the examples. The
  tests cover them.
