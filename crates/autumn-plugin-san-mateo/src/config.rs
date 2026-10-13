//! `[san_mateo]` configuration.
//!
//! Sources, in order (later wins):
//! 1. `[san_mateo]` in `autumn.toml`.
//! 2. `[san_mateo]` in the profile file, for example `autumn-prod.toml`.
//! 3. `.env` values, then the process environment: `AUTUMN_SAN_MATEO__<PATH>`,
//!    for example `AUTUMN_SAN_MATEO__BACKGROUND__ENABLED=false`. Put a comma
//!    between list items. A map key in a path is in lower case.
//!
//! The config holds no secrets. It holds the names of the env vars that hold
//! them. The plugin reads those env vars at startup.

use std::collections::BTreeMap;
use std::path::Path;

use san_mateo::{BackgroundSettings, Catalog, EnvironmentConfig, KeyRing, ProxySettings};
use serde::{Deserialize, Serialize};

use crate::error::PluginError;

/// Name of the TOML section.
pub const SECTION: &str = "san_mateo";
/// Prefix for environment overrides.
pub const ENV_PREFIX: &str = "AUTUMN_SAN_MATEO__";
/// The most webhook URLs for one environment: a primary and a secondary.
pub const MAX_WEBHOOK_URLS: usize = 2;
/// The most webhook retries.
pub const MAX_WEBHOOK_RETRIES: u32 = 10;

/// Plugin configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct SanMateoConfig {
    /// The public URL of the app, for example `https://app.example.com`. The
    /// plugin adds the route prefix to get the San Mateo base URL.
    pub server_url: String,
    /// The base of `connect_link`. Default: `<base URL>/connect`.
    pub connect_url: Option<String>,
    /// The id of the active encryption key.
    pub encryption_key_id: String,
    /// The env var with the active key: 32 bytes in base64.
    pub encryption_key_env: String,
    /// Old keys that still open data, as key id to env var name. Use during
    /// rotation.
    pub old_encryption_keys: BTreeMap<String, String>,
    /// The key id for lookup hashes. Default: `encryption_key_id`. Keep the
    /// old id here during rotation, so that lookup hashes stay the same.
    pub lookup_key_id: Option<String>,
    /// A Nango-format YAML file. Its providers replace the built-in ones with
    /// the same name. A relative path starts at the manifest directory.
    pub providers_file: Option<String>,
    /// The environments, by id.
    pub environments: BTreeMap<String, EnvironmentSettings>,
    /// Proxy settings.
    pub proxy: ProxyConfig,
    /// Background loops: refresh sweep, sync scheduler and session purge.
    pub background: BackgroundConfig,
    /// The longest wait at shutdown for webhooks and syncs that run.
    pub drain_timeout_secs: u64,
}

/// One environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct EnvironmentSettings {
    /// The env var with the API secret key of the environment.
    pub secret_key_env: String,
    /// The OAuth callback URL. Default: `<base URL>/oauth/callback`.
    pub callback_url: Option<String>,
    /// Outbound webhooks.
    pub webhooks: WebhookConfig,
}

/// Outbound webhooks of one environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct WebhookConfig {
    /// The primary URL and an optional secondary URL.
    pub urls: Vec<String>,
    /// The env var with the signing secret. Required when `urls` is set.
    pub signing_secret_env: Option<String>,
    /// Retries after the first attempt, 0 to 10.
    pub retries: u32,
    /// The first backoff.
    pub initial_backoff_ms: u64,
    /// Send `auth` events.
    pub send_auth: bool,
    /// Send `sync` events.
    pub send_sync: bool,
}

/// Proxy settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct ProxyConfig {
    /// Extra hosts for `Base-Url-Override`. `*.example.com` allows sub-domains.
    pub allowed_hosts: Vec<String>,
    /// Allow private, loopback and link-local hosts. Only for local tests.
    pub allow_private_hosts: bool,
    /// Allow plain `http` upstream. Only for local tests.
    pub allow_plain_http: bool,
    /// The first backoff.
    pub initial_backoff_ms: u64,
    /// The longest wait before a retry.
    pub max_wait_ms: u64,
}

/// Background loop settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct BackgroundConfig {
    /// Run the loops. They run only in a process role that runs workers.
    pub enabled: bool,
    /// Time between ticks, 1 s or more.
    pub tick_secs: u64,
    /// Refresh tokens that expire within this time.
    pub refresh_within_secs: u64,
    /// The most refreshes in one tick, 1 or more.
    pub refresh_batch: usize,
    /// Run the sync scheduler.
    pub run_syncs: bool,
}

impl Default for SanMateoConfig {
    fn default() -> Self {
        todo!()
    }
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        todo!()
    }
}

impl Default for WebhookConfig {
    fn default() -> Self {
        todo!()
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        todo!()
    }
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        todo!()
    }
}

impl ProxyConfig {
    /// The core proxy settings.
    #[must_use]
    pub fn settings(&self) -> ProxySettings {
        todo!()
    }
}

impl BackgroundConfig {
    /// The core background settings.
    #[must_use]
    pub fn settings(&self) -> BackgroundSettings {
        todo!()
    }
}

impl SanMateoConfig {
    /// Reads `[san_mateo]` from the text of an `autumn.toml` file.
    ///
    /// # Errors
    /// Returns [`PluginError::Config`] for bad TOML, unknown keys, or failed
    /// validation.
    pub fn from_toml_str(autumn_toml: &str) -> Result<Self, PluginError> {
        let _ = autumn_toml;
        todo!()
    }

    /// Loads config from files in `manifest_dir` (else the current
    /// directory) and from `env`.
    ///
    /// # Errors
    /// Returns [`PluginError::Config`] for a file that does not read or
    /// parse, or for failed validation.
    pub fn load_from_dir(
        manifest_dir: &Path,
        profile_names: &[String],
        env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, PluginError> {
        let _ = (manifest_dir, profile_names, env.into_iter());
        todo!()
    }

    /// Loads config like autumn-web does: `$AUTUMN_MANIFEST_DIR` (else `.`),
    /// the app profile, `.env`, and the process environment.
    ///
    /// # Errors
    /// Returns [`PluginError::Config`] when loading or validation fails.
    pub fn load(profile: Option<&str>) -> Result<Self, PluginError> {
        let _ = profile;
        todo!()
    }

    /// Checks the values.
    ///
    /// # Errors
    /// Returns [`PluginError::Config`] with the first problem found.
    pub fn validate(&self) -> Result<(), PluginError> {
        todo!()
    }

    /// Builds the key ring from the env vars that the config names.
    pub(crate) fn key_ring(&self, secrets: &Secrets) -> Result<KeyRing, PluginError> {
        let _ = secrets;
        todo!()
    }

    /// Builds the core environments from the env vars that the config names.
    pub(crate) fn environment_configs(
        &self,
        secrets: &Secrets,
    ) -> Result<Vec<EnvironmentConfig>, PluginError> {
        let _ = secrets;
        todo!()
    }

    /// Returns `base` (else the built-in catalog) with `providers_file`
    /// merged over it.
    pub(crate) fn catalog(&self, base: Option<Catalog>) -> Result<Catalog, PluginError> {
        let _ = base;
        todo!()
    }
}

/// Finds secret values by env var name.
///
/// Order: values set on the plugin, the process environment, then `.env`.
/// An empty value counts as not set.
#[derive(Clone, Default)]
pub(crate) struct Secrets {
    overrides: BTreeMap<String, String>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl Secrets {
    pub(crate) const fn new(overrides: BTreeMap<String, String>) -> Self {
        Self { overrides }
    }

    /// The value of the env var `name`, if it is set and not empty.
    pub(crate) fn get(&self, name: &str) -> Option<String> {
        let _ = name;
        todo!()
    }

    /// The value of the env var `name`. The error names the var and `what`.
    pub(crate) fn require(&self, name: &str, what: &str) -> Result<String, PluginError> {
        let _ = (name, what);
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_B64: &str = "AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
    const OLD_KEY_B64: &str = "HxweHRwbGhkYFxYVFBMSERAPDg0MCwoJCAcGBQQDAgE=";

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn secrets(pairs: &[(&str, &str)]) -> Secrets {
        Secrets::new(
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        )
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sm-plugin-config-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn defaults_are_valid() {
        let c = SanMateoConfig::default();
        c.validate().unwrap();
        assert_eq!(c.server_url, "http://localhost:3000");
        assert_eq!(c.encryption_key_id, "k1");
        assert_eq!(c.encryption_key_env, "SAN_MATEO_ENCRYPTION_KEY");
        assert_eq!(c.environments.len(), 1);
        assert_eq!(
            c.environments["default"].secret_key_env,
            "SAN_MATEO_SECRET_KEY"
        );
        assert!(c.background.enabled);
        assert_eq!(c.background.tick_secs, 30);
        assert_eq!(c.drain_timeout_secs, 10);
        assert_eq!(
            SanMateoConfig::from_toml_str("").unwrap(),
            SanMateoConfig::default()
        );
    }

    #[test]
    fn reads_section_and_rejects_unknown_keys() {
        let text = r#"
[server]
port = 1

[san_mateo]
server_url = "https://app.example.com/"
providers_file = "extra.yaml"

[san_mateo.environments.prod]
secret_key_env = "SM_PROD_KEY"

[san_mateo.environments.prod.webhooks]
urls = ["https://hooks.example.com/a", "https://hooks.example.com/b"]
signing_secret_env = "SM_PROD_WHSEC"
retries = 0

[san_mateo.proxy]
allowed_hosts = ["*.example.com"]

[san_mateo.background]
enabled = false
"#;
        let c = SanMateoConfig::from_toml_str(text).unwrap();
        assert_eq!(c.server_url, "https://app.example.com/");
        assert_eq!(c.providers_file.as_deref(), Some("extra.yaml"));
        assert_eq!(c.environments.len(), 1, "the user map replaces the default");
        let prod = &c.environments["prod"];
        assert_eq!(prod.secret_key_env, "SM_PROD_KEY");
        assert_eq!(prod.webhooks.urls.len(), 2);
        assert_eq!(prod.webhooks.retries, 0);
        assert!(prod.webhooks.send_auth, "a missing key keeps its default");
        assert_eq!(c.proxy.allowed_hosts, ["*.example.com"]);
        assert!(!c.background.enabled);
        let e = SanMateoConfig::from_toml_str("[san_mateo]\nserver = \"x\"\n").unwrap_err();
        assert!(e.to_string().contains("server"), "{e}");
        let e = SanMateoConfig::from_toml_str(
            "[san_mateo.environments.prod]\nsecret_key = \"sk_live_literal\"\n",
        )
        .unwrap_err();
        assert!(!e.to_string().contains("sk_live_literal"), "{e}");
        assert!(SanMateoConfig::from_toml_str("[san_mateo\n").is_err());
    }

    #[test]
    fn env_overlay_types_values() {
        let c = SanMateoConfig::load_from_dir(
            &scratch_dir("overlay"),
            &[],
            env(&[
                ("AUTUMN_SAN_MATEO__SERVER_URL", "https://sm.example.com"),
                ("AUTUMN_SAN_MATEO__ENVIRONMENTS__PROD__SECRET_KEY_ENV", "PROD_KEY"),
                (
                    "AUTUMN_SAN_MATEO__ENVIRONMENTS__PROD__WEBHOOKS__URLS",
                    "https://h.example.com/a, https://h.example.com/b",
                ),
                (
                    "AUTUMN_SAN_MATEO__ENVIRONMENTS__PROD__WEBHOOKS__SIGNING_SECRET_ENV",
                    "PROD_WHSEC",
                ),
                ("AUTUMN_SAN_MATEO__ENVIRONMENTS__PROD__WEBHOOKS__RETRIES", "4"),
                ("AUTUMN_SAN_MATEO__ENVIRONMENTS__PROD__WEBHOOKS__SEND_SYNC", "false"),
                ("AUTUMN_SAN_MATEO__OLD_ENCRYPTION_KEYS__K0", "OLD_KEY"),
                ("AUTUMN_SAN_MATEO__BACKGROUND__ENABLED", "false"),
                ("AUTUMN_SAN_MATEO__BACKGROUND__REFRESH_BATCH", "7"),
                ("AUTUMN_SAN_MATEO__PROXY__ALLOWED_HOSTS", "a.example.com,b.example.com"),
                ("AUTUMN_OTHER__X", "ignored"),
            ]),
        )
        .unwrap();
        assert_eq!(c.server_url, "https://sm.example.com");
        let prod = &c.environments["prod"];
        assert_eq!(prod.secret_key_env, "PROD_KEY");
        assert_eq!(
            prod.webhooks.urls,
            ["https://h.example.com/a", "https://h.example.com/b"]
        );
        assert_eq!(prod.webhooks.signing_secret_env.as_deref(), Some("PROD_WHSEC"));
        assert_eq!(prod.webhooks.retries, 4);
        assert!(!prod.webhooks.send_sync);
        assert_eq!(c.old_encryption_keys["k0"], "OLD_KEY");
        assert!(!c.background.enabled);
        assert_eq!(c.background.refresh_batch, 7);
        assert_eq!(c.proxy.allowed_hosts, ["a.example.com", "b.example.com"]);
        // The error names the variable, never the value.
        let e = SanMateoConfig::load_from_dir(
            &scratch_dir("overlay-bad"),
            &[],
            env(&[("AUTUMN_SAN_MATEO__BACKGROUND__TICK_SECS", "soon-secret-ish")]),
        )
        .unwrap_err()
        .to_string();
        assert!(e.contains("AUTUMN_SAN_MATEO__BACKGROUND__TICK_SECS"), "{e}");
        assert!(!e.contains("soon-secret-ish"), "{e}");
    }

    #[test]
    fn profile_file_layers_over_base_and_paths_start_at_the_dir() {
        let dir = scratch_dir("profile");
        std::fs::write(
            dir.join("autumn.toml"),
            "[san_mateo]\nserver_url = \"https://base.example.com\"\nproviders_file = \"providers.yaml\"\n[san_mateo.background]\ntick_secs = 5\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("autumn-prod.toml"),
            "[san_mateo]\nserver_url = \"https://prod.example.com\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("providers.yaml"), "").unwrap();
        let c = SanMateoConfig::load_from_dir(&dir, &["prod".to_owned()], Vec::new()).unwrap();
        assert_eq!(c.server_url, "https://prod.example.com");
        assert_eq!(c.background.tick_secs, 5, "the base file still counts");
        assert_eq!(
            c.providers_file.as_deref(),
            Some(dir.join("providers.yaml").to_str().unwrap())
        );
        // No profile file: only the base.
        let c = SanMateoConfig::load_from_dir(&dir, &["staging".to_owned()], Vec::new()).unwrap();
        assert_eq!(c.server_url, "https://base.example.com");
        // No files at all: the defaults.
        let c =
            SanMateoConfig::load_from_dir(&scratch_dir("empty"), &[], Vec::new()).unwrap();
        assert_eq!(c, SanMateoConfig::default());
    }

    #[test]
    fn inline_profile_section_is_not_read() {
        let text = "[san_mateo]\ndrain_timeout_secs = 1\n[profile.prod.san_mateo]\ndrain_timeout_secs = 3\n";
        let c = SanMateoConfig::from_toml_str(text).unwrap();
        assert_eq!(c.drain_timeout_secs, 1);
    }

    #[test]
    fn validation_rules() {
        let check = |f: fn(&mut SanMateoConfig)| {
            let mut c = SanMateoConfig::default();
            f(&mut c);
            c.validate().unwrap_err().to_string()
        };
        assert!(check(|c| c.server_url = "ftp://x".into()).contains("server_url"));
        assert!(check(|c| c.server_url = "http://app.example.com".into()).contains("server_url"));
        assert!(check(|c| c.server_url = "https://x.example.com/?a=1".into()).contains("server_url"));
        assert!(check(|c| c.connect_url = Some("nope".into())).contains("connect_url"));
        assert!(check(|c| c.encryption_key_id = String::new()).contains("encryption_key_id"));
        assert!(check(|c| c.encryption_key_id = "a b".into()).contains("encryption_key_id"));
        assert!(check(|c| c.encryption_key_id = "k".repeat(33)).contains("encryption_key_id"));
        assert!(check(|c| c.encryption_key_env = " ".into()).contains("encryption_key_env"));
        assert!(check(|c| {
            c.old_encryption_keys.insert("k1".into(), "X".into());
        })
        .contains("old_encryption_keys"));
        assert!(check(|c| {
            c.old_encryption_keys.insert("bad id".into(), "X".into());
        })
        .contains("old_encryption_keys"));
        assert!(check(|c| {
            c.old_encryption_keys.insert("k0".into(), String::new());
        })
        .contains("old_encryption_keys"));
        assert!(check(|c| c.lookup_key_id = Some("k9".into())).contains("lookup_key_id"));
        assert!(check(|c| c.providers_file = Some(String::new())).contains("providers_file"));
        assert!(check(|c| c.environments.clear()).contains("environments"));
        assert!(check(|c| {
            c.environments.insert("bad id".into(), EnvironmentSettings::default());
        })
        .contains("environments"));
        assert!(check(|c| {
            c.environments.get_mut("default").unwrap().secret_key_env = String::new();
        })
        .contains("secret_key_env"));
        assert!(check(|c| {
            c.environments.insert("other".into(), EnvironmentSettings::default());
        })
        .contains("secret_key_env"));
        assert!(check(|c| {
            c.environments.get_mut("default").unwrap().callback_url = Some("x".into());
        })
        .contains("callback_url"));
        assert!(check(|c| {
            let w = &mut c.environments.get_mut("default").unwrap().webhooks;
            w.urls = vec!["https://h.example.com".into()];
        })
        .contains("signing_secret_env"));
        assert!(check(|c| {
            let w = &mut c.environments.get_mut("default").unwrap().webhooks;
            w.urls = vec!["https://a.example.com".into(); 3];
            w.signing_secret_env = Some("S".into());
        })
        .contains("urls"));
        assert!(check(|c| {
            let w = &mut c.environments.get_mut("default").unwrap().webhooks;
            w.urls = vec!["http://hooks.example.com".into()];
            w.signing_secret_env = Some("S".into());
        })
        .contains("urls"));
        assert!(check(|c| {
            c.environments.get_mut("default").unwrap().webhooks.signing_secret_env =
                Some(String::new());
        })
        .contains("signing_secret_env"));
        assert!(check(|c| {
            c.environments.get_mut("default").unwrap().webhooks.retries = MAX_WEBHOOK_RETRIES + 1;
        })
        .contains("retries"));
        assert!(check(|c| c.proxy.allowed_hosts = vec!["https://x".into()]).contains("allowed_hosts"));
        assert!(check(|c| c.proxy.allowed_hosts = vec![String::new()]).contains("allowed_hosts"));
        assert!(check(|c| c.background.tick_secs = 0).contains("tick_secs"));
        assert!(check(|c| c.background.refresh_batch = 0).contains("refresh_batch"));
        // Valid edge cases.
        let mut c = SanMateoConfig::default();
        c.server_url = "http://127.0.0.1:8080".into();
        c.connect_url = Some("https://connect.example.com/c".into());
        c.old_encryption_keys.insert("k0".into(), "OLD".into());
        c.lookup_key_id = Some("k0".into());
        let w = &mut c.environments.get_mut("default").unwrap().webhooks;
        w.urls = vec!["https://a.example.com/h".into(), "http://localhost:9/h".into()];
        w.signing_secret_env = Some("WHSEC".into());
        c.validate().unwrap();
    }

    #[test]
    fn settings_map_to_the_core() {
        let mut c = SanMateoConfig::default();
        c.proxy.allowed_hosts = vec!["*.example.com".into()];
        c.proxy.max_wait_ms = 9;
        c.background.tick_secs = 3;
        c.background.run_syncs = false;
        let p = c.proxy.settings();
        assert_eq!(p.allowed_hosts, ["*.example.com"]);
        assert_eq!(p.max_wait_ms, 9);
        assert_eq!(p.initial_backoff_ms, 1_000);
        assert!(!p.allow_private_hosts);
        let b = c.background.settings();
        assert_eq!(b.tick, std::time::Duration::from_secs(3));
        assert_eq!(b.refresh_within, std::time::Duration::from_secs(1_800));
        assert_eq!(b.refresh_batch, 100);
        assert!(!b.run_syncs);
    }

    #[test]
    fn secrets_prefer_overrides_and_skip_empty_values() {
        let s = secrets(&[("SM_TEST_A", "from-plugin"), ("SM_TEST_EMPTY", "  ")]);
        assert_eq!(s.get("SM_TEST_A").as_deref(), Some("from-plugin"));
        assert_eq!(s.get("SM_TEST_EMPTY"), None);
        assert_eq!(s.get("SM_PLUGIN_TEST_NEVER_SET_1"), None);
        // `PATH` comes from the process environment.
        assert!(s.get("PATH").is_some());
        let e = s
            .require("SM_PLUGIN_TEST_NEVER_SET_1", "encryption key")
            .unwrap_err()
            .to_string();
        assert!(e.contains("SM_PLUGIN_TEST_NEVER_SET_1") && e.contains("encryption key"), "{e}");
        let debug = format!("{s:?}");
        assert!(debug.contains("SM_TEST_A") && !debug.contains("from-plugin"), "{debug}");
    }

    #[test]
    fn key_ring_reads_named_env_vars_and_supports_rotation() {
        let mut c = SanMateoConfig::default();
        c.encryption_key_env = "SM_KEY_NEW".into();
        c.encryption_key_id = "k2".into();
        let e = c.key_ring(&secrets(&[])).unwrap_err().to_string();
        assert!(e.contains("SM_KEY_NEW"), "{e}");
        let e = c
            .key_ring(&secrets(&[("SM_KEY_NEW", "not-base64-secret")]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("SM_KEY_NEW") && !e.contains("not-base64-secret"), "{e}");
        // Data sealed with the old key opens after rotation.
        let old = KeyRing::from_base64("k1", OLD_KEY_B64).unwrap();
        let sealed = old.seal(b"token", b"row").unwrap();
        c.old_encryption_keys.insert("k1".into(), "SM_KEY_OLD".into());
        c.lookup_key_id = Some("k1".into());
        let e = c
            .key_ring(&secrets(&[("SM_KEY_NEW", KEY_B64)]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("SM_KEY_OLD"), "{e}");
        let ring = c
            .key_ring(&secrets(&[("SM_KEY_NEW", KEY_B64), ("SM_KEY_OLD", OLD_KEY_B64)]))
            .unwrap();
        assert_eq!(ring.active_key_id(), "k2");
        assert_eq!(ring.open(&sealed, b"row").unwrap(), b"token");
        assert!(ring.seal(b"x", b"row").unwrap().contains(".k2."));
        assert_eq!(ring.lookup_hash("v"), old.lookup_hash("v"), "lookup key stays k1");
    }

    #[test]
    fn environments_resolve_secrets_and_webhooks() {
        let mut c = SanMateoConfig::default();
        let mut prod = EnvironmentSettings {
            secret_key_env: "SM_PROD_SK".into(),
            callback_url: Some("https://app.example.com/cb".into()),
            ..EnvironmentSettings::default()
        };
        prod.webhooks.urls = vec!["https://h.example.com/1".into(), "https://h.example.com/2".into()];
        prod.webhooks.signing_secret_env = Some("SM_PROD_WH".into());
        prod.webhooks.retries = 1;
        c.environments = BTreeMap::from([("prod".to_owned(), prod)]);
        let e = c.environment_configs(&secrets(&[])).unwrap_err().to_string();
        assert!(e.contains("SM_PROD_SK") && e.contains("prod"), "{e}");
        let e = c
            .environment_configs(&secrets(&[("SM_PROD_SK", "sk_prod")]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("SM_PROD_WH"), "{e}");
        let envs = c
            .environment_configs(&secrets(&[("SM_PROD_SK", "sk_prod"), ("SM_PROD_WH", "whsec")]))
            .unwrap();
        assert_eq!(envs.len(), 1);
        let p = &envs[0];
        assert_eq!(p.id, "prod");
        assert_eq!(p.secret_key.expose(), "sk_prod");
        assert_eq!(p.callback_url.as_deref(), Some("https://app.example.com/cb"));
        assert_eq!(p.webhooks.primary_url.as_deref(), Some("https://h.example.com/1"));
        assert_eq!(p.webhooks.secondary_url.as_deref(), Some("https://h.example.com/2"));
        assert_eq!(p.webhooks.signing_secret.as_ref().unwrap().expose(), "whsec");
        assert_eq!(p.webhooks.retries, 1);
        assert!(p.webhooks.send_auth && p.webhooks.send_sync);
        // No webhooks: no URLs and no secret.
        let envs = SanMateoConfig::default()
            .environment_configs(&secrets(&[("SAN_MATEO_SECRET_KEY", "sk")]))
            .unwrap();
        assert_eq!(envs[0].id, "default");
        assert!(envs[0].webhooks.primary_url.is_none());
        assert!(envs[0].webhooks.signing_secret.is_none());
    }

    #[test]
    fn catalog_merges_the_providers_file() {
        let dir = scratch_dir("catalog");
        let file = dir.join("providers.yaml");
        std::fs::write(
            &file,
            "acme:\n    display_name: Acme\n    auth_mode: API_KEY\n    proxy:\n        base_url: https://api.acme.example.com\n",
        )
        .unwrap();
        let mut c = SanMateoConfig::default();
        let builtin = c.catalog(None).unwrap();
        assert!(builtin.len() >= 25);
        c.providers_file = Some(file.to_str().unwrap().to_owned());
        let merged = c.catalog(None).unwrap();
        assert_eq!(merged.len(), builtin.len() + 1);
        assert!(merged.get("acme").is_some());
        let only = c.catalog(Some(Catalog::default())).unwrap();
        assert_eq!(only.len(), 1);
        c.providers_file = Some(dir.join("missing.yaml").to_str().unwrap().to_owned());
        let e = c.catalog(None).unwrap_err().to_string();
        assert!(e.contains("missing.yaml"), "{e}");
    }
}
