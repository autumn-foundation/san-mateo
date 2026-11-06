# ADR 0007: A SQLite store for embedded apps

- Status: Accepted
- Date: 2026-11-06
- Issue: #8

## Context

San Mateo had two stores: `MemoryStore`, which loses data at exit, and
`PgStore`. An app that keeps OAuth connections durably must run Postgres.

littleguy is an always-on personal agent. It runs on SQLite files through
`autumn-harvest-sqlite`, with no database server. It needs mail and calendar
tokens with refresh. A Postgres sidecar defeats the point of that design.

## Decision

- A new crate, `san-mateo-sqlite`, gives `SqliteStore`. It implements
  `san_mateo::Store`. The core crate does not change.
- It uses `rusqlite` 0.40 with `bundled`. `libsqlite3-sys` has
  `links = "sqlite3"`, so one app can link only one version. 0.40 uses the
  same `libsqlite3-sys` as `autumn-harvest-sqlite` 0.10.
- A store holds one connection behind a mutex. Each operation runs on the
  tokio blocking pool. So the operations of one store run one after the
  other.
- Each write with more than one statement runs in `BEGIN IMMEDIATE`. That
  takes the write lock of the file at the start. So two stores on one file
  also see atomic operations, and no transaction upgrades a read lock. A
  store waits up to 5 s for the lock. The file uses WAL mode.
- The schema has the tables of the Postgres schema. A time is an integer
  count of microseconds, as in Postgres `TIMESTAMPTZ`. A UUID is hyphenated
  lower-case text: its text order is its byte order. So cursors and sort
  orders are the same in both stores. A test proves the UUID order.
- `PRAGMA foreign_keys = ON` on each connection, so records and sync states
  cascade from their connection.
- `san-mateo-server` gets a `sqlite` feature. With it,
  `SAN_MATEO_DATABASE_URL=sqlite://<path>` selects `SqliteStore`. The
  feature is off by default, because it builds SQLite from C source.

## The lock file guard

ADR 0006 keeps the C build of SQLite out of the main workspace, because
harvest-camel brought it into every build. This ADR lets it in for one
crate only. The test `the_harvest_camel_workspace_stays_apart` checks that
only `rusqlite` depends on `libsqlite3-sys`, and only `san-mateo-sqlite`
depends on `rusqlite`. The test
`the_sqlite_store_links_the_sqlite_of_autumn_harvest` checks that the
version matches the harvest-camel lock file.

## Proof of parity

`SqliteStore` runs `store::conformance::run_all`, the suite that
`MemoryStore` and `PgStore` run. The concurrency tests of the Postgres store
have SQLite twins. These tests need no service, so they run in every CI run.

## Results

- Good: an app embeds San Mateo with one file and no server.
- Good: one suite defines the store contract for three stores.
- Bad: one connection serializes reads and writes of one store. A personal
  agent or a small server does not notice. A busy multi-tenant server should
  use Postgres.
- Bad: two processes on one file share one writer. Use Postgres for a fleet
  of workers.
- Out of scope: the `harvest_workflow` and `harvest_camel` examples run
  harvest on Postgres. A move to `autumn-harvest-sqlite` is a separate change.
