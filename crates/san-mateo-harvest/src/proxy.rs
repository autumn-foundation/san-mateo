//! Proxy calls from harvest activities.
//!
//! Harvest activities run at least once. Set `forward_idempotency_key` to send
//! the harvest idempotency key upstream as `Idempotency-Key`. The key is the
//! same for each attempt of one activity call, so a provider that supports the
//! header can drop a repeated call.

use std::time::Duration;

use autumn_harvest::context::ActivityContext;
use autumn_harvest::failure::ActivityFailure;
use san_mateo::{ConnectionKey, ProxyRequest, ProxyResponse, SanMateo};

use crate::failure::{config_failure, failure_for_error, failure_for_status};

/// The header that carries the harvest idempotency key upstream.
pub const IDEMPOTENCY_KEY_HEADER: &str = "idempotency-key";

/// Returns the [`SanMateo`] in the harvest state.
///
/// # Errors
///
/// A non-retryable `san_mateo.config` failure when the state has no
/// [`SanMateo`]. Add it with `HarvestPlugin::state(san)` or
/// `HarvestBuilder::state(san)`.
pub fn san_mateo_from(ctx: &ActivityContext) -> Result<&SanMateo, ActivityFailure> {
    ctx.state::<SanMateo>().ok_or_else(|| {
        config_failure(
            "the harvest state has no SanMateo; add it with HarvestPlugin::state(san) \
             or HarvestBuilder::state(san)",
        )
    })
}

/// Sends a proxy call from inside a harvest activity.
///
/// The function reads [`SanMateo`] from the harvest state. Then it calls
/// [`proxy_with`].
///
/// # Errors
///
/// A `san_mateo.config` failure when the state has no [`SanMateo`], and the
/// errors of [`proxy_with`].
pub async fn proxy_from_activity(
    ctx: &ActivityContext,
    environment: &str,
    key: &ConnectionKey,
    request: ProxyRequest,
    forward_idempotency_key: bool,
) -> Result<ProxyResponse, ActivityFailure> {
    let san = san_mateo_from(ctx)?;
    proxy_with(san, ctx, environment, key, request, forward_idempotency_key).await
}

/// Sends a proxy call with a given [`SanMateo`].
///
/// When `forward_idempotency_key` is `true`, the call sends the harvest
/// idempotency key as `Idempotency-Key`. A reply with a status of 400 or more
/// becomes a failure (see [`crate::failure_for_status`]). A retryable reply
/// with a `Retry-After` value in seconds gives a retry hint to harvest.
///
/// A non-idempotent call (for example `POST`) that gets a 5xx, 408, 425 or a
/// network error gives a non-retryable failure: the provider possibly did
/// the call. It stays retryable with `forward_idempotency_key`, or when the
/// status is in `request.retry_on`.
///
/// # Errors
///
/// The failure of [`crate::failure_for_status`] for a 4xx or 5xx reply. The
/// failure of [`crate::failure_for_error`] for a San Mateo error. A
/// `san_mateo.config` failure when the call must forward the idempotency key
/// and the context has none.
pub async fn proxy_with(
    san: &SanMateo,
    ctx: &ActivityContext,
    environment: &str,
    key: &ConnectionKey,
    request: ProxyRequest,
    forward_idempotency_key: bool,
) -> Result<ProxyResponse, ActivityFailure> {
    let mut request = request;
    if forward_idempotency_key {
        let idempotency_key = ctx
            .idempotency_key()
            .map_err(|_| config_failure("the activity context has no idempotency key"))?;
        // The harvest key replaces a caller key, so each attempt sends one key.
        request
            .headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case(IDEMPOTENCY_KEY_HEADER));
        request = request.header(IDEMPOTENCY_KEY_HEADER, idempotency_key.as_str());
    }
    // After a 5xx or a network error, the provider possibly did the call. Send
    // a non-idempotent call again only with an idempotency key or an opt-in.
    // A 429 means the provider did not do the call.
    let safe_to_send_again = |status: Option<u16>| {
        san_mateo::proxy::is_idempotent(&request.method)
            || forward_idempotency_key
            || status.is_some_and(|s| s == 429 || request.retry_on.contains(&s))
    };
    let response = match san.proxy(environment, key, request.clone()).await {
        Ok(response) => response,
        Err(e) => {
            let mut failure = failure_for_error(&e);
            if matches!(e, san_mateo::Error::Upstream(_)) && !safe_to_send_again(None) {
                failure.non_retryable = true;
            }
            return Err(failure);
        }
    };
    match failure_for_status(response.status, &response.body) {
        None => Ok(response),
        Some(mut failure) => {
            if !safe_to_send_again(Some(response.status)) {
                failure.non_retryable = true;
            }
            Err(match retry_after(&response) {
                Some(wait) if !failure.non_retryable => failure.with_retry_after(wait),
                _ => failure,
            })
        }
    }
}

/// Returns the `Retry-After` wait of a reply, when it is a number of seconds.
#[must_use]
pub fn retry_after(response: &ProxyResponse) -> Option<Duration> {
    response
        .header_value("retry-after")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn reply(headers: &[(&str, &str)]) -> ProxyResponse {
        ProxyResponse {
            status: 503,
            headers: headers
                .iter()
                .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                .collect(),
            body: Vec::new(),
        }
    }

    #[test]
    fn retry_after_reads_whole_seconds() {
        assert_eq!(
            retry_after(&reply(&[("retry-after", "7")])),
            Some(Duration::from_secs(7))
        );
        assert_eq!(
            retry_after(&reply(&[("Retry-After", " 12 ")])),
            Some(Duration::from_secs(12))
        );
        assert_eq!(retry_after(&reply(&[])), None);
        assert_eq!(
            retry_after(&reply(&[("retry-after", "Wed, 21 Oct 2015 07:28:00 GMT")])),
            None
        );
        assert_eq!(retry_after(&reply(&[("retry-after", "-3")])), None);
    }

    #[test]
    fn missing_state_is_a_config_failure() {
        let ctx = ActivityContext::new_test();
        let f = san_mateo_from(&ctx).unwrap_err();
        assert_eq!(f.error_type, crate::CONFIG_ERROR);
        assert!(f.non_retryable);
        assert!(f.message.contains("state"), "{}", f.message);
    }

    #[tokio::test]
    async fn proxy_from_activity_without_state_is_a_config_failure() {
        let ctx = ActivityContext::new_test();
        let f = proxy_from_activity(
            &ctx,
            "dev",
            &ConnectionKey::new("crm", "c-1"),
            ProxyRequest::get("/x"),
            false,
        )
        .await
        .unwrap_err();
        assert_eq!(f.error_type, crate::CONFIG_ERROR);
        assert!(f.non_retryable);
    }
}
