//! The plugin error type.

/// An error from the plugin. A message never holds a secret value.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PluginError {
    /// The `[san_mateo]` config or the plugin setup is not valid.
    #[error("san_mateo config: {0}")]
    Config(String),
    /// The San Mateo service did not build.
    #[error("san_mateo: {0}")]
    Service(#[from] san_mateo::Error),
}

impl PluginError {
    /// Makes a config error.
    pub(crate) fn config(message: impl Into<String>) -> Self {
        Self::Config(message.into())
    }
}
