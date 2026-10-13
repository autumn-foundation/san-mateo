//! The proxy send loop and pagination. See ADR 0004.

use std::time::Duration;

use serde_json::Value;

use super::connections::RefreshMode;
use super::{Connection, GetOptions, SanMateo, invalid};
use crate::error::{Error, Result};
use crate::functions::PaginateOptions;
use crate::model::ConnectionKey;
use crate::paginate::Paginator;
use crate::policy::{self, Outcome, RetryRule, Step};
use crate::provider::Provider;
use crate::proxy::{self as px, ProxyRequest, ProxyResponse};
use crate::transport::{HttpRequest, TransportError};

/// The default page limit of [`SanMateo::paginate`].
const DEFAULT_MAX_PAGES: u32 = 100;

/// A request after the URL and header rules.
struct Prepared {
    url: String,
    headers: Vec<(String, String)>,
}

impl SanMateo {
    fn prepare(&self, conn: &Connection, provider: &Provider, request: &ProxyRequest) -> Result<Prepared> {
        let vars = Self::vars_for(&conn.credentials, &conn.connection_config, &conn.metadata);
        let provider_base = provider
            .base_url()
            .map(|b| crate::template::render_url(b, &vars))
            .transpose();
        let (base, is_override) = match &request.base_url_override {
            Some(b) => (b.clone(), true),
            None => (
                provider_base
                    .clone()?
                    .ok_or_else(|| invalid(format!("provider {} has no proxy base URL", provider.name)))?,
                false,
            ),
        };
        let provider_host = provider_base.ok().flatten().as_deref().and_then(px::host_of);
        let mut query = px::provider_query(provider, &vars)?;
        query.extend(request.params.iter().cloned());
        let url = px::add_query(&px::join_url(&base, &request.endpoint)?, &query)?;
        px::check_upstream(&url, provider_host.as_deref(), is_override, &self.inner.config.proxy)?;
        let headers = px::merge_headers(px::provider_headers(provider, &vars)?, px::auth_headers(&conn.credentials, provider));
        let headers = px::strip_hop_by_hop(px::merge_headers(headers, request.headers.clone()));
        Ok(Prepared { url, headers })
    }

    fn can_refresh(conn: &Connection) -> bool {
        match &conn.credentials {
            crate::Credentials::OAuth2(o) => o.refresh_token.is_some(),
            crate::Credentials::OAuth2ClientCredentials(_) => true,
            _ => false,
        }
    }

    /// Sends a call to the provider of a connection, with its auth.
    ///
    /// # Errors
    ///
    /// Connection errors, [`Error::ForbiddenHost`], [`Error::Upstream`],
    /// [`Error::UpstreamTooLarge`].
    pub async fn proxy(&self, env: &str, key: &ConnectionKey, request: ProxyRequest) -> Result<ProxyResponse> {
        let mut conn = self.get_connection(env, key, GetOptions::default()).await?;
        let provider = self.catalog().require(&conn.provider)?;
        let retry = provider.proxy.as_ref().and_then(|p| p.retry.as_ref());
        let mut prepared = self.prepare(&conn, &provider, &request)?;
        let idempotent = px::is_idempotent(&request.method);
        let rule = RetryRule {
            max_attempts: request.retries.saturating_add(1),
            initial_backoff_ms: self.inner.config.proxy.initial_backoff_ms,
            max_wait_ms: self.inner.config.proxy.max_wait_ms,
        };
        let mut refreshed = false;
        let mut attempt = 1_u32;
        loop {
            let mut send = HttpRequest::new(&request.method, prepared.url.clone()).body(request.body.clone());
            send.headers.clone_from(&prepared.headers);
            let result = self.inner.transport.send(send).await;
            let step = match &result {
                Ok(reply) => {
                    if reply.status == 401 && !refreshed && Self::can_refresh(&conn) {
                        refreshed = true;
                        if let Ok((row, creds)) = self.refresh(env, key, RefreshMode::Force).await {
                            conn = self.to_connection(row, creds);
                            prepared = self.prepare(&conn, &provider, &request)?;
                            continue;
                        }
                    }
                    let (outcome, opted_in) = px::classify(reply.status, &reply.headers, retry, &request.retry_on);
                    let wait = px::header_wait_ms(&reply.headers, retry, self.now_ms());
                    policy::decide(outcome, attempt, idempotent, opted_in, wait, &rule)
                }
                Err(TransportError::BodyTooLarge) => return Err(Error::UpstreamTooLarge),
                Err(TransportError::Invalid(m)) => return Err(invalid(format!("the upstream request is not valid: {m}"))),
                Err(_) => policy::decide(Outcome::NetworkError, attempt, idempotent, false, None, &rule),
            };
            match step {
                Step::Retry(wait_ms) => {
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    attempt += 1;
                }
                Step::Done | Step::GiveUp => {
                    return match result {
                        Ok(reply) => Ok(ProxyResponse {
                            status: reply.status,
                            headers: px::strip_hop_by_hop(reply.headers),
                            body: reply.body,
                        }),
                        Err(e) => Err(Error::Upstream(e.to_string())),
                    };
                }
            }
        }
    }

    /// Reads all pages of a list call and returns all records.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::proxy`], and [`Error::Invalid`] when there is no
    /// pagination config. [`Error::Upstream`] when a page is not a 2xx JSON
    /// reply.
    pub async fn paginate(
        &self,
        env: &str,
        key: &ConnectionKey,
        request: ProxyRequest,
        options: PaginateOptions,
    ) -> Result<Vec<Value>> {
        let row = self.connection_row(env, key).await?;
        let provider = self.catalog().require(&row.provider)?;
        let config = options
            .config
            .clone()
            .or_else(|| provider.proxy.as_ref().and_then(|p| p.paginate.clone()))
            .ok_or_else(|| invalid(format!("provider {} has no pagination config", provider.name)))?;
        let mut paginator = Paginator::new(config, options.limit);
        let max_pages = options.max_pages.unwrap_or(DEFAULT_MAX_PAGES).max(1);
        let mut page = request.clone();
        set_params(&mut page, &request.params, paginator.first().params);
        let mut records = Vec::new();
        for _ in 0..max_pages {
            let reply = self.proxy(env, key, page.clone()).await?;
            if !reply.is_success() {
                return Err(Error::Upstream(format!("a page reply has status {}", reply.status)));
            }
            let body: Value = reply.json()?;
            records.extend(paginator.records(&body));
            let Some(next) = paginator.advance(&reply.headers, &body) else {
                break;
            };
            match next.link {
                Some(link) => {
                    let (base, endpoint) = split_link(&link, page.base_url_override.as_deref(), provider.base_url())?;
                    page.base_url_override = base;
                    page.endpoint = endpoint;
                    page.params.clear();
                }
                None => set_params(&mut page, &request.params, next.params),
            }
        }
        Ok(records)
    }
}

/// Sets the query of a page: the caller params, with the page params in place
/// of params with the same name.
fn set_params(page: &mut ProxyRequest, caller: &[(String, String)], page_params: Vec<(String, String)>) {
    let mut params: Vec<(String, String)> = caller
        .iter()
        .filter(|(k, _)| !page_params.iter().any(|(p, _)| p == k))
        .cloned()
        .collect();
    params.extend(page_params);
    page.params = params;
}

/// Splits a next-page link into a base URL override and an endpoint.
///
/// An absolute link uses its own origin, so the proxy checks its host. A path
/// uses the origin of the current base.
fn split_link(link: &str, current_override: Option<&str>, provider_base: Option<&str>) -> Result<(Option<String>, String)> {
    let origin = |u: &url::Url| u.origin().ascii_serialization();
    if let Ok(abs) = url::Url::parse(link) {
        let mut endpoint = abs.path().to_owned();
        if let Some(q) = abs.query() {
            endpoint.push('?');
            endpoint.push_str(q);
        }
        return Ok((Some(origin(&abs)), endpoint));
    }
    if !link.starts_with('/') {
        return Err(invalid("a next-page link is not a URL or a path"));
    }
    let base = current_override
        .or(provider_base)
        .and_then(|b| url::Url::parse(b).ok())
        .map(|u| origin(&u));
    Ok((base, link.to_owned()))
}
