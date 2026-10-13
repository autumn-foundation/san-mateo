# ADR 0005: A framework-free core and adapter crates

- Status: Accepted
- Date: 2026-10-13

## Context

San Mateo must run in autumn-web, in autumn-harvest, in rust-camel, and alone.
These crates are large. Each has its own release cadence.

## Decision

- `san-mateo` is the core. It has no autumn, harvest or camel dependency. It
  gives an axum `Router` for the API.
- `autumn-plugin-san-mateo` nests the router into an autumn app.
- `san-mateo-harvest` gives activities and workflows. Activities read
  `SanMateo` from harvest state.
- `san-mateo-camel` gives a camel component.
- `san-mateo-postgres` gives the Postgres store. It uses the same diesel-async
  pool type as autumn-web and autumn-harvest.
- `san-mateo-server` is the standalone binary.

## Syncs in harvest

A harvest schedule is unique for each workflow name. A schedule for each
connection is not possible. A sync is an entity workflow for each
(connection, sync). It runs the sync activity, waits with `timer`, and calls
`continue_as_new`.

## Results

- Good: a user of one adapter does not compile the others.
- Good: the core tests run fast.
- Bad: the API router is axum. autumn mounts it with `nest`, so the plugin must
  declare its routes.
