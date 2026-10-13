//! Shared domain types.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Free-form string tags.
pub type Tags = BTreeMap<String, String>;

/// The organization of an end user.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    /// The id in the product.
    pub id: String,
    /// The name to show.
    #[serde(default)]
    pub display_name: Option<String>,
}

/// A user of the product who connects an account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndUser {
    /// The id in the product.
    pub id: String,
    /// The email address.
    #[serde(default)]
    pub email: Option<String>,
    /// The name to show.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Tags.
    #[serde(default)]
    pub tags: Tags,
    /// The organization.
    #[serde(default)]
    pub organization: Option<Organization>,
}

/// Identifies a connection in an environment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ConnectionKey {
    /// The integration key (Nango: `provider_config_key`).
    pub provider_config_key: String,
    /// The connection id.
    pub connection_id: String,
}

impl ConnectionKey {
    /// Makes a key.
    #[must_use]
    pub fn new(provider_config_key: impl Into<String>, connection_id: impl Into<String>) -> Self {
        Self {
            provider_config_key: provider_config_key.into(),
            connection_id: connection_id.into(),
        }
    }
}

/// Defaults that a connect session sets for one integration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationDefaults {
    /// User scopes, comma separated. They go in `connection_config.user_scope`.
    /// For Slack, they also go in the `user_scope` parameter. They never
    /// replace the `scope` parameter.
    #[serde(default)]
    pub user_scopes: Option<String>,
    /// Authorization parameters. They replace the provider and end-user
    /// parameters with the same name.
    #[serde(default)]
    pub authorization_params: BTreeMap<String, String>,
    /// Connection config values. They replace the end-user values.
    #[serde(default)]
    pub connection_config: serde_json::Map<String, Value>,
}
