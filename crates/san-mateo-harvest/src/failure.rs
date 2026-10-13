//! Failure mapping: San Mateo errors and HTTP replies to harvest failures.
//!
//! The retry classes follow `harvest-camel`. HTTP 5xx, 408, 425 and 429 are
//! retryable. Other 4xx replies are not retryable, because the same input
//! fails again. A [`san_mateo::Error`] is retryable when
//! [`san_mateo::Error::is_retryable`] is `true`.
//!
//! A failure never holds request headers. A failure for an HTTP reply holds
//! the status and a short preview of the reply body.

use std::time::Duration;

use autumn_harvest::failure::ActivityFailure;
use serde_json::json;

/// The prefix of each error type that this crate makes.
pub const ERROR_PREFIX: &str = "san_mateo.";

/// The error type of a configuration failure. An example is a harvest state
/// with no [`san_mateo::SanMateo`].
pub const CONFIG_ERROR: &str = "san_mateo.config";

/// The error type of an activity input that is not valid.
pub const INVALID_INPUT_ERROR: &str = "san_mateo.invalid_input";

/// The error type of a sync run that ran and failed.
pub const SYNC_FAILED_ERROR: &str = "san_mateo.sync_failed";

/// The largest number of body bytes in a failure.
pub const BODY_PREVIEW_BYTES: usize = 512;

/// The wait of San Mateo after a failed refresh, as a retry hint.
const REFRESH_COOLDOWN: Duration = Duration::from_millis(san_mateo::policy::REFRESH_COOLDOWN_MS);

/// Returns `true` when a later attempt can get a different reply for this
/// HTTP status.
#[must_use]
pub const fn is_retryable_status(status: u16) -> bool {
    status >= 500 || matches!(status, 408 | 425 | 429)
}

/// Returns the failure for an HTTP reply, or `None` for a status below 400.
///
/// The error type is `san_mateo.http_<status>`. The details hold the status
/// and a body preview of [`BODY_PREVIEW_BYTES`] or fewer bytes.
#[must_use]
pub fn failure_for_status(status: u16, body: &[u8]) -> Option<ActivityFailure> {
    if status < 400 {
        return None;
    }
    let error_type = format!("{ERROR_PREFIX}http_{status}");
    let message = format!("the upstream reply has HTTP status {status}");
    let failure = if is_retryable_status(status) {
        ActivityFailure::retryable(error_type, message)
    } else {
        ActivityFailure::non_retryable(error_type, message)
    };
    Some(failure.with_details(json!({
        "status": status,
        "body_preview": body_preview(body),
    })))
}

/// Returns the failure for a San Mateo error.
///
/// The error type is `san_mateo.<code>`. The failure is retryable when the
/// error is retryable. The message is the error text, which has no secrets.
#[must_use]
pub fn failure_for_error(error: &san_mateo::Error) -> ActivityFailure {
    let error_type = format!("{ERROR_PREFIX}{}", error.code());
    let message = error.to_string();
    let failure = if error.is_retryable() {
        ActivityFailure::retryable(error_type, message)
    } else {
        ActivityFailure::non_retryable(error_type, message)
    };
    let mut details = json!({ "code": error.code(), "status": error.status() });
    if let san_mateo::Error::Provider {
        status: Some(upstream),
        ..
    } = error
    {
        details["upstream_status"] = json!(upstream);
    }
    let failure = failure.with_details(details);
    if matches!(error, san_mateo::Error::RefreshCooldown) {
        failure.with_retry_after(REFRESH_COOLDOWN)
    } else {
        failure
    }
}

/// Returns the first [`BODY_PREVIEW_BYTES`] bytes of a body as text.
///
/// The cut is on a character boundary. Bytes that are not UTF-8 become the
/// replacement character.
#[must_use]
pub fn body_preview(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    if text.len() <= BODY_PREVIEW_BYTES {
        return text.into_owned();
    }
    let mut end = BODY_PREVIEW_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

/// Returns a configuration failure. It is not retryable, because a later
/// attempt on the same worker has the same configuration.
pub(crate) fn config_failure(message: impl Into<String>) -> ActivityFailure {
    ActivityFailure::non_retryable(CONFIG_ERROR, message)
}

/// Returns a failure for an input that is not valid. It is not retryable.
pub(crate) fn invalid_input(message: impl Into<String>) -> ActivityFailure {
    ActivityFailure::non_retryable(INVALID_INPUT_ERROR, message)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use san_mateo::Error;

    #[test]
    fn statuses_below_400_are_not_failures() {
        for status in [100, 200, 201, 204, 299, 301, 302, 304, 399] {
            assert!(failure_for_status(status, b"x").is_none(), "{status}");
        }
    }

    #[test]
    fn retryable_statuses() {
        for status in [408, 425, 429, 500, 502, 503, 504, 599, 600] {
            assert!(is_retryable_status(status), "{status}");
            let f = failure_for_status(status, b"busy").unwrap();
            assert!(!f.non_retryable, "{status}");
            assert_eq!(f.error_type, format!("san_mateo.http_{status}"));
        }
    }

    #[test]
    fn other_client_errors_are_not_retryable() {
        for status in [
            400, 401, 402, 403, 404, 405, 409, 410, 413, 415, 422, 451, 499,
        ] {
            assert!(!is_retryable_status(status), "{status}");
            let f = failure_for_status(status, b"nope").unwrap();
            assert!(f.non_retryable, "{status}");
            assert_eq!(f.error_type, format!("san_mateo.http_{status}"));
        }
    }

    #[test]
    fn status_failure_details_hold_status_and_body_preview() {
        let f = failure_for_status(422, br#"{"error":"bad field"}"#).unwrap();
        let details = f.details.unwrap();
        assert_eq!(details["status"], 422);
        assert_eq!(details["body_preview"], r#"{"error":"bad field"}"#);
        assert!(f.message.contains("422"), "{}", f.message);
        assert_eq!(details.as_object().unwrap().len(), 2);
    }

    #[test]
    fn body_preview_is_capped_on_a_char_boundary() {
        assert_eq!(body_preview(b""), "");
        assert_eq!(body_preview(b"short"), "short");
        let long = "a".repeat(BODY_PREVIEW_BYTES + 100);
        assert_eq!(body_preview(long.as_bytes()).len(), BODY_PREVIEW_BYTES);
        // A two-byte char that crosses the cap is left out.
        let mut text = "b".repeat(BODY_PREVIEW_BYTES - 1);
        text.push('é');
        let preview = body_preview(text.as_bytes());
        assert_eq!(preview, "b".repeat(BODY_PREVIEW_BYTES - 1));
        // Bytes that are not UTF-8 do not panic.
        let preview = body_preview(&[0xff, 0xfe, b'o', b'k']);
        assert!(preview.ends_with("ok"), "{preview}");
    }

    #[test]
    fn errors_map_to_codes_and_retry_classes() {
        let cases: Vec<(Error, &str, bool)> = vec![
            (
                Error::Upstream("reset".into()),
                "san_mateo.upstream_error",
                true,
            ),
            (Error::Store("busy".into()), "san_mateo.store_error", true),
            (
                Error::RefreshCooldown,
                "san_mateo.connection_refresh_backoff",
                true,
            ),
            (
                Error::Provider {
                    code: "refresh_failed",
                    status: Some(503),
                    message: "down".into(),
                },
                "san_mateo.refresh_failed",
                true,
            ),
            (
                Error::Provider {
                    code: "refresh_failed",
                    status: Some(400),
                    message: "invalid_grant".into(),
                },
                "san_mateo.refresh_failed",
                false,
            ),
            (
                Error::UnknownConnection("crm/c-1".into()),
                "san_mateo.unknown_connection",
                false,
            ),
            (
                Error::UnknownFunction("crm/x".into()),
                "san_mateo.unknown_function",
                false,
            ),
            (
                Error::RefreshExhausted,
                "san_mateo.connection_refresh_exhausted",
                false,
            ),
            (
                Error::Invalid("bad".into()),
                "san_mateo.invalid_body",
                false,
            ),
            (
                Error::Function("boom".into()),
                "san_mateo.function_failed",
                false,
            ),
            (Error::Crypto, "san_mateo.decryption_failed", false),
        ];
        for (error, error_type, retryable) in cases {
            let f = failure_for_error(&error);
            assert_eq!(f.error_type, error_type, "{error:?}");
            assert_eq!(f.non_retryable, !retryable, "{error:?}");
            assert_eq!(f.message, error.to_string());
            let details = f.details.unwrap();
            assert_eq!(details["code"], error.code());
            assert_eq!(details["status"], error.status());
        }
    }

    #[test]
    fn provider_errors_carry_the_upstream_status() {
        let f = failure_for_error(&Error::Provider {
            code: "refresh_failed",
            status: Some(429),
            message: "slow down".into(),
        });
        assert_eq!(f.details.unwrap()["upstream_status"], 429);
        let f = failure_for_error(&Error::Upstream("reset".into()));
        assert!(f.details.unwrap().get("upstream_status").is_none());
    }

    #[test]
    fn a_refresh_cooldown_gives_a_retry_hint() {
        let f = failure_for_error(&Error::RefreshCooldown);
        assert_eq!(f.retry_after, Some(std::time::Duration::from_secs(30)));
        assert_eq!(failure_for_error(&Error::Crypto).retry_after, None);
    }

    #[test]
    fn helper_failures_are_not_retryable() {
        let f = config_failure("no state");
        assert_eq!(f.error_type, CONFIG_ERROR);
        assert!(f.non_retryable);
        let f = invalid_input("bad");
        assert_eq!(f.error_type, INVALID_INPUT_ERROR);
        assert!(f.non_retryable);
        assert!(CONFIG_ERROR.starts_with(ERROR_PREFIX));
        assert!(SYNC_FAILED_ERROR.starts_with(ERROR_PREFIX));
    }
}
