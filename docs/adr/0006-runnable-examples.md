# ADR 0006: Runnable examples that CI runs

- Status: Accepted
- Date: 2026-10-13

## Context

Users need code that they can run and copy for each way to use San Mateo:
the core in an axum app, the autumn-web plugin, a harvest workflow and a
harvest-camel route. Example code that no job runs can rot.

## Decision

- Three examples are cargo examples with `test = true`, as in harvest-camel
  `order_flow`. `cargo test` runs their tests. The harvest-camel example is
  a binary in its own workspace, and its own CI job runs its tests. CI runs
  each example with `cargo run`, as a user does.
- Each example checks exact values and returns an error that starts with
  `check failed:` when they are wrong. Failure tests change one fake reply
  and expect that error, or expect a failed workflow with no retry.
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
- harvest-camel was not on crates.io at first. The example pinned a full
  git `rev` with camel 0.57. Since harvest-camel 0.2.0 (2026-10-31), the
  example uses the crates.io release.

## Results

- Good: CI proves on each PR that the four integration paths work.
- Good: the examples are also end-to-end tests of the adapters.
- Bad: the harvest examples need Postgres.
- Bad: the separate workspace has its own `Cargo.lock`. A change to a
  dependency of `san-mateo` or `san-mateo-camel` needs a lock update there
  too. CI prints the command.
