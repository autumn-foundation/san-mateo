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
  `DELETE`. A caller can opt in for other methods with `Retry-On`. The
  provider can opt in with `retry.error_code`.
- 401 on an OAuth2 connection: refresh one time and send one more time.
- The wait comes from `Retry-After`, or from the provider `retry.at` and
  `retry.after` headers. Else it is exponential backoff with a cap. A wait above
  the cap gives up.
- Attempts are bounded: `1 + Retries`, max 10.
- `policy::decide` holds these rules. Verus proves them.
- `Base-Url-Override` must use HTTPS. Its host must be the provider host or a
  host in `proxy.allowed_hosts`.
- The proxy rejects upstream URLs with internal IP literals, unless
  `proxy.allow_private_hosts` is on (tests and local development). Internal
  means private, loopback, link-local, shared, benchmark, multicast or
  reserved. An IPv4 address inside an IPv6 address (mapped, compatible,
  NAT64, 6to4) uses the IPv4 rule. `localhost` and `*.localhost` (also with a
  trailing dot) are internal.
- An end-user value that changes the host of a provider URL needs one of:
  an `allowed` list in the catalog, a `pattern` that fixes the domain, or a
  host in `proxy.allowed_hosts`. The check renders the URL with two probe
  values and compares the last two host labels. A pattern fixes the domain
  when it rejects the probe hosts `san-mateo-probe.invalid` and
  `sanmateoprobe`. Values from the session defaults come from the developer.
  They need no check. Callback values (`redirect_uri_metadata`) get the same
  check before the token URL renders.
- Webhook override URLs must use HTTPS and a public host.
- The transport never follows redirects.

## Results

- Good: no double writes from retries.
- Good: credentials go only to known hosts.
- Bad: a caller that wants Nango's retry of `POST` on 5xx must send `Retry-On`.
- Bad: the check does not block DNS names that resolve to private IPs.
  Operators must use a network policy for that.
- Bad: the host check takes the last two labels as the site. A template such
  as `https://${x}.co.uk` passes. Write catalog URLs with a full domain.
