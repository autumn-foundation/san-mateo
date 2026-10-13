# San Mateo

San Mateo is an integration auth platform in Rust. It is a clean-room
version of the [Nango](https://github.com/NangoHQ/nango) public API.

San Mateo does these tasks for the users of your product:

- It keeps OAuth tokens, API keys and passwords. It seals them with
  AES-256-GCM.
- It refreshes OAuth tokens before they expire.
- It sends authorized calls to provider APIs through a proxy.
- It runs syncs and actions that you write in Rust. It keeps the records
  that syncs make.
- It sends signed webhooks to your backend.

San Mateo is part of the autumn ecosystem. Use it as a standalone server, as
an [autumn-web](https://github.com/autumn-foundation/autumn) plugin, from
[autumn-harvest](https://github.com/autumn-foundation/autumn-harvest)
workflows, or from [rust-camel](https://github.com/autumn-foundation/harvest-camel)
routes.

## Crates

| Crate | Use it to |
|---|---|
| [`san-mateo`](crates/san-mateo) | Embed the core: service, REST API (axum), memory store. |
| [`san-mateo-postgres`](crates/san-mateo-postgres) | Keep data in Postgres (diesel-async). |
| [`san-mateo-server`](crates/san-mateo-server) | Run the `san-mateo` binary. |
| [`autumn-plugin-san-mateo`](crates/autumn-plugin-san-mateo) | Mount San Mateo in an autumn-web app. |
| [`san-mateo-harvest`](crates/san-mateo-harvest) | Call San Mateo from durable harvest workflows. |
| [`san-mateo-camel`](crates/san-mateo-camel) | Call provider APIs from camel routes (`san-mateo:` URIs). |

## Design

```mermaid
flowchart LR
  subgraph Product
    BE[Backend] -->|Bearer secret key| API
    FE[End user browser] -->|connect session token| CONNECT
  end
  subgraph Core[san-mateo]
    API[REST API] --> SVC[SanMateo service]
    CONNECT[Connect page and auth flows] --> SVC
    SVC --> CAT[Catalog: Nango-format YAML]
    SVC --> POL{{policy: proved with Verus}}
    SVC --> KR[KeyRing: AES-256-GCM]
    SVC --> ST[(Store)]
    SVC --> TR[HTTP transport: no redirects]
    SVC --> FN[Syncs and actions]
    SVC --> WH[Webhook sender]
  end
  ST --> MEM[MemoryStore]
  ST --> PG[(PgStore)]
  TR --> P[(Provider APIs)]
  WH -->|HMAC-SHA256| BE
  AUT[autumn-plugin-san-mateo] --> API
  HAR[san-mateo-harvest] --> SVC
  CAM[san-mateo-camel] --> SVC
  BIN[san-mateo-server] --> API
```

The OAuth flow:

```mermaid
sequenceDiagram
  participant B as Backend
  participant S as San Mateo
  participant U as Browser
  participant P as Provider
  B->>S: POST /connect/sessions (secret key)
  S-->>B: token, connect_link
  B->>U: open connect_link
  U->>S: GET /oauth/connect/{integration}
  S-->>U: 302 to provider (state, PKCE S256)
  U->>P: user consents
  P-->>U: 302 to /oauth/callback?code&state
  U->>S: GET /oauth/callback
  S->>P: token request
  P-->>S: tokens
  S->>S: seal and store the connection
  S->>B: auth webhook (signed)
```

Scopes work as in Nango:

- The `scope` parameter holds the integration scopes. When the integration
  has no scopes, it holds the provider `default_scopes`.
- The session `user_scopes` go in `connection_config.user_scope`. For
  Slack, they also go in the `user_scope` parameter. They never replace
  `scope`.

More design detail: [docs/plan.md](docs/plan.md) and the ADRs in
[docs/adr](docs/adr).

## Quick start: the server

1. Make a key: `openssl rand -base64 32`.
2. Start Postgres (optional). With no database, data stays in memory.
3. Run the server:

```sh
export SAN_MATEO_ENCRYPTION_KEY="$(openssl rand -base64 32)"
export SAN_MATEO_DEV_SECRET_KEY="sk_dev_$(openssl rand -hex 16)"
export SAN_MATEO_DATABASE_URL="postgres://user:password@localhost/san_mateo"
cargo run -p san-mateo-server --release
```

4. Make an integration and a connect session:

```sh
curl -s localhost:3003/integrations \
  -H "Authorization: Bearer $SAN_MATEO_DEV_SECRET_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"unique_key":"github","provider":"github","credentials":{"type":"OAUTH2","client_id":"...","client_secret":"..."}}'

curl -s localhost:3003/connect/sessions \
  -H "Authorization: Bearer $SAN_MATEO_DEV_SECRET_KEY" \
  -H 'Content-Type: application/json' \
  -d '{"end_user":{"id":"u1"}}'
```

5. Open `connect_link` in a browser. After the user connects, call the
   provider through the proxy:

```sh
curl -s localhost:3003/proxy/user \
  -H "Authorization: Bearer $SAN_MATEO_DEV_SECRET_KEY" \
  -H 'Provider-Config-Key: github' \
  -H 'Connection-Id: <connection id>'
```

The server reads these variables:

| Variable | Meaning |
|---|---|
| `SAN_MATEO_SERVER_URL` | Public URL of the API. Default `http://localhost:3003`. |
| `SAN_MATEO_CONNECT_URL` | Base of `connect_link`. Default `<server URL>/connect`. |
| `SAN_MATEO_BIND` | Listen address. Default `0.0.0.0:3003`. |
| `SAN_MATEO_DATABASE_URL` | Postgres URL. With no URL, data stays in memory. |
| `SAN_MATEO_ENCRYPTION_KEY` | Base64 32-byte key. Required. |
| `SAN_MATEO_ENCRYPTION_KEY_ID` | Id of that key. Default `k1`. |
| `SAN_MATEO_OLD_ENCRYPTION_KEYS` | Old keys for rotation: `id:base64,id:base64`. |
| `SAN_MATEO_LOOKUP_KEY_ID` | Key id for lookup hashes. Default: the active key. Required with old keys. |
| `SAN_MATEO_ENVIRONMENTS` | Environment ids, comma separated. Default `dev`. |
| `SAN_MATEO_<ENV>_SECRET_KEY` | Secret key of each environment. Required. |
| `SAN_MATEO_<ENV>_CALLBACK_URL` | OAuth callback URL of the environment. |
| `SAN_MATEO_<ENV>_WEBHOOK_URL`, `..._WEBHOOK_URL_SECONDARY` | Webhook URLs. |
| `SAN_MATEO_<ENV>_WEBHOOK_SECRET` | Webhook signing secret. |
| `SAN_MATEO_PROVIDERS_FILE` | A Nango-format YAML file. It adds to the built-in catalog. |
| `SAN_MATEO_ALLOWED_HOSTS` | Extra hosts for `Base-Url-Override` and end-user host values, comma separated. |
| `SAN_MATEO_ALLOW_PRIVATE_HOSTS`, `SAN_MATEO_ALLOW_PLAIN_HTTP` | Development only. Default `false`. |
| `SAN_MATEO_BACKGROUND` | Run the refresh sweep, the sync scheduler and the session purge. Default `true`. |
| `SAN_MATEO_TICK_SECS` | Seconds between background ticks. Default `30`. |
| `RUST_LOG` | Log level. Default `info`. |

`<ENV>` is the environment id in upper case, with `-` as `_`. For example,
the secret key of `prod-eu` is `SAN_MATEO_PROD_EU_SECRET_KEY`.

Exit codes: `0` after a clean stop (`SIGTERM` or Ctrl-C), `1` when the server
fails, `2` when the settings are wrong.

## Embed the core

```rust,no_run
use std::sync::Arc;
use san_mateo::{EnvironmentConfig, KeyRing, SanMateo};
use san_mateo_postgres::PgStore;

# async fn demo() -> san_mateo::Result<()> {
let store = PgStore::connect("postgres://localhost/san_mateo", 16)?;
store.migrate().await?;
let san = SanMateo::builder()
    .server_url("https://sm.example.com")
    .environment(EnvironmentConfig::new("prod", "sk_live_..."))
    .key_ring(KeyRing::from_base64("k1", "<32 random bytes, base64>")?)
    .store(Arc::new(store))
    .build()?;
let app: axum::Router = san_mateo::api::router(san.clone());
let background = san.start_background(Default::default());
# let _ = (app, background);
# Ok(())
# }
```

Write syncs and actions as Rust types that implement `SyncFunction` and
`ActionFunction`. Put them in a `Registry` and give it to the builder.

## Use from the ecosystem

### autumn-web

See [`autumn-plugin-san-mateo`](crates/autumn-plugin-san-mateo).

### autumn-harvest

See [`san-mateo-harvest`](crates/san-mateo-harvest).

### rust-camel

Register `SanMateoComponent::new(san)` with the camel context. Then send
exchanges to an endpoint such as:

```text
san-mateo:hubspot?connectionId=acme&path=/crm/v3/objects/contacts&retries=2
```

- The exchange body goes upstream. `SanMateoProxy-<Name>` headers go
  upstream as `<Name>`.
- The reply sets the body, `CamelHttpResponseCode` and
  `SanMateoResponse-<name>` headers. The component does not copy secret
  reply headers.
- Errors map to camel errors. harvest-camel retries the same errors that
  San Mateo marks as retryable.
- The `SanMateoConnectionId`, `SanMateoMethod` and `SanMateoPath` headers
  replace the URI options only with `allowHeaderOverrides=true`. The
  default is `false`. Do not set it on a route where an outside caller can
  set headers.

## Differences from Nango

San Mateo uses the paths and JSON shapes of the Nango public API. These
items are different:

- **Functions.** Syncs and actions are Rust types, not TypeScript scripts.
  In-process syncs are not durable. Use `san-mateo-harvest` for durable syncs.
- **Auth modes.** San Mateo supports `OAUTH2`, `OAUTH2_CC`, `API_KEY`,
  `BASIC` and `NONE`. The catalog reads all other modes. The service rejects
  them with `unsupported_auth_mode`.
- **Environments and secret keys** come from the configuration, not from a
  database. There is no dashboard.
- **Connect UI.** A small server-side HTML page replaces the React Connect
  UI. Connect session tokens start with `smcs_`.
- **Proxy headers.** San Mateo accepts `San-Mateo-Proxy-*` and
  `Nango-Proxy-*`.
- **Webhooks.** San Mateo sends `X-San-Mateo-Hmac-Sha256` and
  `X-Nango-Hmac-Sha256`. Both hold the hex HMAC-SHA256 of the body. San
  Mateo does not send the old `X-Nango-Signature` header.
- **Records** have the Nango `_nango_metadata` key. San Mateo seals record
  data at rest.
- **Not included:** OAuth 1.0a and the other auth modes above, provider
  webhook routing scripts, TypeScript deploy, billing and KMS-wrapped keys.

## Security

- **At rest.** San Mateo seals credentials, client secrets, PKCE verifiers
  and records with AES-256-GCM. The AAD binds each value to its row, so a
  value copied to a different row does not open. See
  [ADR 0003](docs/adr/0003-row-bound-encryption.md).
- **Keys.** Keep `SAN_MATEO_ENCRYPTION_KEY` in a secret store. To rotate,
  set a new key and move the old one to `SAN_MATEO_OLD_ENCRYPTION_KEYS`.
  Set `SAN_MATEO_LOOKUP_KEY_ID` to the first key id: the lookup hashes of
  secret keys, tokens and records use it. The server and the autumn plugin
  do not start with old keys and no lookup key id.
- **Lookups.** San Mateo finds secret keys, connect session tokens and
  OAuth state by their HMAC-SHA256 hash. The store keeps only the hashes of
  tokens and state. San Mateo compares secret keys in constant time.
- **Proxy.** The proxy does not follow redirects. `Base-Url-Override` must
  use HTTPS and the provider host or a host in `SAN_MATEO_ALLOWED_HOSTS`.
  The proxy rejects internal IP literals (private, loopback, link-local,
  shared, multicast, reserved), also inside IPv6 forms (mapped, NAT64,
  6to4). It also rejects `localhost` names. See
  [ADR 0004](docs/adr/0004-proxy-retry-and-ssrf.md).
- **End-user hosts.** An end-user value can set the host of a provider
  URL. Such a value needs one of these: an `allowed` list in the catalog, a
  catalog `pattern` that fixes the domain, or a host in
  `SAN_MATEO_ALLOWED_HOSTS`. Values in the session defaults come from you
  and need no check.
- **OAuth parameters.** An end user cannot set `client_id`,
  `client_secret`, `redirect_uri`, `state`, `scope`, `response_type` or the
  PKCE parameters. Session defaults can set them.
- **Webhook overrides** must use HTTPS and a public host.
- **DNS.** The SSRF check reads IP literals. It does not resolve host names.
  Put a network policy in front of San Mateo if a provider host can resolve
  to a private address.
- **Logs.** San Mateo does not log tokens, secrets or bodies. `Debug`
  output of `Secret` and of the server settings hides the values.
- **Postgres TLS.** `PgStore::connect` makes a pool with no TLS. For TLS,
  make a pool with a TLS setup and use `PgStore::new`.
- **Crypto backends.** The core uses `ring` through rustls. The autumn
  plugin and the camel crate also use `aws-lc-rs` through `jsonwebtoken`.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
SAN_MATEO_TEST_DATABASE_URL=postgres://sm:sm@localhost/sm_test cargo test --workspace
cargo llvm-cov --workspace --fail-under-lines 85
```

- The Postgres tests run only when `SAN_MATEO_TEST_DATABASE_URL` is set.
- `verus/policy.rs` holds the policy proofs. The test
  `policy::verus_bodies_match` fails when `src/policy.rs` and the spec are
  not the same. CI runs Verus.
- Set the git hooks: `git config core.hooksPath .githooks`.
- [docs/ac-evidence.md](docs/ac-evidence.md) lists each acceptance
  criterion and its evidence.

## License

Apache-2.0. San Mateo has no Nango code. It reads files in the Nango
`providers.yaml` format. See
[ADR 0001](docs/adr/0001-clean-room-and-nango-format.md).
