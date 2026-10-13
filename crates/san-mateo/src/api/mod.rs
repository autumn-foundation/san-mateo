//! The REST API as an axum [`Router`].
//!
//! Paths and JSON shapes follow the Nango public API. See `docs/plan.md`
//! section 8. `Authorization: Bearer <secret key>` selects the environment.
//! Connect routes take a connect session token in `connect_session_token`
//! (query) or in `Authorization: Bearer smcs_...`.

use std::collections::BTreeMap;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{FromRequestParts, Path, RawQuery, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::credentials::Credentials;
use crate::error::Error;
use crate::model::{ConnectionKey, EndUser, IntegrationDefaults, Organization, Tags};
use crate::proxy::ProxyRequest;
use crate::secret::Secret;
use crate::service::{
    CONNECT_TOKEN_PREFIX, ConnectOptions, ConnectSession, ConnectSessionCreated, ConnectSessionInput,
    Connection, ConnectionPatch, ConnectionSummary, GetOptions, ImportConnection, Integration,
    IntegrationPatch, NewIntegration, RecordsRequest, SanMateo, SyncStatusEntry,
};
use crate::store::ConnectionFilter;

mod html;

/// One route of the API, for route listings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteSpec {
    /// The HTTP method, or `ANY`.
    pub method: &'static str,
    /// The path, with `{name}` for path parameters.
    pub path: &'static str,
    /// `true` when the route needs the secret key.
    pub secret_key: bool,
}

const fn r(method: &'static str, path: &'static str, secret_key: bool) -> RouteSpec {
    RouteSpec { method, path, secret_key }
}

/// All routes of the API.
pub const ROUTES: &[RouteSpec] = &[
    r("GET", "/health", false),
    r("GET", "/providers", true),
    r("GET", "/providers/{name}", true),
    r("GET", "/integrations", true),
    r("POST", "/integrations", true),
    r("GET", "/integrations/{key}", true),
    r("PATCH", "/integrations/{key}", true),
    r("DELETE", "/integrations/{key}", true),
    r("POST", "/connect/sessions", true),
    r("POST", "/connect/sessions/reconnect", true),
    r("GET", "/connect/session", false),
    r("DELETE", "/connect/session", false),
    r("GET", "/connect", false),
    r("GET", "/oauth/connect/{key}", false),
    r("GET", "/oauth/callback", false),
    r("POST", "/oauth2/auth/{key}", false),
    r("POST", "/api-auth/api-key/{key}", false),
    r("POST", "/api-auth/basic/{key}", false),
    r("POST", "/auth/unauthenticated/{key}", false),
    r("GET", "/connections", true),
    r("POST", "/connections", true),
    r("POST", "/connections/metadata", true),
    r("PATCH", "/connections/metadata", true),
    r("GET", "/connections/{id}", true),
    r("PATCH", "/connections/{id}", true),
    r("DELETE", "/connections/{id}", true),
    r("GET", "/connection", true),
    r("GET", "/connection/{id}", true),
    r("DELETE", "/connection/{id}", true),
    r("ANY", "/proxy/{*path}", true),
    r("GET", "/records", true),
    r("POST", "/sync/trigger", true),
    r("POST", "/sync/pause", true),
    r("POST", "/sync/start", true),
    r("GET", "/sync/status", true),
    r("POST", "/action/trigger", true),
];

/// Returns the API router.
#[must_use]
pub fn router(san: SanMateo) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/providers", get(list_providers))
        .route("/providers/{name}", get(get_provider))
        .route("/integrations", get(list_integrations).post(create_integration))
        .route(
            "/integrations/{key}",
            get(get_integration).patch(update_integration).delete(delete_integration),
        )
        .route("/connect/sessions", post(create_session))
        .route("/connect/sessions/reconnect", post(create_reconnect_session))
        .route("/connect/session", get(get_session).delete(delete_session))
        .route("/connect", get(html::connect_page))
        .route("/oauth/connect/{key}", get(oauth_connect))
        .route("/oauth/callback", get(html::oauth_callback))
        .route("/oauth2/auth/{key}", post(connect_client_credentials))
        .route("/api-auth/api-key/{key}", post(connect_api_key))
        .route("/api-auth/basic/{key}", post(connect_basic))
        .route("/auth/unauthenticated/{key}", post(connect_unauthenticated))
        .route("/connections", get(list_connections).post(import_connection))
        .route("/connections/metadata", post(set_metadata).patch(update_metadata))
        .route(
            "/connections/{id}",
            get(get_connection).patch(patch_connection).delete(delete_connection),
        )
        .route("/connection", get(list_connections))
        .route("/connection/{id}", get(get_connection).delete(delete_connection))
        .route("/proxy/{*path}", any(proxy))
        .route("/records", get(list_records))
        .route("/sync/trigger", post(trigger_syncs))
        .route("/sync/pause", post(pause_syncs))
        .route("/sync/start", post(start_syncs))
        .route("/sync/status", get(sync_status))
        .route("/action/trigger", post(trigger_action))
        .with_state(san)
}

// ------------------------------------------------------------------ errors

/// An API error: `{"error": {"code", "message"}}` with the error status.
pub(crate) struct ApiError(pub(crate) Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.0.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if status.is_server_error() {
            tracing::error!(code = self.0.code(), "request failed");
        }
        let message = match &self.0 {
            // Internal details stay in the log.
            Error::Store(_) | Error::Internal(_) => "an internal error occurred".to_owned(),
            other => other.to_string(),
        };
        (status, axum::Json(json!({"error": {"code": self.0.code(), "message": message}}))).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

fn invalid(msg: impl Into<String>) -> ApiError {
    ApiError(Error::Invalid(msg.into()))
}

/// Parses a JSON body. An empty body gives `Value::Null`.
fn body_json<T: serde::de::DeserializeOwned>(bytes: &Bytes) -> ApiResult<T> {
    let v: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(bytes).map_err(|e| invalid(format!("the body is not valid JSON: {e}")))?
    };
    serde_json::from_value(v).map_err(|e| invalid(format!("the body is not valid: {e}")))
}

// ------------------------------------------------------------- extractors

fn bearer(parts: &Parts) -> Option<&str> {
    let v = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// The environment of the secret key.
pub(crate) struct Env(pub(crate) String);

impl FromRequestParts<SanMateo> for Env {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, san: &SanMateo) -> ApiResult<Self> {
        let key = bearer(parts).ok_or_else(|| ApiError(Error::Unauthorized("a secret key is required".to_owned())))?;
        Ok(Self(san.authenticate(key)?.id.clone()))
    }
}

/// Query pairs in order.
fn query_pairs(raw: Option<&str>) -> Vec<(String, String)> {
    raw.map(|q| url::form_urlencoded::parse(q.as_bytes()).into_owned().collect())
        .unwrap_or_default()
}

fn query_get<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// The connect session token, from the query or the `Authorization` header.
pub(crate) struct SessionToken(pub(crate) String);

impl FromRequestParts<SanMateo> for SessionToken {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _san: &SanMateo) -> ApiResult<Self> {
        let pairs = query_pairs(parts.uri.query());
        let token = query_get(&pairs, "connect_session_token")
            .or_else(|| query_get(&pairs, "session_token"))
            .map(str::to_owned)
            .or_else(|| bearer(parts).filter(|t| t.starts_with(CONNECT_TOKEN_PREFIX)).map(str::to_owned))
            .ok_or(ApiError(Error::InvalidConnectSession))?;
        Ok(Self(token))
    }
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim).filter(|v| !v.is_empty())
}

/// The connection of a request, from `Connection-Id` and `Provider-Config-Key`.
fn connection_from_headers(headers: &HeaderMap) -> ApiResult<ConnectionKey> {
    let id = header_str(headers, "connection-id").ok_or_else(|| ApiError(Error::MissingField("Connection-Id".to_owned())))?;
    let key = header_str(headers, "provider-config-key")
        .ok_or_else(|| ApiError(Error::MissingField("Provider-Config-Key".to_owned())))?;
    Ok(ConnectionKey::new(key, id))
}

// ----------------------------------------------------------------- shapes

pub(crate) fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

fn end_user_json(user: Option<&EndUser>, org: Option<&Organization>) -> Value {
    let Some(u) = user else {
        return Value::Null;
    };
    let org = org.or(u.organization.as_ref());
    json!({
        "id": u.id,
        "email": u.email,
        "display_name": u.display_name,
        "tags": u.tags,
        "organization": org.map(|o| json!({"id": o.id, "display_name": o.display_name})),
    })
}

fn integration_json(i: &Integration, provider_mode: Option<&str>, credentials: bool) -> Value {
    let mut v = json!({
        "unique_key": i.unique_key,
        "provider": i.provider,
        "display_name": i.display_name,
        "created_at": rfc3339(i.created_at),
        "updated_at": rfc3339(i.updated_at),
    });
    if credentials {
        v["credentials"] = json!({
            "type": provider_mode,
            "client_id": i.client_id,
            "scopes": i.scopes.join(","),
        });
    }
    v
}

fn summary_json(c: &ConnectionSummary) -> Value {
    json!({
        "id": c.id,
        "connection_id": c.connection_id,
        "provider_config_key": c.provider_config_key,
        "provider": c.provider,
        "created": rfc3339(c.created_at),
        "metadata": c.metadata,
        "end_user": end_user_json(c.end_user.as_ref(), None),
        "tags": c.tags,
        "errors": if c.refresh_exhausted { json!([{"type": "auth", "log_id": ""}]) } else { json!([]) },
    })
}

fn connection_json(c: &Connection) -> Value {
    json!({
        "id": c.id,
        "connection_id": c.connection_id,
        "provider_config_key": c.provider_config_key,
        "provider": c.provider,
        "connection_config": c.connection_config,
        "metadata": c.metadata,
        "created_at": rfc3339(c.created_at),
        "updated_at": rfc3339(c.updated_at),
        "last_fetched_at": c.last_fetched_at.map(rfc3339),
        "credentials": c.credentials,
        "end_user": end_user_json(c.end_user.as_ref(), None),
        "tags": c.tags,
        "errors": [],
        "webhook_url_override": c.webhook_url_override,
    })
}

fn session_json(s: &ConnectSession) -> Value {
    json!({
        "allowed_integrations": s.allowed_integrations,
        "integrations_config_defaults": s.integrations_config_defaults,
        "endUser": end_user_json(s.end_user.as_ref(), s.organization.as_ref()),
        "isReconnecting": s.reconnect.is_some(),
        "tags": s.tags,
        "webhook_url_override": s.webhook_url_override,
        "expires_at": rfc3339(s.expires_at),
    })
}

fn created_json(c: &ConnectSessionCreated) -> Response {
    (
        StatusCode::CREATED,
        axum::Json(json!({"data": {"token": c.token.expose(), "connect_link": c.connect_link, "expires_at": rfc3339(c.expires_at)}})),
    )
        .into_response()
}

fn auth_result(c: &Connection) -> axum::Json<Value> {
    axum::Json(json!({"connectionId": c.connection_id, "providerConfigKey": c.provider_config_key}))
}

// --------------------------------------------------------------- handlers

async fn health() -> axum::Json<Value> {
    axum::Json(json!({"status": "ok"}))
}

async fn list_providers(_env: Env, State(san): State<SanMateo>) -> axum::Json<Value> {
    let list: Vec<Value> = san
        .catalog()
        .iter()
        .map(|p| {
            json!({
                "name": p.name,
                "display_name": p.display_name,
                "auth_mode": p.auth_mode,
                "categories": p.categories,
                "docs": p.docs,
                "supported": p.auth_mode.is_supported(),
            })
        })
        .collect();
    axum::Json(json!({ "data": list }))
}

async fn get_provider(_env: Env, State(san): State<SanMateo>, Path(name): Path<String>) -> ApiResult<axum::Json<Value>> {
    let p = san.catalog().require(&name)?;
    Ok(axum::Json(json!({ "data": &*p })))
}

#[derive(Deserialize)]
struct IntegrationCredentials {
    #[serde(rename = "type", default)]
    _kind: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    client_secret: Option<Secret>,
    #[serde(default)]
    scopes: Option<String>,
}

fn split_scopes(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

#[derive(Deserialize)]
struct CreateIntegrationBody {
    unique_key: String,
    provider: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    credentials: Option<IntegrationCredentials>,
    #[serde(default)]
    custom: BTreeMap<String, String>,
}

fn mode_of(san: &SanMateo, provider: &str) -> Option<String> {
    san.catalog().get(provider).map(|p| p.auth_mode.as_str().to_owned())
}

async fn list_integrations(Env(env): Env, State(san): State<SanMateo>) -> ApiResult<axum::Json<Value>> {
    let list: Vec<Value> = san
        .list_integrations(&env)
        .await?
        .iter()
        .map(|i| integration_json(i, None, false))
        .collect();
    Ok(axum::Json(json!({ "data": list })))
}

async fn create_integration(Env(env): Env, State(san): State<SanMateo>, body: Bytes) -> ApiResult<axum::Json<Value>> {
    let b: CreateIntegrationBody = body_json(&body)?;
    let creds = b.credentials.unwrap_or(IntegrationCredentials { _kind: None, client_id: None, client_secret: None, scopes: None });
    let i = san
        .create_integration(
            &env,
            NewIntegration {
                unique_key: b.unique_key,
                provider: b.provider,
                display_name: b.display_name,
                client_id: creds.client_id,
                client_secret: creds.client_secret,
                scopes: creds.scopes.as_deref().map(split_scopes).unwrap_or_default(),
                custom: b.custom,
            },
        )
        .await?;
    Ok(axum::Json(json!({ "data": integration_json(&i, None, false) })))
}

async fn get_integration(
    Env(env): Env,
    State(san): State<SanMateo>,
    Path(key): Path<String>,
    RawQuery(q): RawQuery,
) -> ApiResult<axum::Json<Value>> {
    let i = san.get_integration(&env, &key).await?;
    let pairs = query_pairs(q.as_deref());
    let include = pairs.iter().any(|(k, v)| k.starts_with("include") && v.split(',').any(|x| x == "credentials"));
    Ok(axum::Json(json!({ "data": integration_json(&i, mode_of(&san, &i.provider).as_deref(), include) })))
}

#[derive(Deserialize)]
struct UpdateIntegrationBody {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    credentials: Option<IntegrationCredentials>,
    #[serde(default)]
    custom: Option<BTreeMap<String, String>>,
}

async fn update_integration(
    Env(env): Env,
    State(san): State<SanMateo>,
    Path(key): Path<String>,
    body: Bytes,
) -> ApiResult<axum::Json<Value>> {
    let b: UpdateIntegrationBody = body_json(&body)?;
    let (client_id, client_secret, scopes) = match b.credentials {
        Some(c) => (c.client_id, c.client_secret, c.scopes.as_deref().map(split_scopes)),
        None => (None, None, None),
    };
    let i = san
        .update_integration(&env, &key, IntegrationPatch { display_name: b.display_name, client_id, client_secret, scopes, custom: b.custom })
        .await?;
    Ok(axum::Json(json!({ "data": integration_json(&i, None, false) })))
}

async fn delete_integration(Env(env): Env, State(san): State<SanMateo>, Path(key): Path<String>) -> ApiResult<axum::Json<Value>> {
    san.delete_integration(&env, &key).await?;
    Ok(axum::Json(json!({"success": true})))
}

#[derive(Deserialize, Default)]
struct EndUserBody {
    id: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    tags: Tags,
}

#[derive(Deserialize, Default)]
struct OrganizationBody {
    id: String,
    #[serde(default)]
    display_name: Option<String>,
}

#[derive(Deserialize, Default)]
struct SessionBody {
    #[serde(default)]
    end_user: Option<EndUserBody>,
    #[serde(default)]
    organization: Option<OrganizationBody>,
    #[serde(default)]
    allowed_integrations: Vec<String>,
    #[serde(default)]
    integrations_config_defaults: BTreeMap<String, IntegrationDefaults>,
    #[serde(default)]
    tags: Tags,
    #[serde(default)]
    webhook_url_override: Option<String>,
    #[serde(default)]
    connection_id: Option<String>,
    #[serde(default)]
    integration_id: Option<String>,
}

impl SessionBody {
    fn into_input(self) -> ConnectSessionInput {
        let organization = self.organization.map(|o| Organization { id: o.id, display_name: o.display_name });
        ConnectSessionInput {
            end_user: self.end_user.map(|u| EndUser {
                id: u.id,
                email: u.email,
                display_name: u.display_name,
                tags: u.tags,
                organization: organization.clone(),
            }),
            organization,
            allowed_integrations: self.allowed_integrations,
            integrations_config_defaults: self.integrations_config_defaults,
            tags: self.tags,
            webhook_url_override: self.webhook_url_override,
        }
    }
}

async fn create_session(Env(env): Env, State(san): State<SanMateo>, body: Bytes) -> ApiResult<Response> {
    let b: Option<SessionBody> = body_json(&body)?;
    let created = san.create_connect_session(&env, b.unwrap_or_default().into_input()).await?;
    Ok(created_json(&created))
}

async fn create_reconnect_session(Env(env): Env, State(san): State<SanMateo>, body: Bytes) -> ApiResult<Response> {
    let b: SessionBody = body_json(&body)?;
    let connection_id = b.connection_id.clone().ok_or_else(|| ApiError(Error::MissingField("connection_id".to_owned())))?;
    let integration_id = b.integration_id.clone().ok_or_else(|| ApiError(Error::MissingField("integration_id".to_owned())))?;
    let key = ConnectionKey::new(integration_id, connection_id);
    let created = san.create_reconnect_session(&env, &key, b.into_input()).await?;
    Ok(created_json(&created))
}

async fn get_session(State(san): State<SanMateo>, SessionToken(token): SessionToken) -> ApiResult<axum::Json<Value>> {
    let s = san.connect_session(&token).await?;
    Ok(axum::Json(json!({ "data": session_json(&s) })))
}

async fn delete_session(State(san): State<SanMateo>, SessionToken(token): SessionToken) -> ApiResult<StatusCode> {
    san.delete_connect_session(&token).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Reads `params[x]`, `authorization_params[x]` and `user_scope` from a query.
fn connect_options(pairs: &[(String, String)]) -> ConnectOptions {
    let mut o = ConnectOptions::default();
    for (k, v) in pairs {
        if let Some(name) = k.strip_prefix("params[").and_then(|s| s.strip_suffix(']')) {
            o.connection_config.insert(name.to_owned(), json!(v));
        } else if let Some(name) = k.strip_prefix("authorization_params[").and_then(|s| s.strip_suffix(']')) {
            o.authorization_params.insert(name.to_owned(), v.clone());
        } else if k == "user_scope" {
            o.user_scopes = Some(v.clone());
        }
    }
    o
}

async fn oauth_connect(
    State(san): State<SanMateo>,
    SessionToken(token): SessionToken,
    Path(key): Path<String>,
    RawQuery(q): RawQuery,
) -> ApiResult<Response> {
    let url = san.oauth_start(&token, &key, connect_options(&query_pairs(q.as_deref()))).await?;
    Ok((StatusCode::FOUND, [(header::LOCATION, url)]).into_response())
}

/// Merges `connection_config` from a JSON body into the query options.
fn body_options(q: Option<&str>, body: &Map<String, Value>) -> ConnectOptions {
    let mut o = connect_options(&query_pairs(q));
    if let Some(Value::Object(cfg)) = body.get("connection_config").or_else(|| body.get("params")) {
        o.connection_config.extend(cfg.clone());
    }
    o
}

fn body_string(body: &Map<String, Value>, name: &str) -> Option<String> {
    body.get(name).and_then(Value::as_str).map(str::to_owned)
}

async fn connect_api_key(
    State(san): State<SanMateo>,
    SessionToken(token): SessionToken,
    Path(key): Path<String>,
    RawQuery(q): RawQuery,
    body: Bytes,
) -> ApiResult<axum::Json<Value>> {
    let b: Map<String, Value> = body_json::<Option<Map<String, Value>>>(&body)?.unwrap_or_default();
    let api_key = body_string(&b, "apiKey").or_else(|| body_string(&b, "api_key")).unwrap_or_default();
    let c = san.connect_api_key(&token, &key, Secret::new(api_key), body_options(q.as_deref(), &b)).await?;
    Ok(auth_result(&c))
}

async fn connect_basic(
    State(san): State<SanMateo>,
    SessionToken(token): SessionToken,
    Path(key): Path<String>,
    RawQuery(q): RawQuery,
    body: Bytes,
) -> ApiResult<axum::Json<Value>> {
    let b: Map<String, Value> = body_json::<Option<Map<String, Value>>>(&body)?.unwrap_or_default();
    let c = san
        .connect_basic(
            &token,
            &key,
            body_string(&b, "username").unwrap_or_default(),
            Secret::new(body_string(&b, "password").unwrap_or_default()),
            body_options(q.as_deref(), &b),
        )
        .await?;
    Ok(auth_result(&c))
}

async fn connect_client_credentials(
    State(san): State<SanMateo>,
    SessionToken(token): SessionToken,
    Path(key): Path<String>,
    RawQuery(q): RawQuery,
    body: Bytes,
) -> ApiResult<axum::Json<Value>> {
    let b: Map<String, Value> = body_json::<Option<Map<String, Value>>>(&body)?.unwrap_or_default();
    let c = san
        .connect_client_credentials(
            &token,
            &key,
            body_string(&b, "client_id").unwrap_or_default(),
            Secret::new(body_string(&b, "client_secret").unwrap_or_default()),
            body_options(q.as_deref(), &b),
        )
        .await?;
    Ok(auth_result(&c))
}

async fn connect_unauthenticated(
    State(san): State<SanMateo>,
    SessionToken(token): SessionToken,
    Path(key): Path<String>,
    RawQuery(q): RawQuery,
    body: Bytes,
) -> ApiResult<axum::Json<Value>> {
    let b: Map<String, Value> = body_json::<Option<Map<String, Value>>>(&body)?.unwrap_or_default();
    let c = san.connect_unauthenticated(&token, &key, body_options(q.as_deref(), &b)).await?;
    Ok(auth_result(&c))
}

async fn list_connections(Env(env): Env, State(san): State<SanMateo>, RawQuery(q): RawQuery) -> ApiResult<axum::Json<Value>> {
    let pairs = query_pairs(q.as_deref());
    let get = |names: &[&str]| names.iter().find_map(|n| query_get(&pairs, n)).map(str::to_owned);
    let filter = ConnectionFilter {
        provider_config_key: get(&["integrationId", "provider_config_key"]),
        connection_id: get(&["connectionId", "connection_id"]),
        end_user_id: get(&["endUserId", "end_user_id"]),
    };
    let list: Vec<Value> = san.list_connections(&env, &filter).await?.iter().map(summary_json).collect();
    Ok(axum::Json(json!({ "connections": list })))
}

fn key_from_query(id: String, q: Option<&str>) -> ApiResult<(ConnectionKey, Vec<(String, String)>)> {
    let pairs = query_pairs(q);
    let key = query_get(&pairs, "provider_config_key")
        .ok_or_else(|| ApiError(Error::MissingField("provider_config_key".to_owned())))?
        .to_owned();
    Ok((ConnectionKey::new(key, id), pairs))
}

fn query_flag(pairs: &[(String, String)], name: &str) -> bool {
    query_get(pairs, name).is_some_and(|v| v == "true" || v == "1")
}

async fn get_connection(
    Env(env): Env,
    State(san): State<SanMateo>,
    Path(id): Path<String>,
    RawQuery(q): RawQuery,
) -> ApiResult<axum::Json<Value>> {
    let (key, pairs) = key_from_query(id, q.as_deref())?;
    let options = GetOptions { refresh_if_needed: true, force_refresh: query_flag(&pairs, "force_refresh") };
    let c = san.get_connection(&env, &key, options).await?;
    Ok(axum::Json(connection_json(&c)))
}

#[derive(Deserialize)]
struct PatchConnectionBody {
    #[serde(default)]
    end_user: Option<EndUserBody>,
    #[serde(default)]
    tags: Option<Tags>,
    #[serde(default, deserialize_with = "double_option")]
    webhook_url_override: Option<Option<String>>,
}

/// Tells a missing field (`None`) from a JSON `null` (`Some(None)`).
fn double_option<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

async fn patch_connection(
    Env(env): Env,
    State(san): State<SanMateo>,
    Path(id): Path<String>,
    RawQuery(q): RawQuery,
    body: Bytes,
) -> ApiResult<axum::Json<Value>> {
    let (key, _) = key_from_query(id, q.as_deref())?;
    let b: PatchConnectionBody = body_json(&body)?;
    let patch = ConnectionPatch {
        end_user: b.end_user.map(|u| EndUser { id: u.id, email: u.email, display_name: u.display_name, tags: u.tags, organization: None }),
        tags: b.tags,
        webhook_url_override: b.webhook_url_override,
    };
    san.patch_connection(&env, &key, patch).await?;
    Ok(axum::Json(json!({"success": true})))
}

async fn delete_connection(
    Env(env): Env,
    State(san): State<SanMateo>,
    Path(id): Path<String>,
    RawQuery(q): RawQuery,
) -> ApiResult<StatusCode> {
    let (key, _) = key_from_query(id, q.as_deref())?;
    san.delete_connection(&env, &key).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ImportBody {
    #[serde(default)]
    connection_id: Option<String>,
    provider_config_key: String,
    credentials: Credentials,
    #[serde(default)]
    connection_config: Map<String, Value>,
    #[serde(default)]
    metadata: Map<String, Value>,
    #[serde(default)]
    tags: Tags,
    #[serde(default)]
    end_user: Option<EndUserBody>,
    #[serde(default)]
    webhook_url_override: Option<String>,
}

async fn import_connection(Env(env): Env, State(san): State<SanMateo>, body: Bytes) -> ApiResult<axum::Json<Value>> {
    let b: ImportBody = body_json(&body)?;
    let c = san
        .import_connection(
            &env,
            ImportConnection {
                connection_id: b.connection_id,
                provider_config_key: b.provider_config_key,
                credentials: b.credentials,
                connection_config: b.connection_config,
                metadata: b.metadata,
                tags: b.tags,
                end_user: b.end_user.map(|u| EndUser { id: u.id, email: u.email, display_name: u.display_name, tags: u.tags, organization: None }),
                webhook_url_override: b.webhook_url_override,
            },
        )
        .await?;
    Ok(axum::Json(connection_json(&c)))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct MetadataBody {
    connection_id: OneOrMany,
    provider_config_key: String,
    metadata: Map<String, Value>,
}

async fn metadata(env: &str, san: &SanMateo, body: &Bytes, merge: bool) -> ApiResult<axum::Json<Value>> {
    let b: MetadataBody = body_json(body)?;
    let ids = match &b.connection_id {
        OneOrMany::One(id) => vec![id.clone()],
        OneOrMany::Many(ids) => ids.clone(),
    };
    for id in &ids {
        san.set_metadata(env, &ConnectionKey::new(&b.provider_config_key, id), b.metadata.clone(), merge)
            .await?;
    }
    let connection_id = match b.connection_id {
        OneOrMany::One(id) => json!(id),
        OneOrMany::Many(ids) => json!(ids),
    };
    Ok(axum::Json(json!({"connection_id": connection_id, "provider_config_key": b.provider_config_key, "metadata": b.metadata})))
}

async fn set_metadata(Env(env): Env, State(san): State<SanMateo>, body: Bytes) -> ApiResult<axum::Json<Value>> {
    metadata(&env, &san, &body, false).await
}

async fn update_metadata(Env(env): Env, State(san): State<SanMateo>, body: Bytes) -> ApiResult<axum::Json<Value>> {
    metadata(&env, &san, &body, true).await
}

/// Caller headers that go upstream with no prefix.
const PASS_HEADERS: &[&str] = &["content-type"];

async fn proxy(
    Env(env): Env,
    State(san): State<SanMateo>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Response> {
    let key = connection_from_headers(&headers)?;
    let path_and_query = uri.path_and_query().map_or_else(|| uri.path().to_owned(), |p| p.as_str().to_owned());
    let endpoint = path_and_query.strip_prefix("/proxy").unwrap_or(&path_and_query).to_owned();
    let pairs: Vec<(String, String)> = headers
        .iter()
        .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().to_owned(), v.to_owned())))
        .collect();
    let mut forwarded = crate::proxy::forwarded_headers(&pairs);
    for name in PASS_HEADERS {
        if let Some(v) = header_str(&headers, name) {
            if !forwarded.iter().any(|(k, _)| k == name) {
                forwarded.push(((*name).to_owned(), v.to_owned()));
            }
        }
    }
    let retries = match header_str(&headers, "retries") {
        Some(v) => v.parse::<u32>().map_err(|_| invalid("Retries must be a whole number"))?,
        None => 0,
    };
    let retry_on = match header_str(&headers, "retry-on") {
        Some(v) => v
            .split(',')
            .map(|s| s.trim().parse::<u16>().map_err(|_| invalid("Retry-On must be a list of status codes")))
            .collect::<ApiResult<Vec<u16>>>()?,
        None => Vec::new(),
    };
    let request = ProxyRequest {
        method: method.as_str().to_owned(),
        endpoint,
        headers: forwarded,
        body: body.to_vec(),
        retries,
        retry_on,
        base_url_override: header_str(&headers, "base-url-override").map(str::to_owned),
        params: Vec::new(),
    };
    let reply = san.proxy(&env, &key, request).await?;
    let mut res = Response::new(Body::from(reply.body));
    *res.status_mut() = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::BAD_GATEWAY);
    for (k, v) in reply.headers {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(&v)) {
            res.headers_mut().append(name, value);
        }
    }
    Ok(res)
}

async fn list_records(Env(env): Env, State(san): State<SanMateo>, headers: HeaderMap, RawQuery(q): RawQuery) -> ApiResult<axum::Json<Value>> {
    let key = connection_from_headers(&headers)?;
    let pairs = query_pairs(q.as_deref());
    let model = query_get(&pairs, "model").ok_or_else(|| ApiError(Error::MissingField("model".to_owned())))?;
    let limit = query_get(&pairs, "limit")
        .map(|l| l.parse::<u32>().map_err(|_| invalid("limit must be a whole number")))
        .transpose()?;
    let ids = pairs
        .iter()
        .filter(|(k, _)| k == "ids" || k == "ids[]")
        .flat_map(|(_, v)| v.split(',').map(str::to_owned).collect::<Vec<_>>())
        .filter(|s| !s.is_empty())
        .collect();
    let request = RecordsRequest {
        model: model.to_owned(),
        cursor: query_get(&pairs, "cursor").map(str::to_owned),
        modified_after: query_get(&pairs, "modified_after").or_else(|| query_get(&pairs, "delta")).map(str::to_owned),
        limit,
        filter: query_get(&pairs, "filter").map(str::to_owned),
        ids,
    };
    let page = san.list_records(&env, &key, request).await?;
    Ok(axum::Json(json!({"records": page.records, "next_cursor": page.next_cursor})))
}

#[derive(Deserialize)]
struct SyncsBody {
    #[serde(default)]
    syncs: Vec<Value>,
    provider_config_key: String,
    #[serde(default)]
    connection_id: Option<String>,
}

impl SyncsBody {
    fn names(&self) -> ApiResult<Vec<String>> {
        self.syncs
            .iter()
            .map(|s| match s {
                Value::String(n) => Ok(n.clone()),
                Value::Object(o) => o
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("a sync needs a name")),
                _ => Err(invalid("a sync is a name or an object with a name")),
            })
            .collect()
    }
}

async fn trigger_syncs(Env(env): Env, State(san): State<SanMateo>, body: Bytes) -> ApiResult<axum::Json<Value>> {
    let b: SyncsBody = body_json(&body)?;
    san.trigger_syncs(&env, &b.provider_config_key, b.connection_id.as_deref(), &b.names()?).await?;
    Ok(axum::Json(json!({"success": true})))
}

async fn pause_syncs(Env(env): Env, State(san): State<SanMateo>, body: Bytes) -> ApiResult<axum::Json<Value>> {
    let b: SyncsBody = body_json(&body)?;
    san.set_sync_paused(&env, &b.provider_config_key, b.connection_id.as_deref(), &b.names()?, true).await?;
    Ok(axum::Json(json!({"success": true})))
}

async fn start_syncs(Env(env): Env, State(san): State<SanMateo>, body: Bytes) -> ApiResult<axum::Json<Value>> {
    let b: SyncsBody = body_json(&body)?;
    san.set_sync_paused(&env, &b.provider_config_key, b.connection_id.as_deref(), &b.names()?, false).await?;
    Ok(axum::Json(json!({"success": true})))
}

fn frequency_text(d: std::time::Duration) -> String {
    let s = d.as_secs();
    match s {
        s if s % 86_400 == 0 => format!("every {}d", s / 86_400),
        s if s % 3_600 == 0 => format!("every {}h", s / 3_600),
        s if s % 60 == 0 => format!("every {}m", s / 60),
        s => format!("every {s}s"),
    }
}

fn status_json(e: &SyncStatusEntry) -> Value {
    json!({
        "id": format!("{}:{}", e.connection_id, e.name),
        "connection_id": e.connection_id,
        "name": e.name,
        "status": e.status,
        "type": "INCREMENTAL",
        "frequency": frequency_text(e.frequency),
        "finishedAt": e.finished_at.map(rfc3339),
        "nextScheduledSyncAt": e.next_scheduled_at.map(rfc3339),
        "latestResult": e.latest_result,
    })
}

async fn sync_status(Env(env): Env, State(san): State<SanMateo>, RawQuery(q): RawQuery) -> ApiResult<axum::Json<Value>> {
    let pairs = query_pairs(q.as_deref());
    let key = query_get(&pairs, "provider_config_key").ok_or_else(|| ApiError(Error::MissingField("provider_config_key".to_owned())))?;
    let syncs: Vec<String> = query_get(&pairs, "syncs")
        .map(|s| s.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    let list = san.sync_status(&env, key, query_get(&pairs, "connection_id"), &syncs).await?;
    Ok(axum::Json(json!({"syncs": list.iter().map(status_json).collect::<Vec<_>>()})))
}

#[derive(Deserialize)]
struct ActionBody {
    action_name: String,
    #[serde(default)]
    input: Value,
}

async fn trigger_action(Env(env): Env, State(san): State<SanMateo>, headers: HeaderMap, body: Bytes) -> ApiResult<axum::Json<Value>> {
    let key = connection_from_headers(&headers)?;
    let b: ActionBody = body_json(&body)?;
    Ok(axum::Json(san.run_action(&env, &key, &b.action_name, b.input).await?))
}
