//! Connect sessions and auth flows: `OAuth2`, client credentials, API key,
//! Basic and no auth.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Duration;
use serde_json::{Map, Value, json};

use super::{
    CONNECT_TOKEN_PREFIX, ConnectOptions, ConnectSession, ConnectSessionCreated,
    ConnectSessionInput, Connection, ImportConnection, SanMateo, invalid,
};
use crate::credentials::{ClientCredentials, Credentials, OAuth2Credentials};
use crate::error::{Error, Result};
use crate::model::{ConnectionKey, IntegrationDefaults};
use crate::oauth2;
use crate::policy::session_live;
use crate::provider::{AuthMode, Provider};
use crate::secret::Secret;
use crate::store::{ConnectSessionRow, ConnectionRow, IntegrationRow, OAuthSessionRow};
use crate::template;
use crate::transport::HttpRequest;
use crate::webhook::{AuthEvent, AuthOperation, ErrorPayload, WebhookEvent};

/// The largest length of a connection id.
const MAX_CONNECTION_ID_LEN: usize = 255;

/// Authorization parameters that only San Mateo sets. An end user cannot set
/// them. Session defaults from the developer can.
const RESERVED_PARAMS: &[&str] = &[
    "client_id",
    "client_secret",
    "redirect_uri",
    "state",
    "scope",
    "response_type",
    "code_challenge",
    "code_challenge_method",
];

/// Providers that take user scopes in a `user_scope` parameter.
const USER_SCOPE_PROVIDERS: &[&str] = &["slack"];

/// Hosts that no safe host pattern accepts.
const PROBE_HOSTS: &[&str] = &["san-mateo-probe.invalid", "sanmateoprobe"];

/// Returns `true` when `pattern` rejects all probe hosts, so it fixes the
/// domain of a host value.
fn pattern_fixes_domain(pattern: &str) -> bool {
    regex::Regex::new(pattern).is_ok_and(|re| !PROBE_HOSTS.iter().any(|h| re.is_match(h)))
}

/// Returns the URL templates of a provider.
fn url_templates(provider: &Provider) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    out.extend(provider.authorization_url.as_deref());
    match &provider.token_url {
        Some(crate::provider::TokenUrl::One(u)) => out.push(u),
        Some(crate::provider::TokenUrl::ByMode(m)) => out.extend(m.values().map(String::as_str)),
        None => {}
    }
    out.extend(provider.refresh_url.as_deref());
    out.extend(provider.base_url());
    if let Some(v) = provider
        .proxy
        .as_ref()
        .and_then(|p| p.verification.as_ref())
    {
        out.extend(v.base_url_override.as_deref());
    }
    out
}

/// Checks one connection config value against its catalog schema.
fn check_field(name: &str, schema: &crate::provider::FieldSchema, value: &str) -> Result<()> {
    if let Some(pattern) = &schema.pattern {
        let re = regex::Regex::new(pattern)
            .map_err(|_| Error::Internal(format!("the catalog pattern of {name} is not valid")))?;
        if !re.is_match(value) {
            return Err(invalid(format!(
                "connection config {name} does not match its pattern"
            )));
        }
    }
    if !schema.allowed.is_empty() && !schema.allowed.iter().any(|a| a == value) {
        return Err(invalid(format!(
            "connection config {name} is not an allowed value"
        )));
    }
    Ok(())
}

/// Returns `true` for a valid connection id: 1 to 255 printable chars.
pub(crate) fn valid_connection_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_CONNECTION_ID_LEN && !id.chars().any(char::is_control)
}

fn to_session(row: ConnectSessionRow) -> ConnectSession {
    ConnectSession {
        id: row.id,
        environment: row.environment,
        end_user: row.end_user,
        organization: row.organization,
        allowed_integrations: row.allowed_integrations,
        integrations_config_defaults: row.integrations_config_defaults,
        tags: row.tags,
        webhook_url_override: row.webhook_url_override,
        reconnect: row.reconnect,
        expires_at: row.expires_at,
    }
}

/// The parts of a connect call after the session and integration checks.
struct Target {
    session: ConnectSession,
    integration: IntegrationRow,
    provider: Arc<Provider>,
    defaults: IntegrationDefaults,
}

impl Target {
    fn connection_id(&self, san: &SanMateo) -> String {
        self.session.reconnect.as_ref().map_or_else(
            || san.inner.entropy.uuid().to_string(),
            |k| k.connection_id.clone(),
        )
    }
}

impl SanMateo {
    /// Checks a webhook URL from a caller.
    pub(crate) fn check_webhook_url(&self, url: &str) -> Result<()> {
        let parsed = url::Url::parse(url).map_err(|_| invalid("the webhook URL is not valid"))?;
        match parsed.scheme() {
            "https" => {}
            "http" if self.inner.config.proxy.allow_plain_http => {}
            _ => return Err(invalid("the webhook URL must use https")),
        }
        // No internal hosts: a webhook carries a signed body.
        crate::proxy::check_upstream(url, None, false, &self.inner.config.proxy)
    }

    fn connect_link(&self, token: &str) -> String {
        let base = self.inner.config.connect_url.clone().unwrap_or_else(|| {
            format!(
                "{}/connect",
                self.inner.config.server_url.trim_end_matches('/')
            )
        });
        format!("{base}?session_token={token}")
    }

    async fn insert_session(
        &self,
        env: &str,
        input: ConnectSessionInput,
        reconnect: Option<ConnectionKey>,
    ) -> Result<ConnectSessionCreated> {
        self.environment(env)?;
        for key in &input.allowed_integrations {
            self.integration_row(env, key).await?;
        }
        if let Some(url) = &input.webhook_url_override {
            self.check_webhook_url(url)?;
        }
        let token = format!("{CONNECT_TOKEN_PREFIX}{}", self.inner.entropy.hex(32));
        let now = self.now();
        let ttl = Duration::from_std(self.inner.config.connect_session_ttl)
            .unwrap_or(Duration::minutes(30));
        let expires_at = now + ttl;
        let allowed = match &reconnect {
            Some(k) => vec![k.provider_config_key.clone()],
            None => input.allowed_integrations,
        };
        self.inner
            .store
            .insert_connect_session(ConnectSessionRow {
                id: self.inner.entropy.uuid(),
                environment: env.to_owned(),
                token_hash: self.inner.keys.lookup_hash(&token),
                end_user: input.end_user,
                organization: input.organization,
                allowed_integrations: allowed,
                integrations_config_defaults: input.integrations_config_defaults,
                tags: input.tags,
                webhook_url_override: input.webhook_url_override,
                reconnect,
                created_at: now,
                expires_at,
            })
            .await?;
        Ok(ConnectSessionCreated {
            connect_link: self.connect_link(&token),
            token: Secret::new(token),
            expires_at,
        })
    }

    /// Makes a connect session.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIntegration`] for an allowed integration that does not
    /// exist. [`Error::Invalid`] for a webhook URL that is not HTTPS, and
    /// [`Error::ForbiddenHost`] for an internal webhook host.
    pub async fn create_connect_session(
        &self,
        env: &str,
        input: ConnectSessionInput,
    ) -> Result<ConnectSessionCreated> {
        self.insert_session(env, input, None).await
    }

    /// Makes a connect session that reconnects one connection.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownConnection`] when the connection does not exist.
    pub async fn create_reconnect_session(
        &self,
        env: &str,
        key: &ConnectionKey,
        input: ConnectSessionInput,
    ) -> Result<ConnectSessionCreated> {
        self.connection_row(env, key).await?;
        self.insert_session(env, input, Some(key.clone())).await
    }

    /// Reads a live connect session.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConnectSession`] when the token is unknown, wrong or expired.
    pub async fn connect_session(&self, token: &str) -> Result<ConnectSession> {
        if !token.starts_with(CONNECT_TOKEN_PREFIX) {
            return Err(Error::InvalidConnectSession);
        }
        let row = self
            .inner
            .store
            .get_connect_session(&self.inner.keys.lookup_hash(token))
            .await?
            .ok_or(Error::InvalidConnectSession)?;
        if !session_live(self.now_ms(), crate::clock::to_ms(row.expires_at)) {
            return Err(Error::InvalidConnectSession);
        }
        Ok(to_session(row))
    }

    /// Deletes a connect session.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConnectSession`].
    pub async fn delete_connect_session(&self, token: &str) -> Result<()> {
        let s = self.connect_session(token).await?;
        self.inner.store.delete_connect_session(s.id).await?;
        Ok(())
    }

    /// Checks the session and the integration, and gives the defaults.
    async fn target(&self, token: &str, integration: &str, mode: &AuthMode) -> Result<Target> {
        let session = self.connect_session(token).await?;
        let allowed = match &session.reconnect {
            Some(k) => k.provider_config_key == integration,
            None => {
                session.allowed_integrations.is_empty()
                    || session
                        .allowed_integrations
                        .iter()
                        .any(|a| a == integration)
            }
        };
        if !allowed {
            return Err(Error::IntegrationNotAllowed(integration.to_owned()));
        }
        let row = self
            .integration_row(&session.environment, integration)
            .await?;
        let provider = self.catalog().require(&row.provider)?;
        if &provider.auth_mode != mode {
            return Err(Error::UnsupportedAuthMode(format!(
                "{} uses {}",
                integration,
                provider.auth_mode.as_str()
            )));
        }
        let defaults = session
            .integrations_config_defaults
            .get(integration)
            .cloned()
            .unwrap_or_default();
        Ok(Target {
            session,
            integration: row,
            provider,
            defaults,
        })
    }

    /// Merges and checks connection config: end-user values, then the
    /// session defaults (they win). See [`Self::check_user_hosts`] for the
    /// end-user values that can set a host.
    fn connection_config(
        &self,
        provider: &Provider,
        user: Map<String, Value>,
        defaults: &Map<String, Value>,
    ) -> Result<Map<String, Value>> {
        let user_keys: Vec<String> = user
            .keys()
            .filter(|k| !defaults.contains_key(*k))
            .cloned()
            .collect();
        let mut cfg = user;
        for (k, v) in defaults {
            cfg.insert(k.clone(), v.clone());
        }
        for (name, schema) in &provider.connection_config {
            let value = match cfg.get(name) {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if s.is_empty() => None,
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Number(n)) => Some(n.to_string()),
                Some(Value::Bool(b)) => Some(b.to_string()),
                Some(_) => return Err(invalid(format!("connection config {name} must be text"))),
            };
            let Some(value) = value else {
                let optional =
                    schema.optional.unwrap_or(false) || schema.automated.unwrap_or(false);
                if let Some(d) = &schema.default_value {
                    cfg.insert(name.clone(), json!(d));
                } else if !optional {
                    return Err(Error::MissingField(name.clone()));
                }
                continue;
            };
            check_field(name, schema, &value)?;
        }
        self.check_user_hosts(provider, &cfg, &user_keys)?;
        Ok(cfg)
    }

    /// Checks end-user values that can set the host of a provider URL. Else
    /// the client secret or the tokens can go to a host of the end user.
    ///
    /// Such a value needs one of: an `allowed` list, a `pattern` that fixes
    /// the domain, or a host in the allowed hosts. Values from the session
    /// defaults come from the developer. They need no check.
    fn check_user_hosts(
        &self,
        provider: &Provider,
        cfg: &Map<String, Value>,
        keys: &[String],
    ) -> Result<()> {
        let vars = json!({ "connectionConfig": cfg });
        let templates = url_templates(provider);
        for key in keys {
            let safe = provider.connection_config.get(key).is_some_and(|s| {
                !s.allowed.is_empty() || s.pattern.as_deref().is_some_and(pattern_fixes_domain)
            });
            if safe {
                continue;
            }
            let path = format!("connectionConfig.{key}");
            for t in &templates {
                if !template::value_sets_host(t, &vars, &path) {
                    continue;
                }
                let allowed = template::render_url(t, &vars)
                    .ok()
                    .and_then(|u| url::Url::parse(&u).ok())
                    .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
                    .is_some_and(|h| {
                        crate::proxy::host_allowed(&h, &self.inner.config.proxy.allowed_hosts)
                    });
                if !allowed {
                    return Err(invalid(format!(
                        "connection config {key} sets the host of a provider URL: set it in the session defaults or add the host to the allowed hosts"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Returns the scopes to ask for: the integration scopes, or the
    /// provider default scopes when the integration has none.
    pub(crate) fn requested_scopes(
        integration: &IntegrationRow,
        provider: &Provider,
    ) -> Vec<String> {
        if integration.scopes.is_empty() {
            provider.default_scopes.clone()
        } else {
            integration.scopes.clone()
        }
    }

    fn render_params(
        params: &BTreeMap<String, String>,
        vars: &Value,
    ) -> Result<BTreeMap<String, String>> {
        params
            .iter()
            .map(|(k, v)| Ok((k.clone(), template::render(v, vars)?)))
            .collect()
    }

    fn redirect_uri(&self, env: &str) -> Result<String> {
        Ok(self
            .environment(env)?
            .callback_url
            .clone()
            .unwrap_or_else(|| {
                format!(
                    "{}/oauth/callback",
                    self.inner.config.server_url.trim_end_matches('/')
                )
            }))
    }

    /// Starts an `OAuth2` flow. Returns the provider URL for the browser.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidConnectSession`], [`Error::IntegrationNotAllowed`],
    /// [`Error::UnknownIntegration`], [`Error::UnsupportedAuthMode`],
    /// [`Error::MissingField`], [`Error::Template`].
    pub async fn oauth_start(
        &self,
        token: &str,
        integration: &str,
        options: ConnectOptions,
    ) -> Result<String> {
        let t = self.target(token, integration, &AuthMode::OAuth2).await?;
        if let Some(k) = options
            .authorization_params
            .keys()
            .find(|k| RESERVED_PARAMS.contains(&k.as_str()))
        {
            return Err(invalid(format!(
                "the authorization parameter {k} is reserved"
            )));
        }
        let env = t.session.environment.clone();
        let mut cfg = self.connection_config(
            &t.provider,
            options.connection_config,
            &t.defaults.connection_config,
        )?;
        // As in Nango: only the session defaults set user scopes.
        let user_scope = t.defaults.user_scopes.clone().filter(|s| !s.is_empty());
        if let Some(us) = &user_scope {
            cfg.insert("user_scope".to_owned(), json!(us));
        }
        let vars = json!({ "connectionConfig": cfg });
        let auth_url = t.provider.authorization_url.as_deref().ok_or_else(|| {
            Error::Internal(format!(
                "provider {} has no authorization URL",
                t.provider.name
            ))
        })?;
        let auth_url = template::render_url(auth_url, &vars)?;
        let mut params = Self::render_params(&t.provider.authorization_params, &vars)?;
        params.extend(options.authorization_params);
        if let Some(us) =
            user_scope.filter(|_| USER_SCOPE_PROVIDERS.contains(&t.provider.name.as_str()))
        {
            params.insert("user_scope".to_owned(), us);
        }
        params.extend(t.defaults.authorization_params.clone());
        let client_id = t
            .integration
            .client_id
            .clone()
            .ok_or_else(|| Error::MissingField("client_id".to_owned()))?;
        let scopes = Self::requested_scopes(&t.integration, &t.provider);
        let redirect_uri = self.redirect_uri(&env)?;
        let state = self.random_token(32);
        let state_hash = self.inner.keys.lookup_hash(&state);
        let verifier = t.provider.uses_pkce().then(|| self.random_token(32));
        let challenge = verifier.as_deref().map(oauth2::pkce_challenge);
        let url = oauth2::authorize_url(&oauth2::AuthorizeRequest {
            authorization_url: &auth_url,
            client_id: &client_id,
            redirect_uri: &redirect_uri,
            scopes: &scopes,
            scope_separator: t.provider.scope_separator(),
            state: &state,
            code_challenge: challenge.as_deref(),
            params: &params,
        })?;
        let now = self.now();
        let ttl = Duration::from_std(self.inner.config.oauth_session_ttl)
            .unwrap_or(Duration::minutes(15));
        let code_verifier = verifier
            .map(|v| {
                self.inner
                    .keys
                    .seal(v.as_bytes(), &crate::crypto::aad(&["oauth", &state_hash]))
            })
            .transpose()?;
        self.inner
            .store
            .insert_oauth_session(OAuthSessionRow {
                connection_id: t.connection_id(self),
                state_hash,
                environment: env,
                provider_config_key: integration.to_owned(),
                connect_session_id: Some(t.session.id),
                code_verifier,
                connection_config: Value::Object(cfg),
                redirect_uri,
                created_at: now,
                // The flow ends with its connect session.
                expires_at: (now + ttl).min(t.session.expires_at),
            })
            .await?;
        Ok(url)
    }

    /// Sends an `auth` failure event for a connect call.
    fn auth_failure(
        &self,
        env: &str,
        key: &ConnectionKey,
        provider: &Provider,
        session: Option<&ConnectSession>,
        err: &Error,
    ) {
        let event = WebhookEvent::Auth(AuthEvent {
            connection_id: key.connection_id.clone(),
            provider_config_key: key.provider_config_key.clone(),
            provider: provider.name.clone(),
            auth_mode: provider.auth_mode.as_str().to_owned(),
            environment: env.to_owned(),
            operation: AuthOperation::Creation,
            success: false,
            error: Some(ErrorPayload {
                kind: err.code().to_owned(),
                description: err.to_string(),
            }),
            tags: session.map(|s| s.tags.clone()).unwrap_or_default(),
            end_user: session.and_then(|s| s.end_user.clone()),
        });
        self.spawn_webhook(
            env,
            session.and_then(|s| s.webhook_url_override.clone()),
            event,
        );
    }

    /// Ends an `OAuth2` flow with the callback query.
    ///
    /// # Errors
    ///
    /// [`Error::InvalidOAuthState`], [`Error::Provider`] when the provider
    /// gives an error or the code exchange fails.
    pub async fn oauth_callback(&self, query: &BTreeMap<String, String>) -> Result<Connection> {
        let state = query
            .get("state")
            .filter(|s| !s.is_empty())
            .ok_or(Error::InvalidOAuthState)?;
        let state_hash = self.inner.keys.lookup_hash(state);
        let row = self
            .inner
            .store
            .take_oauth_session(&state_hash)
            .await?
            .ok_or(Error::InvalidOAuthState)?;
        if !session_live(self.now_ms(), crate::clock::to_ms(row.expires_at)) {
            return Err(Error::InvalidOAuthState);
        }
        // The state is now used. Finish in a tracked task: a dropped request
        // must not lose the code exchange or the failure event.
        let this = self.clone();
        let query = query.clone();
        self.inner
            .tasks
            .spawn(async move { this.finish_callback(row, &query).await })
            .await
            .map_err(|_| Error::Internal("the OAuth callback task failed".to_owned()))?
    }

    async fn finish_callback(
        &self,
        row: OAuthSessionRow,
        query: &BTreeMap<String, String>,
    ) -> Result<Connection> {
        let env = row.environment.clone();
        let session = match row.connect_session_id {
            Some(id) => {
                // A deleted or expired connect session ends its flows.
                let s = self
                    .inner
                    .store
                    .get_connect_session_by_id(id)
                    .await?
                    .map(to_session)
                    .ok_or(Error::InvalidConnectSession)?;
                if !session_live(self.now_ms(), crate::clock::to_ms(s.expires_at)) {
                    return Err(Error::InvalidConnectSession);
                }
                Some(s)
            }
            None => None,
        };
        let integration = self.integration_row(&env, &row.provider_config_key).await?;
        let provider = self.catalog().require(&integration.provider)?;
        let key = ConnectionKey::new(&row.provider_config_key, &row.connection_id);
        match self
            .exchange_code(&row, &integration, &provider, query, session.as_ref())
            .await
        {
            Ok(conn) => Ok(conn),
            Err(e) => {
                self.auth_failure(&env, &key, &provider, session.as_ref(), &e);
                Err(e)
            }
        }
    }

    async fn exchange_code(
        &self,
        row: &OAuthSessionRow,
        integration: &IntegrationRow,
        provider: &Provider,
        query: &BTreeMap<String, String>,
        session: Option<&ConnectSession>,
    ) -> Result<Connection> {
        if let Some(error) = query.get("error") {
            let desc = query
                .get("error_description")
                .map(String::as_str)
                .unwrap_or_default();
            return Err(Error::Provider {
                code: "authorization_denied",
                status: None,
                message: if desc.is_empty() {
                    error.clone()
                } else {
                    format!("{error}: {desc}")
                },
            });
        }
        let code_param = provider
            .authorization_code_param_in_callback
            .as_deref()
            .unwrap_or("code");
        let code = query
            .get(code_param)
            .filter(|c| !c.is_empty())
            .ok_or_else(|| invalid("the callback has no authorization code"))?;
        let mut cfg = match &row.connection_config {
            Value::Object(m) => m.clone(),
            _ => Map::new(),
        };
        // Values from the callback can go into the token URL. The end user
        // controls them, so check them first.
        let mut from_callback = Vec::new();
        for name in &provider.redirect_uri_metadata {
            if let Some(v) = query.get(name).filter(|v| !v.is_empty()) {
                if let Some(schema) = provider.connection_config.get(name) {
                    check_field(name, schema, v)?;
                }
                cfg.insert(name.clone(), json!(v));
                from_callback.push(name.clone());
            }
        }
        self.check_user_hosts(provider, &cfg, &from_callback)?;
        let vars = json!({ "connectionConfig": cfg });
        let token_url = provider.token_url_for(&AuthMode::OAuth2).ok_or_else(|| {
            Error::Internal(format!("provider {} has no token URL", provider.name))
        })?;
        let token_url = template::render_url(token_url, &vars)?;
        crate::proxy::check_upstream(&token_url, None, false, &self.inner.config.proxy)?;
        let verifier = row
            .code_verifier
            .as_deref()
            .map(|sealed| {
                let bytes = self
                    .inner
                    .keys
                    .open(sealed, &crate::crypto::aad(&["oauth", &row.state_hash]))?;
                String::from_utf8(bytes).map_err(|_| Error::Crypto)
            })
            .transpose()?;
        let token_params = Self::render_params(&provider.token_params, &vars)?;
        let params = oauth2::code_exchange_params(
            code,
            &row.redirect_uri,
            verifier.as_deref(),
            &token_params,
        );
        let client_id = integration.client_id.clone().unwrap_or_default();
        let client_secret = self.client_secret(integration)?.unwrap_or_default();
        let request = oauth2::token_request(
            provider,
            &token_url,
            &client_id,
            client_secret.expose(),
            params,
        );
        let reply = self
            .token_call(request, provider, "token_exchange_failed")
            .await?;
        cfg.extend(oauth2::extract_metadata(
            &reply.raw,
            &provider.token_response_metadata,
        ));
        let creds = Credentials::OAuth2(OAuth2Credentials {
            access_token: reply.access_token,
            refresh_token: reply.refresh_token,
            expires_at: reply.expires_at,
            raw: reply.raw,
        });
        let key = ConnectionKey::new(&row.provider_config_key, &row.connection_id);
        self.save_connection(&row.environment, &key, provider, creds, cfg, session, None)
            .await
    }

    /// Sends a token request and parses the reply.
    pub(crate) async fn token_call(
        &self,
        request: HttpRequest,
        provider: &Provider,
        code: &'static str,
    ) -> Result<oauth2::TokenReply> {
        let reply = self
            .inner
            .transport
            .send(request)
            .await
            .map_err(|e| Error::Provider {
                code,
                status: None,
                message: e.to_string(),
            })?;
        if !(200..300).contains(&reply.status) {
            return Err(Error::Provider {
                code,
                status: Some(reply.status),
                message: oauth2::reply_error(&reply.body)
                    .unwrap_or_else(|| format!("HTTP {}", reply.status)),
            });
        }
        oauth2::parse_token_reply(
            &reply.body,
            provider.alternate_access_token_response_path.as_deref(),
            provider.expires_in_ms(),
            self.now(),
            code,
        )
    }

    /// Stores a connection and sends the `auth` event.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn save_connection(
        &self,
        env: &str,
        key: &ConnectionKey,
        provider: &Provider,
        credentials: Credentials,
        connection_config: Map<String, Value>,
        session: Option<&ConnectSession>,
        import: Option<&ImportConnection>,
    ) -> Result<Connection> {
        let now = self.now();
        let existing = self
            .inner
            .store
            .get_connection(env, &key.provider_config_key, &key.connection_id)
            .await?;
        let sealed = self.inner.keys.seal_json(
            &credentials,
            &Self::connection_aad(env, &key.provider_config_key, &key.connection_id),
        )?;
        let pick_tags = |new: Option<&crate::model::Tags>| {
            new.filter(|t| !t.is_empty())
                .cloned()
                .or_else(|| existing.as_ref().map(|e| e.tags.clone()))
                .unwrap_or_default()
        };
        // An import sets all fields. A connect call keeps the stored fields
        // that the session does not set.
        let (metadata, tags, end_user, webhook_url_override) = import.map_or_else(
            || {
                (
                    existing
                        .as_ref()
                        .map_or_else(|| json!({}), |e| e.metadata.clone()),
                    pick_tags(session.map(|s| &s.tags)),
                    session
                        .and_then(|s| s.end_user.clone())
                        .or_else(|| existing.as_ref().and_then(|e| e.end_user.clone())),
                    session
                        .and_then(|s| s.webhook_url_override.clone())
                        .or_else(|| {
                            existing
                                .as_ref()
                                .and_then(|e| e.webhook_url_override.clone())
                        }),
                )
            },
            |i| {
                (
                    Value::Object(i.metadata.clone()),
                    i.tags.clone(),
                    i.end_user.clone(),
                    i.webhook_url_override.clone(),
                )
            },
        );
        let row = ConnectionRow {
            id: self.inner.entropy.uuid(),
            environment: env.to_owned(),
            provider_config_key: key.provider_config_key.clone(),
            connection_id: key.connection_id.clone(),
            provider: provider.name.clone(),
            credentials: sealed,
            credentials_expires_at: credentials.expires_at(),
            connection_config: Value::Object(connection_config),
            metadata,
            tags,
            end_user,
            webhook_url_override,
            created_at: now,
            updated_at: now,
            last_fetched_at: None,
            last_refresh_success: None,
            last_refresh_failure: None,
            refresh_attempts: 0,
            refresh_exhausted: false,
            version: 0,
        };
        let up = self.inner.store.upsert_connection(row).await?;
        let conn = Self::to_connection(up.row, credentials);
        self.spawn_webhook(
            env,
            conn.webhook_url_override.clone(),
            WebhookEvent::Auth(AuthEvent {
                connection_id: conn.connection_id.clone(),
                provider_config_key: conn.provider_config_key.clone(),
                provider: conn.provider.clone(),
                auth_mode: provider.auth_mode.as_str().to_owned(),
                environment: env.to_owned(),
                operation: if up.created {
                    AuthOperation::Creation
                } else {
                    AuthOperation::Override
                },
                success: true,
                error: None,
                tags: conn.tags.clone(),
                end_user: conn.end_user.clone(),
            }),
        );
        Ok(conn)
    }

    /// Checks API key or Basic credentials with the provider `verification`.
    async fn verify_credentials(
        &self,
        provider: &Provider,
        credentials: &Credentials,
        cfg: &Map<String, Value>,
    ) -> Result<()> {
        let Some(check) = provider
            .proxy
            .as_ref()
            .and_then(|p| p.verification.as_ref())
        else {
            return Ok(());
        };
        if check.endpoints.is_empty() {
            return Ok(());
        }
        let vars = Self::vars_for(credentials, cfg, &Map::new());
        let base = match check
            .base_url_override
            .as_deref()
            .or_else(|| provider.base_url())
        {
            Some(b) => template::render_url(b, &vars)?,
            None => return Ok(()),
        };
        let mut headers = crate::proxy::merge_headers(
            crate::proxy::auth_headers(credentials, provider),
            crate::proxy::provider_headers(provider, &vars)?,
        );
        headers = crate::proxy::merge_headers(
            headers,
            check
                .headers
                .iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
                .collect(),
        );
        let body = check
            .data
            .as_ref()
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|e| Error::Internal(format!("verification data: {e}")))?;
        let mut last = None;
        for endpoint in &check.endpoints {
            let endpoint = template::render_url(endpoint, &vars)?;
            let url = crate::proxy::add_query(
                &crate::proxy::join_url(&base, &endpoint)?,
                &crate::proxy::provider_query(provider, &vars)?,
            )?;
            crate::proxy::check_upstream(&url, None, false, &self.inner.config.proxy)?;
            let mut req = HttpRequest::new(&check.method, url);
            req.headers.clone_from(&headers);
            if let Some(b) = &body {
                req = req
                    .header("content-type", "application/json")
                    .body(b.clone());
            }
            match self.inner.transport.send(req).await {
                Ok(r) if (200..300).contains(&r.status) => return Ok(()),
                Ok(r) => last = Some(r.status),
                Err(_) => last = None,
            }
        }
        Err(Error::Provider {
            code: "connection_test_failed",
            status: last,
            message: "the provider did not accept the credentials".to_owned(),
        })
    }

    async fn connect_with(
        &self,
        token: &str,
        integration: &str,
        mode: AuthMode,
        options: ConnectOptions,
        make: impl FnOnce(&Target) -> Result<Credentials>,
    ) -> Result<Connection> {
        let t = self.target(token, integration, &mode).await?;
        let cfg = self.connection_config(
            &t.provider,
            options.connection_config,
            &t.defaults.connection_config,
        )?;
        let creds = make(&t)?;
        self.verify_credentials(&t.provider, &creds, &cfg).await?;
        let key = ConnectionKey::new(integration, t.connection_id(self));
        let env = t.session.environment.clone();
        self.save_connection(&env, &key, &t.provider, creds, cfg, Some(&t.session), None)
            .await
    }

    /// Makes an API key connection.
    ///
    /// # Errors
    ///
    /// Session errors, and [`Error::Provider`] with `connection_test_failed`.
    pub async fn connect_api_key(
        &self,
        token: &str,
        integration: &str,
        api_key: Secret,
        options: ConnectOptions,
    ) -> Result<Connection> {
        self.connect_with(token, integration, AuthMode::ApiKey, options, |_| {
            if api_key.is_empty() {
                return Err(Error::MissingField("apiKey".to_owned()));
            }
            Ok(Credentials::ApiKey { api_key })
        })
        .await
    }

    /// Makes a Basic auth connection.
    ///
    /// # Errors
    ///
    /// Session errors, and [`Error::Provider`] with `connection_test_failed`.
    pub async fn connect_basic(
        &self,
        token: &str,
        integration: &str,
        username: String,
        password: Secret,
        options: ConnectOptions,
    ) -> Result<Connection> {
        self.connect_with(token, integration, AuthMode::Basic, options, |_| {
            if username.is_empty() {
                return Err(Error::MissingField("username".to_owned()));
            }
            Ok(Credentials::Basic { username, password })
        })
        .await
    }

    /// Makes a connection with no auth.
    ///
    /// # Errors
    ///
    /// Session errors.
    pub async fn connect_unauthenticated(
        &self,
        token: &str,
        integration: &str,
        options: ConnectOptions,
    ) -> Result<Connection> {
        self.connect_with(token, integration, AuthMode::None, options, |_| {
            Ok(Credentials::Unauthenticated)
        })
        .await
    }

    /// Gets a client credentials token.
    pub(crate) async fn client_credentials_token(
        &self,
        provider: &Provider,
        scopes: &[String],
        cfg: &Map<String, Value>,
        client_id: &str,
        client_secret: &Secret,
        code: &'static str,
    ) -> Result<ClientCredentials> {
        let vars = json!({ "connectionConfig": cfg });
        let url = provider
            .token_url_for(&AuthMode::OAuth2ClientCredentials)
            .ok_or_else(|| {
                Error::Internal(format!("provider {} has no token URL", provider.name))
            })?;
        let url = template::render_url(url, &vars)?;
        crate::proxy::check_upstream(&url, None, false, &self.inner.config.proxy)?;
        let token_params = Self::render_params(&provider.token_params, &vars)?;
        let params =
            oauth2::client_credentials_params(scopes, provider.scope_separator(), &token_params);
        let request =
            oauth2::token_request(provider, &url, client_id, client_secret.expose(), params);
        let reply = self.token_call(request, provider, code).await?;
        Ok(ClientCredentials {
            token: reply.access_token,
            expires_at: reply.expires_at,
            client_id: client_id.to_owned(),
            client_secret: client_secret.clone(),
            raw: reply.raw,
        })
    }

    /// Makes an `OAuth2` client credentials connection.
    ///
    /// # Errors
    ///
    /// Session errors, and [`Error::Provider`] with `token_exchange_failed`.
    pub async fn connect_client_credentials(
        &self,
        token: &str,
        integration: &str,
        client_id: String,
        client_secret: Secret,
        options: ConnectOptions,
    ) -> Result<Connection> {
        let t = self
            .target(token, integration, &AuthMode::OAuth2ClientCredentials)
            .await?;
        if client_id.is_empty() {
            return Err(Error::MissingField("client_id".to_owned()));
        }
        if client_secret.is_empty() {
            return Err(Error::MissingField("client_secret".to_owned()));
        }
        let cfg = self.connection_config(
            &t.provider,
            options.connection_config,
            &t.defaults.connection_config,
        )?;
        let scopes = Self::requested_scopes(&t.integration, &t.provider);
        let creds = self
            .client_credentials_token(
                &t.provider,
                &scopes,
                &cfg,
                &client_id,
                &client_secret,
                "token_exchange_failed",
            )
            .await?;
        let key = ConnectionKey::new(integration, t.connection_id(self));
        let env = t.session.environment.clone();
        self.save_connection(
            &env,
            &key,
            &t.provider,
            Credentials::OAuth2ClientCredentials(creds),
            cfg,
            Some(&t.session),
            None,
        )
        .await
    }

    /// Imports a connection with known credentials.
    ///
    /// # Errors
    ///
    /// [`Error::UnknownIntegration`], [`Error::Invalid`] when the credentials
    /// do not match the auth mode.
    pub async fn import_connection(
        &self,
        env: &str,
        input: ImportConnection,
    ) -> Result<Connection> {
        let integration = self
            .integration_row(env, &input.provider_config_key)
            .await?;
        let provider = self.catalog().require(&integration.provider)?;
        if input.credentials.auth_mode() != provider.auth_mode {
            return Err(invalid(format!(
                "the credentials are {} but the integration uses {}",
                input.credentials.auth_mode().as_str(),
                provider.auth_mode.as_str()
            )));
        }
        if let Some(url) = &input.webhook_url_override {
            self.check_webhook_url(url)?;
        }
        let connection_id = match &input.connection_id {
            Some(id) if valid_connection_id(id) => id.clone(),
            Some(_) => {
                return Err(invalid(
                    "the connection id must be 1 to 255 printable chars",
                ));
            }
            None => self.inner.entropy.uuid().to_string(),
        };
        let key = ConnectionKey::new(&input.provider_config_key, connection_id);
        self.save_connection(
            env,
            &key,
            &provider,
            input.credentials.clone(),
            input.connection_config.clone(),
            None,
            Some(&input),
        )
        .await
    }
}
