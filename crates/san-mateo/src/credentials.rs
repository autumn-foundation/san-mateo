//! Connection credentials. The JSON shape is the same as in the Nango API.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::provider::AuthMode;
use crate::secret::Secret;

/// OAuth2 authorization code credentials.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct OAuth2Credentials {
    /// The access token.
    pub access_token: Secret,
    /// The refresh token, if the provider gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<Secret>,
    /// The expiry instant, if the provider gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// The token reply as it came from the provider.
    #[serde(default)]
    pub raw: Value,
}

/// OAuth2 client credentials.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientCredentials {
    /// The access token.
    pub token: Secret,
    /// The expiry instant, if the provider gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// The client id.
    pub client_id: String,
    /// The client secret.
    pub client_secret: Secret,
    /// The token reply as it came from the provider.
    #[serde(default)]
    pub raw: Value,
}

/// The credentials of one connection.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Credentials {
    /// OAuth2 authorization code.
    #[serde(rename = "OAUTH2")]
    OAuth2(OAuth2Credentials),
    /// OAuth2 client credentials.
    #[serde(rename = "OAUTH2_CC")]
    OAuth2ClientCredentials(ClientCredentials),
    /// An API key.
    #[serde(rename = "API_KEY")]
    ApiKey {
        /// The key.
        #[serde(rename = "apiKey")]
        api_key: Secret,
    },
    /// HTTP Basic auth.
    #[serde(rename = "BASIC")]
    Basic {
        /// The user name.
        username: String,
        /// The password.
        password: Secret,
    },
    /// No auth.
    #[serde(rename = "NONE")]
    Unauthenticated,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Only the mode, the expiry and the client id show. Tokens and `raw` do not.
        let mut d = f.debug_struct("Credentials");
        d.field("type", &self.auth_mode().as_str());
        if let Some(t) = self.expires_at() {
            d.field("expires_at", &t);
        }
        match self {
            Self::OAuth2(c) => d.field("has_refresh_token", &c.refresh_token.is_some()),
            Self::OAuth2ClientCredentials(c) => d.field("client_id", &c.client_id),
            Self::Basic { username, .. } => d.field("username", username),
            Self::ApiKey { .. } | Self::Unauthenticated => &mut d,
        };
        d.finish_non_exhaustive()
    }
}

impl Credentials {
    /// Returns the auth mode of these credentials.
    #[must_use]
    pub const fn auth_mode(&self) -> AuthMode {
        match self {
            Self::OAuth2(_) => AuthMode::OAuth2,
            Self::OAuth2ClientCredentials(_) => AuthMode::OAuth2ClientCredentials,
            Self::ApiKey { .. } => AuthMode::ApiKey,
            Self::Basic { .. } => AuthMode::Basic,
            Self::Unauthenticated => AuthMode::None,
        }
    }

    /// Returns the expiry instant, if any.
    #[must_use]
    pub const fn expires_at(&self) -> Option<DateTime<Utc>> {
        match self {
            Self::OAuth2(c) => c.expires_at,
            Self::OAuth2ClientCredentials(c) => c.expires_at,
            _ => None,
        }
    }

    /// Returns the bearer token for OAuth2 modes.
    #[must_use]
    pub fn bearer_token(&self) -> Option<&str> {
        match self {
            Self::OAuth2(c) => Some(c.access_token.expose()),
            Self::OAuth2ClientCredentials(c) => Some(c.token.expose()),
            _ => None,
        }
    }

    /// Returns the refresh token, if any.
    #[must_use]
    pub fn refresh_token(&self) -> Option<&str> {
        match self {
            Self::OAuth2(c) => c.refresh_token.as_ref().map(Secret::expose),
            _ => None,
        }
    }

    /// Returns the template variables: `apiKey`, `accessToken`, `clientId`
    /// and `credentials.*`.
    #[must_use]
    pub fn template_vars(&self) -> Value {
        match self {
            Self::OAuth2(c) => json!({
                "accessToken": c.access_token.expose(),
                "credentials": {"access_token": c.access_token.expose(), "raw": c.raw},
            }),
            Self::OAuth2ClientCredentials(c) => json!({
                "accessToken": c.token.expose(),
                "clientId": c.client_id,
                "clientSecret": c.client_secret.expose(),
                "credentials": {"token": c.token.expose(), "client_id": c.client_id, "raw": c.raw},
            }),
            Self::ApiKey { api_key } => json!({
                "apiKey": api_key.expose(),
                "credentials": {"apiKey": api_key.expose()},
            }),
            Self::Basic { username, password } => json!({
                "credentials": {"username": username, "password": password.expose()},
            }),
            Self::Unauthenticated => json!({"credentials": {}}),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn oauth() -> Credentials {
        Credentials::OAuth2(OAuth2Credentials {
            access_token: "at-1".into(),
            refresh_token: Some("rt-1".into()),
            expires_at: Some(Utc.timestamp_opt(1_700_000_000, 0).unwrap()),
            raw: json!({"access_token": "at-1", "scope": "a b"}),
        })
    }

    #[test]
    fn json_shapes_match_the_nango_api() {
        assert_eq!(
            serde_json::to_value(oauth()).unwrap(),
            json!({"type": "OAUTH2", "access_token": "at-1", "refresh_token": "rt-1", "expires_at": "2023-11-14T22:13:20Z", "raw": {"access_token": "at-1", "scope": "a b"}})
        );
        assert_eq!(
            serde_json::to_value(Credentials::ApiKey { api_key: "k".into() }).unwrap(),
            json!({"type": "API_KEY", "apiKey": "k"})
        );
        assert_eq!(
            serde_json::to_value(Credentials::Basic { username: "u".into(), password: "p".into() }).unwrap(),
            json!({"type": "BASIC", "username": "u", "password": "p"})
        );
        assert_eq!(serde_json::to_value(Credentials::Unauthenticated).unwrap(), json!({"type": "NONE"}));
        let cc: Credentials = serde_json::from_value(json!({"type": "OAUTH2_CC", "token": "t", "client_id": "id", "client_secret": "s"})).unwrap();
        assert_eq!(cc.auth_mode(), AuthMode::OAuth2ClientCredentials);
        let back: Credentials = serde_json::from_value(serde_json::to_value(oauth()).unwrap()).unwrap();
        assert_eq!(back, oauth());
    }

    #[test]
    fn debug_hides_secrets() {
        let all = [
            oauth(),
            Credentials::ApiKey { api_key: "key-secret".into() },
            Credentials::Basic { username: "u".into(), password: "pw-secret".into() },
            Credentials::OAuth2ClientCredentials(ClientCredentials {
                token: "tok-secret".into(),
                expires_at: None,
                client_id: "cid".into(),
                client_secret: "cs-secret".into(),
                raw: json!({"access_token": "tok-secret"}),
            }),
        ];
        for c in all {
            let d = format!("{c:?}");
            for s in ["at-1", "rt-1", "key-secret", "pw-secret", "tok-secret", "cs-secret"] {
                assert!(!d.contains(s), "{d}");
            }
        }
    }

    #[test]
    fn accessors() {
        let o = oauth();
        assert_eq!(o.auth_mode(), AuthMode::OAuth2);
        assert_eq!(o.bearer_token(), Some("at-1"));
        assert_eq!(o.refresh_token(), Some("rt-1"));
        assert!(o.expires_at().is_some());
        let k = Credentials::ApiKey { api_key: "k".into() };
        assert_eq!(k.auth_mode(), AuthMode::ApiKey);
        assert_eq!(k.bearer_token(), None);
        assert_eq!(k.expires_at(), None);
        assert_eq!(Credentials::Unauthenticated.auth_mode(), AuthMode::None);
        assert_eq!(Credentials::Basic { username: "u".into(), password: "p".into() }.auth_mode(), AuthMode::Basic);
    }

    #[test]
    fn template_vars_by_mode() {
        let v = Credentials::ApiKey { api_key: "k".into() }.template_vars();
        assert_eq!(v["apiKey"], "k");
        assert_eq!(v["credentials"]["apiKey"], "k");
        let v = Credentials::Basic { username: "u".into(), password: "p".into() }.template_vars();
        assert_eq!(v["credentials"]["username"], "u");
        assert_eq!(v["credentials"]["password"], "p");
        let v = oauth().template_vars();
        assert_eq!(v["accessToken"], "at-1");
        assert_eq!(v["credentials"]["access_token"], "at-1");
        let v = Credentials::OAuth2ClientCredentials(ClientCredentials {
            token: "t".into(),
            expires_at: None,
            client_id: "cid".into(),
            client_secret: "cs".into(),
            raw: Value::Null,
        })
        .template_vars();
        assert_eq!(v["accessToken"], "t");
        assert_eq!(v["clientId"], "cid");
        assert_eq!(v["clientSecret"], "cs");
        assert!(Credentials::Unauthenticated.template_vars().is_object());
    }
}
