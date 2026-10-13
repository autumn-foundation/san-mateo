# ADR 0002: Own OAuth2 client on `HttpTransport`

- Status: Accepted
- Date: 2026-10-13

## Context

OAuth2 token replies are not uniform. Some providers put the token under a
nested key (Slack: `authed_user`). Some give `expires_in` in milliseconds. Some
need Basic client auth (Notion, Airtable). Some need a JSON body. Some give no
`token_type`. The Nango catalog describes these quirks as data.

## Options

1. Use the `oauth2` crate (5.0).
2. Write a small OAuth2 client on the `HttpTransport` trait.

## Decision

Option 2.

## Reasons

- The `oauth2` crate parses the token reply into a strict type. A reply with
  no `token_type` or a nested token fails.
- The catalog data (`token_params`, `body_format`, `token_request_auth_method`,
  `alternate_access_token_response_path`, `expires_in_unit`) maps directly to
  request and parse options.
- `MemoryTransport` tests every flow with no network.
- PKCE S256 and `state` need little code. Tests check PKCE against the
  RFC 7636 vector.

## Results

- Good: one code path for all providers. Quirks are data.
- Good: tests use the RFC 7636 appendix B vector.
- Bad: San Mateo owns the OAuth2 code. Tests must cover it well.
