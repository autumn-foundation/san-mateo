# ADR 0004: Proxy retry rules and SSRF checks

- Status: Accepted
- Date: 2026-10-13

## Context

Nango retries a proxy call on 5xx, 429 and 401 for all HTTP methods when the
caller sets `Retries`. After a 5xx or a network error, the provider possibly
did the call. A `POST` can then occur two times. Nango also accepts any
`Base-Url-Override`, so the caller can send credentials to any host.

## Decision

- 429: retry for all methods. A 429 means the provider did not do the call.
- 5xx and network errors: retry only for `GET`, `HEAD`, `OPTIONS`, `PUT` and
  `DELETE`. A caller can opt in for other methods with `Retry-On`.
- 401 on an OAuth2 connection: refresh one time and send one more time.
- The wait comes from `Retry-After`, or from the provider `retry.at` and
  `retry.after` headers. Else it is exponential backoff with a cap. A wait above
  the cap gives up.
- Attempts are bounded: `1 + Retries`, max 10.
- `policy::decide` holds these rules. Verus proves them.
- `Base-Url-Override` must use HTTPS. Its host must be the provider host or a
  host in `proxy.allowed_hosts`.
- Upstream URLs with private, loopback or link-local IP literals are rejected,
  unless `proxy.allow_private_hosts` is on (tests and local development).
- The transport never follows redirects.

## Results

- Good: no double writes from retries.
- Good: credentials go only to known hosts.
- Bad: a caller that wants Nango's retry of `POST` on 5xx must send `Retry-On`.
- Bad: DNS names that resolve to private IPs are not blocked. Operators must
  use a network policy for that.
