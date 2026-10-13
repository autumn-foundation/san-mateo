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
//!
//! ```toml
//! [san_mateo]
//! server_url = "https://app.example.com"
//! encryption_key_env = "SAN_MATEO_ENCRYPTION_KEY"
//! providers_file = "providers.yaml"
//!
//! [san_mateo.environments.prod]
//! secret_key_env = "SAN_MATEO_PROD_SECRET_KEY"
//!
//! [san_mateo.environments.prod.webhooks]
//! urls = ["https://app.example.com/hooks/san-mateo"]
//! signing_secret_env = "SAN_MATEO_PROD_WEBHOOK_SECRET"
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use san_mateo::{
    BackgroundSettings, Catalog, EnvironmentConfig, KeyRing, ProxySettings, Secret, WebhookSettings,
};
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
/// The id of the environment in the default config.
pub const DEFAULT_ENVIRONMENT: &str = "default";
/// A map key in the env overlay schema that stands for any key.
const ANY_KEY: &str = "*";

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
        Self {
            server_url: "http://localhost:3000".to_owned(),
            connect_url: None,
            encryption_key_id: "k1".to_owned(),
            encryption_key_env: "SAN_MATEO_ENCRYPTION_KEY".to_owned(),
            old_encryption_keys: BTreeMap::new(),
            lookup_key_id: None,
            providers_file: None,
            environments: BTreeMap::from([(
                DEFAULT_ENVIRONMENT.to_owned(),
                EnvironmentSettings::default(),
            )]),
            proxy: ProxyConfig::default(),
            background: BackgroundConfig::default(),
            drain_timeout_secs: 10,
        }
    }
}

impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self {
            secret_key_env: "SAN_MATEO_SECRET_KEY".to_owned(),
            callback_url: None,
            webhooks: WebhookConfig::default(),
        }
    }
}

impl Default for WebhookConfig {
    fn default() -> Self {
        let core = WebhookSettings::default();
        Self {
            urls: Vec::new(),
            signing_secret_env: None,
            retries: core.retries,
            initial_backoff_ms: core.initial_backoff_ms,
            send_auth: core.send_auth,
            send_sync: core.send_sync,
        }
    }
}

impl Default for ProxyConfig {
    fn default() -> Self {
        let core = ProxySettings::default();
        Self {
            allowed_hosts: core.allowed_hosts,
            allow_private_hosts: core.allow_private_hosts,
            allow_plain_http: core.allow_plain_http,
            initial_backoff_ms: core.initial_backoff_ms,
            max_wait_ms: core.max_wait_ms,
        }
    }
}

impl Default for BackgroundConfig {
    fn default() -> Self {
        let core = BackgroundSettings::default();
        Self {
            enabled: true,
            tick_secs: core.tick.as_secs(),
            refresh_within_secs: core.refresh_within.as_secs(),
            refresh_batch: core.refresh_batch,
            run_syncs: core.run_syncs,
        }
    }
}

impl ProxyConfig {
    /// The core proxy settings.
    #[must_use]
    pub fn settings(&self) -> ProxySettings {
        ProxySettings {
            allowed_hosts: self.allowed_hosts.clone(),
            allow_private_hosts: self.allow_private_hosts,
            allow_plain_http: self.allow_plain_http,
            initial_backoff_ms: self.initial_backoff_ms,
            max_wait_ms: self.max_wait_ms,
        }
    }
}

impl BackgroundConfig {
    /// The core background settings.
    #[must_use]
    pub const fn settings(&self) -> BackgroundSettings {
        BackgroundSettings {
            tick: Duration::from_secs(self.tick_secs),
            refresh_within: Duration::from_secs(self.refresh_within_secs),
            refresh_batch: self.refresh_batch,
            run_syncs: self.run_syncs,
        }
    }
}

impl SanMateoConfig {
    /// Reads `[san_mateo]` from the text of an `autumn.toml` file.
    ///
    /// # Errors
    /// Returns [`PluginError::Config`] for bad TOML, unknown keys, or failed
    /// validation.
    pub fn from_toml_str(autumn_toml: &str) -> Result<Self, PluginError> {
        Self::from_layers(Some(autumn_toml), None, std::iter::empty(), None)
    }

    /// Merges, in order: `[san_mateo]` in `base`, `[san_mateo]` in
    /// `profile_file`, and env vars. A relative `providers_file` starts at
    /// `dir` when the file is there. Then validates.
    fn from_layers(
        base: Option<&str>,
        profile_file: Option<&str>,
        env: impl IntoIterator<Item = (String, String)>,
        dir: Option<&Path>,
    ) -> Result<Self, PluginError> {
        let mut table = toml::Table::new();
        for text in [base, profile_file].into_iter().flatten() {
            let parsed: toml::Table =
                toml::from_str(text).map_err(|e| PluginError::config(format!("toml: {e}")))?;
            if let Some(toml::Value::Table(section)) = parsed.get(SECTION) {
                merge(&mut table, section.clone());
            }
        }
        let schema = schema()?;
        for (key, value) in env {
            let Some(path) = key.strip_prefix(ENV_PREFIX) else {
                continue;
            };
            let path: Vec<String> = path.split("__").map(str::to_ascii_lowercase).collect();
            // The error names the variable, never the value.
            let typed = typed_env_value(&schema, &path, &value)
                .ok_or_else(|| PluginError::config(format!("{key}: the value is not valid")))?;
            insert_path(&mut table, &path, typed);
        }
        let mut cfg: Self = toml::Value::Table(table)
            .try_into()
            .map_err(|e| PluginError::config(e.to_string()))?;
        if let (Some(dir), Some(file)) = (dir, cfg.providers_file.as_mut()) {
            let candidate = dir.join(&*file);
            if Path::new(file).is_relative() && candidate.exists() {
                *file = candidate.to_string_lossy().into_owned();
            }
        }
        cfg.validate()?;
        Ok(cfg)
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
        let find = |file: &str| {
            let candidate = manifest_dir.join(file);
            if candidate.exists() {
                candidate
            } else {
                PathBuf::from(file)
            }
        };
        let base = read_optional(&find("autumn.toml"))?;
        let mut profile = None;
        for name in profile_names {
            if let Some(text) = read_optional(&find(&format!("autumn-{name}.toml")))? {
                profile = Some(text);
                break;
            }
        }
        Self::from_layers(base.as_deref(), profile.as_deref(), env, Some(manifest_dir))
    }

    /// Loads config like autumn-web does: `$AUTUMN_MANIFEST_DIR` (else `.`),
    /// the app profile, `.env`, and the process environment.
    ///
    /// # Errors
    /// Returns [`PluginError::Config`] when loading or validation fails.
    pub fn load(profile: Option<&str>) -> Result<Self, PluginError> {
        use autumn_web::config::Env as _;
        let os = autumn_web::config::OsEnv;
        let names = profile
            .map(|p| {
                // Same selector order as autumn-web: env vars, then `--profile`.
                let selector = ["AUTUMN_ENV", "AUTUMN_PROFILE"]
                    .iter()
                    .find_map(|k| os.var(k).ok().filter(|v| !v.trim().is_empty()))
                    .or_else(profile_flag)
                    .map_or_else(|| p.to_owned(), |v| v.trim().to_owned());
                autumn_web::config::profile_override_file_lookup_names(p, &selector)
            })
            .unwrap_or_default();
        let dir = os
            .var("AUTUMN_MANIFEST_DIR")
            .map_or_else(|_| PathBuf::from("."), PathBuf::from);
        Self::load_from_dir(&dir, &names, process_env()?)
    }

    /// Checks the values.
    ///
    /// # Errors
    /// Returns [`PluginError::Config`] with the first problem found.
    pub fn validate(&self) -> Result<(), PluginError> {
        let bad = |m: String| Err(PluginError::config(m));
        if !valid_url(&self.server_url) {
            return bad(format!(
                "server_url must be an https URL with no query (http only on loopback): {}",
                self.server_url
            ));
        }
        if let Some(url) = &self.connect_url
            && !valid_url(url)
        {
            return bad(format!(
                "connect_url must be an https URL with no query (http only on loopback): {url}"
            ));
        }
        if !valid_id(&self.encryption_key_id, 32) {
            return bad(format!("encryption_key_id {ID_RULE}, at most 32"));
        }
        if self.encryption_key_env.trim().is_empty() {
            return bad("encryption_key_env must not be empty".to_owned());
        }
        for (id, env) in &self.old_encryption_keys {
            if !valid_id(id, 32) {
                return bad(format!(
                    "old_encryption_keys: the id {id:?} {ID_RULE}, at most 32"
                ));
            }
            if *id == self.encryption_key_id {
                return bad(format!("old_encryption_keys: {id} is the active key id"));
            }
            if env.trim().is_empty() {
                return bad(format!("old_encryption_keys.{id} must name an env var"));
            }
        }
        if let Some(id) = &self.lookup_key_id
            && *id != self.encryption_key_id
            && !self.old_encryption_keys.contains_key(id)
        {
            return bad(format!(
                "lookup_key_id {id} must be encryption_key_id or a key in old_encryption_keys"
            ));
        }
        if self
            .providers_file
            .as_ref()
            .is_some_and(|f| f.trim().is_empty())
        {
            return bad("providers_file must not be empty".to_owned());
        }
        self.validate_environments()?;
        if self
            .proxy
            .allowed_hosts
            .iter()
            .any(|h| h.trim().is_empty() || h.contains(['/', ':', ' ']))
        {
            return bad("proxy.allowed_hosts must list host names only".to_owned());
        }
        if self.background.tick_secs == 0 {
            return bad("background.tick_secs must be 1 or more".to_owned());
        }
        if self.background.refresh_batch == 0 {
            return bad("background.refresh_batch must be 1 or more".to_owned());
        }
        Ok(())
    }

    fn validate_environments(&self) -> Result<(), PluginError> {
        let bad = |m: String| Err(PluginError::config(m));
        if self.environments.is_empty() {
            return bad("environments must hold one or more environments".to_owned());
        }
        let mut key_envs = std::collections::BTreeSet::new();
        for (id, env) in &self.environments {
            if !valid_id(id, 64) {
                return bad(format!("environments: the id {id:?} {ID_RULE}, at most 64"));
            }
            if env.secret_key_env.trim().is_empty() {
                return bad(format!(
                    "environments.{id}.secret_key_env must not be empty"
                ));
            }
            if !key_envs.insert(env.secret_key_env.as_str()) {
                return bad(format!(
                    "environments.{id}.secret_key_env: each environment needs its own env var, \
                     not {}",
                    env.secret_key_env
                ));
            }
            if let Some(url) = &env.callback_url
                && !valid_url(url)
            {
                return bad(format!(
                    "environments.{id}.callback_url must be an https URL (http only on loopback)"
                ));
            }
            let hooks = &env.webhooks;
            if hooks.urls.len() > MAX_WEBHOOK_URLS
                || !hooks.urls.iter().all(|u| valid_url_with(u, true))
            {
                return bad(format!(
                    "environments.{id}.webhooks.urls must hold at most {MAX_WEBHOOK_URLS} https \
                     URLs (http only on loopback)"
                ));
            }
            match &hooks.signing_secret_env {
                Some(name) if name.trim().is_empty() => {
                    return bad(format!(
                        "environments.{id}.webhooks.signing_secret_env must not be empty"
                    ));
                }
                None if !hooks.urls.is_empty() => {
                    return bad(format!(
                        "environments.{id}.webhooks.signing_secret_env is required with urls, \
                         so that the product can check each webhook"
                    ));
                }
                _ => {}
            }
            if hooks.retries > MAX_WEBHOOK_RETRIES {
                return bad(format!(
                    "environments.{id}.webhooks.retries must be 0 to {MAX_WEBHOOK_RETRIES}"
                ));
            }
        }
        Ok(())
    }

    /// Builds the key ring from the env vars that the config names.
    pub(crate) fn key_ring(&self, secrets: &Secrets) -> Result<KeyRing, PluginError> {
        let active = secrets.require(&self.encryption_key_env, "encryption key")?;
        let mut ring = KeyRing::from_base64(&self.encryption_key_id, &active)
            .map_err(|_| bad_key(&self.encryption_key_env))?;
        for (id, env) in &self.old_encryption_keys {
            let old = secrets.require(env, &format!("old encryption key {id}"))?;
            ring = ring.with_base64_key(id, &old).map_err(|_| bad_key(env))?;
        }
        if let Some(id) = &self.lookup_key_id {
            ring = ring
                .lookup_with(id)
                .map_err(|e| PluginError::config(format!("lookup_key_id: {e}")))?;
        }
        Ok(ring)
    }

    /// Builds the core environments from the env vars that the config names.
    pub(crate) fn environment_configs(
        &self,
        secrets: &Secrets,
    ) -> Result<Vec<EnvironmentConfig>, PluginError> {
        self.environments
            .iter()
            .map(|(id, settings)| {
                let key = secrets.require(
                    &settings.secret_key_env,
                    &format!("secret key for environment {id}"),
                )?;
                let hooks = &settings.webhooks;
                let signing_secret = hooks
                    .signing_secret_env
                    .as_deref()
                    .map(|name| {
                        secrets.require(
                            name,
                            &format!("webhook signing secret for environment {id}"),
                        )
                    })
                    .transpose()?
                    .map(Secret::new);
                let mut env = EnvironmentConfig::new(id.clone(), key);
                env.callback_url.clone_from(&settings.callback_url);
                env.webhooks = WebhookSettings {
                    primary_url: hooks.urls.first().cloned(),
                    secondary_url: hooks.urls.get(1).cloned(),
                    signing_secret,
                    retries: hooks.retries,
                    initial_backoff_ms: hooks.initial_backoff_ms,
                    send_auth: hooks.send_auth,
                    send_sync: hooks.send_sync,
                };
                Ok(env)
            })
            .collect()
    }

    /// Returns `base` (else the built-in catalog) with `providers_file`
    /// merged over it.
    pub(crate) fn catalog(&self, base: Option<Catalog>) -> Result<Catalog, PluginError> {
        let base = match base {
            Some(c) => c,
            None => Catalog::builtin()?,
        };
        match &self.providers_file {
            Some(file) => {
                let extra = Catalog::from_yaml_file(file)
                    .map_err(|e| PluginError::config(format!("providers_file: {e}")))?;
                Ok(base.merge(extra))
            }
            None => Ok(base),
        }
    }
}

/// The rule for key and environment ids, for error messages.
const ID_RULE: &str = "must be 1 or more characters of A-Z, a-z, 0-9, _ or -";

fn valid_id(id: &str, max: usize) -> bool {
    (1..=max).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn bad_key(env: &str) -> PluginError {
    PluginError::config(format!(
        "the env var {env} must hold a 32-byte key in base64"
    ))
}

/// An `https` URL with a host and no query or fragment. Plain `http` only on
/// a loopback host, for local tests.
fn valid_url(text: &str) -> bool {
    valid_url_with(text, false)
}

/// Same as [`valid_url`]. With `query`, the URL can have a query.
fn valid_url_with(text: &str, query: bool) -> bool {
    let Ok(u) = url::Url::parse(text) else {
        return false;
    };
    let loopback = match u.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => return false,
    };
    let scheme_ok = u.scheme() == "https" || (u.scheme() == "http" && loopback);
    scheme_ok && (query || u.query().is_none()) && u.fragment().is_none()
}

/// The schema for env values: the defaults, with an entry for any map key.
fn schema() -> Result<toml::Table, PluginError> {
    let mut sample = SanMateoConfig::default();
    sample
        .environments
        .insert(ANY_KEY.to_owned(), EnvironmentSettings::default());
    sample
        .old_encryption_keys
        .insert(ANY_KEY.to_owned(), String::new());
    toml::Table::try_from(sample).map_err(|e| PluginError::config(e.to_string()))
}

/// Finds secret values by env var name.
///
/// Order: values set on the plugin, the process environment, then `.env`.
/// An empty value counts as not set.
#[derive(Clone, Default)]
pub(crate) struct Secrets {
    overrides: BTreeMap<String, String>,
    dotenv: OnceLock<Vec<(String, String)>>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Names only. Values are secrets.
        f.debug_struct("Secrets")
            .field("overrides", &self.overrides.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Secrets {
    pub(crate) const fn new(overrides: BTreeMap<String, String>) -> Self {
        Self {
            overrides,
            dotenv: OnceLock::new(),
        }
    }

    /// The value of the env var `name`, if it is set and not empty.
    pub(crate) fn get(&self, name: &str) -> Option<String> {
        self.overrides
            .get(name)
            .map_or_else(
                || std::env::var(name).ok().or_else(|| self.dotenv_var(name)),
                |v| Some(v.clone()),
            )
            .filter(|v| !v.trim().is_empty())
    }

    /// The value of `name` in `.env`. The plugin reads the file one time.
    fn dotenv_var(&self, name: &str) -> Option<String> {
        self.dotenv
            .get_or_init(|| autumn_web::dotenv::resolve_process_dotenv().unwrap_or_default())
            .iter()
            .find_map(|(k, v)| (k == name).then(|| v.clone()))
    }

    /// The value of the env var `name`. The error names the var and `what`.
    pub(crate) fn require(&self, name: &str, what: &str) -> Result<String, PluginError> {
        self.get(name)
            .ok_or_else(|| PluginError::config(format!("no {what}; set the env var {name}")))
    }
}

/// Returns `.env` values, then the process environment (later wins).
///
/// It skips a variable whose name or value is not UTF-8.
fn process_env() -> Result<Vec<(String, String)>, PluginError> {
    let mut vars = autumn_web::dotenv::resolve_process_dotenv()
        .map_err(|e| PluginError::config(format!(".env: {e}")))?;
    vars.extend(
        std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?))),
    );
    Ok(vars)
}

/// The value of `--profile <name>` or `--profile=<name>` in the process args.
fn profile_flag() -> Option<String> {
    let args: Vec<String> = std::env::args_os()
        .filter_map(|a| a.into_string().ok())
        .collect();
    args.iter()
        .enumerate()
        .find_map(|(i, a)| {
            a.strip_prefix("--profile=").map(str::to_owned).or_else(|| {
                (a == "--profile")
                    .then(|| args.get(i + 1).cloned())
                    .flatten()
            })
        })
        // Like autumn-web: an empty value selects nothing.
        .filter(|p| !p.trim().is_empty())
}

fn read_optional(path: &Path) -> Result<Option<String>, PluginError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PluginError::config(format!("{}: {e}", path.display()))),
    }
}

/// Deep merge: tables merge by key; other values replace.
fn merge(into: &mut toml::Table, from: toml::Table) {
    for (key, value) in from {
        match (into.get_mut(&key), value) {
            (Some(toml::Value::Table(dst)), toml::Value::Table(src)) => merge(dst, src),
            (_, value) => {
                into.insert(key, value);
            }
        }
    }
}

fn insert_path(table: &mut toml::Table, path: &[String], value: toml::Value) {
    let Some((last, parents)) = path.split_last() else {
        return;
    };
    let mut cur = table;
    for key in parents {
        let entry = cur
            .entry(key.clone())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if !entry.is_table() {
            *entry = toml::Value::Table(toml::Table::new());
        }
        let toml::Value::Table(next) = entry else {
            return;
        };
        cur = next;
    }
    cur.insert(last.clone(), value);
}

/// Converts an env value to the type of the schema value at the same path.
/// A list is a comma-separated string.
fn typed_env_value(schema: &toml::Table, path: &[String], raw: &str) -> Option<toml::Value> {
    let mut node: Option<&toml::Value> = None;
    let mut table = Some(schema);
    for key in path {
        node = table.and_then(|t| t.get(key).or_else(|| t.get(ANY_KEY)));
        table = node.and_then(toml::Value::as_table);
    }
    match node {
        Some(toml::Value::Integer(_)) => raw.trim().parse().ok().map(toml::Value::Integer),
        Some(toml::Value::Boolean(_)) => raw.trim().parse().ok().map(toml::Value::Boolean),
        Some(toml::Value::Array(_)) => Some(toml::Value::Array(
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| toml::Value::String(s.to_owned()))
                .collect(),
        )),
        _ => Some(toml::Value::String(raw.to_owned())),
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
        let dir =
            std::env::temp_dir().join(format!("sm-plugin-config-{name}-{}", std::process::id()));
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
                (
                    "AUTUMN_SAN_MATEO__ENVIRONMENTS__PROD__SECRET_KEY_ENV",
                    "PROD_KEY",
                ),
                (
                    "AUTUMN_SAN_MATEO__ENVIRONMENTS__PROD__WEBHOOKS__URLS",
                    "https://h.example.com/a, https://h.example.com/b",
                ),
                (
                    "AUTUMN_SAN_MATEO__ENVIRONMENTS__PROD__WEBHOOKS__SIGNING_SECRET_ENV",
                    "PROD_WHSEC",
                ),
                (
                    "AUTUMN_SAN_MATEO__ENVIRONMENTS__PROD__WEBHOOKS__RETRIES",
                    "4",
                ),
                (
                    "AUTUMN_SAN_MATEO__ENVIRONMENTS__PROD__WEBHOOKS__SEND_SYNC",
                    "false",
                ),
                ("AUTUMN_SAN_MATEO__OLD_ENCRYPTION_KEYS__K0", "OLD_KEY"),
                ("AUTUMN_SAN_MATEO__BACKGROUND__ENABLED", "false"),
                ("AUTUMN_SAN_MATEO__BACKGROUND__REFRESH_BATCH", "7"),
                (
                    "AUTUMN_SAN_MATEO__PROXY__ALLOWED_HOSTS",
                    "a.example.com,b.example.com",
                ),
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
        assert_eq!(
            prod.webhooks.signing_secret_env.as_deref(),
            Some("PROD_WHSEC")
        );
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
        let c = SanMateoConfig::load_from_dir(&scratch_dir("empty"), &[], Vec::new()).unwrap();
        assert_eq!(c, SanMateoConfig::default());
    }

    #[test]
    fn inline_profile_section_is_not_read() {
        let text = "[san_mateo]\ndrain_timeout_secs = 1\n[profile.prod.san_mateo]\ndrain_timeout_secs = 3\n";
        let c = SanMateoConfig::from_toml_str(text).unwrap();
        assert_eq!(c.drain_timeout_secs, 1);
    }

    /// Applies `f` to the defaults and returns the validation error.
    fn check(f: fn(&mut SanMateoConfig)) -> String {
        let mut c = SanMateoConfig::default();
        f(&mut c);
        c.validate().unwrap_err().to_string()
    }

    fn default_env(c: &mut SanMateoConfig) -> &mut EnvironmentSettings {
        c.environments.get_mut(DEFAULT_ENVIRONMENT).unwrap()
    }

    #[test]
    fn validation_rules_for_urls_keys_and_loops() {
        assert!(check(|c| c.server_url = "ftp://x".into()).contains("server_url"));
        assert!(check(|c| c.server_url = "http://app.example.com".into()).contains("server_url"));
        assert!(
            check(|c| c.server_url = "https://x.example.com/?a=1".into()).contains("server_url")
        );
        assert!(check(|c| c.connect_url = Some("nope".into())).contains("connect_url"));
        assert!(check(|c| c.encryption_key_id = String::new()).contains("encryption_key_id"));
        assert!(check(|c| c.encryption_key_id = "a b".into()).contains("encryption_key_id"));
        assert!(check(|c| c.encryption_key_id = "k".repeat(33)).contains("encryption_key_id"));
        assert!(check(|c| c.encryption_key_env = " ".into()).contains("encryption_key_env"));
        for (id, env) in [("k1", "X"), ("bad id", "X"), ("k0", "")] {
            let mut c = SanMateoConfig::default();
            c.old_encryption_keys.insert(id.into(), env.into());
            let e = c.validate().unwrap_err().to_string();
            assert!(e.contains("old_encryption_keys"), "{e}");
        }
        assert!(check(|c| c.lookup_key_id = Some("k9".into())).contains("lookup_key_id"));
        assert!(check(|c| c.providers_file = Some(String::new())).contains("providers_file"));
        assert!(
            check(|c| c.proxy.allowed_hosts = vec!["https://x".into()]).contains("allowed_hosts")
        );
        assert!(check(|c| c.proxy.allowed_hosts = vec![String::new()]).contains("allowed_hosts"));
        assert!(check(|c| c.background.tick_secs = 0).contains("tick_secs"));
        assert!(check(|c| c.background.refresh_batch = 0).contains("refresh_batch"));
    }

    #[test]
    fn validation_rules_for_environments() {
        assert!(check(|c| c.environments.clear()).contains("environments"));
        assert!(
            check(|c| {
                c.environments
                    .insert("bad id".into(), EnvironmentSettings::default());
            })
            .contains("environments")
        );
        assert!(
            check(|c| default_env(c).secret_key_env = String::new()).contains("secret_key_env")
        );
        assert!(
            check(|c| {
                c.environments
                    .insert("other".into(), EnvironmentSettings::default());
            })
            .contains("secret_key_env")
        );
        assert!(check(|c| default_env(c).callback_url = Some("x".into())).contains("callback_url"));
        assert!(
            check(|c| default_env(c).webhooks.urls = vec!["https://h.example.com".into()])
                .contains("signing_secret_env")
        );
        assert!(
            check(|c| {
                let w = &mut default_env(c).webhooks;
                w.urls = vec!["https://a.example.com".into(); 3];
                w.signing_secret_env = Some("S".into());
            })
            .contains("urls")
        );
        assert!(
            check(|c| {
                let w = &mut default_env(c).webhooks;
                w.urls = vec!["http://hooks.example.com".into()];
                w.signing_secret_env = Some("S".into());
            })
            .contains("urls")
        );
        assert!(
            check(|c| default_env(c).webhooks.signing_secret_env = Some(String::new()))
                .contains("signing_secret_env")
        );
        assert!(
            check(|c| default_env(c).webhooks.retries = MAX_WEBHOOK_RETRIES + 1)
                .contains("retries")
        );
    }

    #[test]
    fn validation_accepts_edge_cases() {
        let mut c = SanMateoConfig {
            server_url: "http://127.0.0.1:8080".into(),
            connect_url: Some("https://connect.example.com/c".into()),
            old_encryption_keys: BTreeMap::from([("k0".into(), "OLD".into())]),
            lookup_key_id: Some("k0".into()),
            ..SanMateoConfig::default()
        };
        let w = &mut default_env(&mut c).webhooks;
        w.urls = vec![
            "https://a.example.com/h?team=1".into(),
            "http://localhost:9/h".into(),
        ];
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
        assert!(
            e.contains("SM_PLUGIN_TEST_NEVER_SET_1") && e.contains("encryption key"),
            "{e}"
        );
        let debug = format!("{s:?}");
        assert!(
            debug.contains("SM_TEST_A") && !debug.contains("from-plugin"),
            "{debug}"
        );
    }

    #[test]
    fn key_ring_reads_named_env_vars_and_supports_rotation() {
        let mut c = SanMateoConfig {
            encryption_key_env: "SM_KEY_NEW".into(),
            encryption_key_id: "k2".into(),
            ..SanMateoConfig::default()
        };
        let e = c.key_ring(&secrets(&[])).unwrap_err().to_string();
        assert!(e.contains("SM_KEY_NEW"), "{e}");
        let e = c
            .key_ring(&secrets(&[("SM_KEY_NEW", "not-base64-secret")]))
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("SM_KEY_NEW") && !e.contains("not-base64-secret"),
            "{e}"
        );
        // Data sealed with the old key opens after rotation.
        let old = KeyRing::from_base64("k1", OLD_KEY_B64).unwrap();
        let sealed = old.seal(b"token", b"row").unwrap();
        c.old_encryption_keys
            .insert("k1".into(), "SM_KEY_OLD".into());
        c.lookup_key_id = Some("k1".into());
        let e = c
            .key_ring(&secrets(&[("SM_KEY_NEW", KEY_B64)]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("SM_KEY_OLD"), "{e}");
        let ring = c
            .key_ring(&secrets(&[
                ("SM_KEY_NEW", KEY_B64),
                ("SM_KEY_OLD", OLD_KEY_B64),
            ]))
            .unwrap();
        assert_eq!(ring.active_key_id(), "k2");
        assert_eq!(ring.open(&sealed, b"row").unwrap(), b"token");
        assert!(ring.seal(b"x", b"row").unwrap().contains(".k2."));
        assert_eq!(
            ring.lookup_hash("v"),
            old.lookup_hash("v"),
            "lookup key stays k1"
        );
    }

    #[test]
    fn environments_resolve_secrets_and_webhooks() {
        let mut c = SanMateoConfig::default();
        let mut prod = EnvironmentSettings {
            secret_key_env: "SM_PROD_SK".into(),
            callback_url: Some("https://app.example.com/cb".into()),
            ..EnvironmentSettings::default()
        };
        prod.webhooks.urls = vec![
            "https://h.example.com/1".into(),
            "https://h.example.com/2".into(),
        ];
        prod.webhooks.signing_secret_env = Some("SM_PROD_WH".into());
        prod.webhooks.retries = 1;
        c.environments = BTreeMap::from([("prod".to_owned(), prod)]);
        let e = c
            .environment_configs(&secrets(&[]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("SM_PROD_SK") && e.contains("prod"), "{e}");
        let e = c
            .environment_configs(&secrets(&[("SM_PROD_SK", "sk_prod")]))
            .unwrap_err()
            .to_string();
        assert!(e.contains("SM_PROD_WH"), "{e}");
        let envs = c
            .environment_configs(&secrets(&[
                ("SM_PROD_SK", "sk_prod"),
                ("SM_PROD_WH", "whsec"),
            ]))
            .unwrap();
        assert_eq!(envs.len(), 1);
        let p = &envs[0];
        assert_eq!(p.id, "prod");
        assert_eq!(p.secret_key.expose(), "sk_prod");
        assert_eq!(
            p.callback_url.as_deref(),
            Some("https://app.example.com/cb")
        );
        assert_eq!(
            p.webhooks.primary_url.as_deref(),
            Some("https://h.example.com/1")
        );
        assert_eq!(
            p.webhooks.secondary_url.as_deref(),
            Some("https://h.example.com/2")
        );
        assert_eq!(
            p.webhooks.signing_secret.as_ref().unwrap().expose(),
            "whsec"
        );
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
