# ADR 0006: Runnable examples that CI runs

- Status: Accepted
- Date: 2026-10-13

## Context

Users need code that they can run and copy for each way to use San Mateo:
the core in an axum app, the autumn-web plugin, a harvest workflow and a
harvest-camel route. Example code that no job runs can rot.

## Decision

- Each example is a cargo example with `test = true`, as in harvest-camel
  `order_flow`. `cargo test` runs its test. The `examples` CI job runs it
  with `cargo run`, as a user does.
- Each example checks exact values and returns an error when they are
  wrong. A failure test removes one fake reply and expects that error.
- The fake provider is `MemoryTransport`. The examples need no port for the
  provider and no dev-only proxy flags.
- The autumn example test uses `TestApp`, because `run()` cannot stop from
  code and ends the process when the boot fails. A script boots the real
  binary in the dev and prod profiles and stops it with SIGTERM.
- The harvest examples run a real worker on Postgres. Each run has its own
  queue and workflow id. Their workflows send activities to
  `ctx.queue_name()`.
- The harvest-camel example has its own cargo workspace in
  `examples/harvest-camel`. harvest-camel needs `autumn-harvest-plugin`,
  which turns on the autumn-web default features (C builds of libpq and
  SQLite). In the main workspace, these features would go into all builds.
- harvest-camel is not on crates.io. The example pins a full git `rev` with
  camel 0.57.

## Results

- Good: CI proves on each PR that the four integration paths work.
- Good: the examples are also end-to-end tests of the adapters.
- Bad: the harvest examples need Postgres.
- Bad: the separate workspace has its own `Cargo.lock`. A change to a
  dependency of `san-mateo` or `san-mateo-camel` needs a lock update there
  too. CI prints the command.
