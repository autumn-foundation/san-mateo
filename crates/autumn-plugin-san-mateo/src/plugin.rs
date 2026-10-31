//! `SanMateoPlugin` and the started runtime.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use autumn_web::actuator::HealthIndicator;
use autumn_web::app::AppBuilder;
use autumn_web::config::AutumnConfig;
use autumn_web::plugin::Plugin;
use autumn_web::plugin_contract::PluginContract;
use autumn_web::reexports::axum::Router;
use autumn_web::{AppState, AutumnError, ProcessRole};
use san_mateo::{ActionFunction, Catalog, HttpTransport, Registry, SanMateo, Store, SyncFunction};

use crate::config::{SECTION, SanMateoConfig, Secrets};
use crate::error::PluginError;
use crate::health::{INDICATOR_NAME, SanMateoHealth};
use crate::routes;
use crate::runtime::{Cell, Runtime};
use crate::service::SanMateoService;

/// The plugin name. autumn uses it to find a plugin that is added two times.
pub const PLUGIN_NAME: &str = "autumn-plugin-san-mateo";
/// Default route prefix.
pub const DEFAULT_PREFIX: &str = "/san-mateo";
/// The `autumn-web` versions that this plugin supports.
///
/// It must equal the `autumn-web` requirement in `Cargo.toml`. A unit test
/// checks this.
pub const AUTUMN_WEB_REQUIREMENT: &str = "0.9";

/// The contract that [`SanMateoPlugin`] declares to autumn.
#[must_use]
pub fn plugin_contract() -> PluginContract {
    PluginContract::new(env!("CARGO_PKG_NAME"))
        .plugin_version(env!("CARGO_PKG_VERSION"))
        .autumn_web(AUTUMN_WEB_REQUIREMENT)
}

/// San Mateo plugin for autumn-web.
///
/// ```rust,ignore
/// autumn_web::app()
///     .routes(routes![index])
///     .plugin(SanMateoPlugin::new().prefix("/integrations-api").sync(ContactsSync))
///     .run()
///     .await;
/// ```
pub struct SanMateoPlugin {
    config: Option<SanMateoConfig>,
    prefix: String,
    service: Option<SanMateo>,
    registry: Option<Registry>,
    catalog: Option<Catalog>,
    store: Option<Arc<dyn Store>>,
    transport: Option<Arc<dyn HttpTransport>>,
    background: Option<bool>,
    secrets: BTreeMap<String, String>,
}

impl std::fmt::Debug for SanMateoPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Env var names only. The values are secrets.
        f.debug_struct("SanMateoPlugin")
            .field("prefix", &self.prefix)
            .field("service", &self.service.is_some())
            .field("registry", &self.registry)
            .field("background", &self.background)
            .field("secret_envs", &self.secrets.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Default for SanMateoPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl SanMateoPlugin {
    /// Makes the plugin. It reads `[san_mateo]` at startup.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: None,
            prefix: DEFAULT_PREFIX.to_owned(),
            service: None,
            registry: None,
            catalog: None,
            store: None,
            transport: None,
            background: None,
            secrets: BTreeMap::new(),
        }
    }

    /// Makes the plugin with this config. It reads no config files.
    #[must_use]
    pub fn with_config(config: SanMateoConfig) -> Self {
        Self {
            config: Some(config),
            ..Self::new()
        }
    }

    /// Sets the route prefix. Default: `/san-mateo`. `"/"` mounts at the root.
    ///
    /// autumn mounts routes before it loads config. So this is a builder
    /// setting, not a config key.
    #[must_use]
    pub fn prefix(mut self, path: &str) -> Self {
        self.prefix = normalize_prefix(path);
        self
    }

    /// Uses this service. The plugin then builds no service and reads no
    /// secrets. It still reads `[san_mateo]` for the background loops.
    #[must_use]
    pub fn service(mut self, san: SanMateo) -> Self {
        self.service = Some(san);
        self
    }

    /// Sets all syncs and actions.
    #[must_use]
    pub fn registry(mut self, registry: Registry) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Adds a sync.
    #[must_use]
    pub fn sync(mut self, function: impl SyncFunction) -> Self {
        self.registry = Some(self.registry.take().unwrap_or_default().sync(function));
        self
    }

    /// Adds an action.
    #[must_use]
    pub fn action(mut self, function: impl ActionFunction) -> Self {
        self.registry = Some(self.registry.take().unwrap_or_default().action(function));
        self
    }

    /// Uses this catalog, not the built-in one. A `providers_file` still
    /// merges over it.
    #[must_use]
    pub fn catalog(mut self, catalog: Catalog) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// Uses this store. Default: a memory store, which loses all data at
    /// restart.
    #[must_use]
    pub fn store(mut self, store: Arc<dyn Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// Uses this transport for provider calls and webhooks. Default: the
    /// reqwest transport.
    #[must_use]
    pub fn transport(mut self, transport: Arc<dyn HttpTransport>) -> Self {
        self.transport = Some(transport);
        self
    }

    /// Turns the background loops on or off. This replaces
    /// `[san_mateo.background] enabled`.
    #[must_use]
    pub const fn background(mut self, on: bool) -> Self {
        self.background = Some(on);
        self
    }

    /// Uses `value` for the env var `name`. The plugin then does not read
    /// `name` from the environment. Use it for tests, or for secrets from a
    /// vault.
    #[must_use]
    pub fn secret_env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.secrets.insert(name.into(), value.into());
        self
    }

    /// The normalized route prefix, for example `/san-mateo`. The root is
    /// `""`.
    #[must_use]
    pub fn route_prefix(&self) -> &str {
        &self.prefix
    }

    /// Starts the plugin outside an app build. Use it in tests. The app
    /// build does the same at startup.
    ///
    /// # Errors
    /// Returns a config error, or an error from the service build.
    pub fn start(self, state: &AppState) -> Result<SanMateoRuntime, PluginError> {
        let role = state.role();
        self.start_with_role(state, role)
    }

    /// Starts the plugin as if the process has `role`. A role that serves no
    /// HTTP skips the CSRF check. A role that runs no workers runs no
    /// background loops.
    ///
    /// # Errors
    /// Same as [`Self::start`].
    pub fn start_with_role(
        self,
        state: &AppState,
        role: ProcessRole,
    ) -> Result<SanMateoRuntime, PluginError> {
        let cell: Cell = Arc::new(OnceLock::new());
        let health = Arc::new(SanMateoHealth::new(Arc::clone(&cell)));
        let runtime = self.start_inner(state, role)?;
        runtime.start_loops();
        let _ = cell.set(Arc::clone(&runtime));
        Ok(SanMateoRuntime {
            runtime,
            cell,
            health,
        })
    }

    fn start_inner(self, state: &AppState, role: ProcessRole) -> Result<Arc<Runtime>, PluginError> {
        let config = match &self.config {
            Some(c) => {
                c.validate()?;
                c.clone()
            }
            // autumn sets the state profile from the config; "default" means none.
            None => SanMateoConfig::load(Some(state.profile()).filter(|p| *p != "default"))?,
        };
        if role.serves_http() {
            let app_config = state.config();
            check_security(&app_config, &self.prefix)?;
            if let Some(warning) = csp_warning(&app_config) {
                tracing::warn!("san mateo: {warning}");
            }
        }
        let prefix = self.prefix.clone();
        let loops_on = self.background.unwrap_or(config.background.enabled) && role.runs_workers();
        let san = self.into_service(&config)?;
        state.insert_extension(SanMateoService::new(san.clone()));
        tracing::info!(
            prefix = %prefix,
            environments = san.config().environments.len(),
            providers = san.catalog().len(),
            background = loops_on,
            "san mateo started"
        );
        // The loops start in the startup hook, not here: autumn also runs
        // state initializers in modes that serve nothing, such as a build.
        Ok(Arc::new(Runtime::new(
            san,
            loops_on.then(|| config.background.settings()),
            Duration::from_secs(config.drain_timeout_secs),
            state.config().security.upload.max_request_size_bytes,
        )))
    }

    /// The given service, or a new service from the config and the secrets.
    fn into_service(self, config: &SanMateoConfig) -> Result<SanMateo, PluginError> {
        if let Some(san) = self.service {
            if let Some(warning) = server_url_warning(&san.config().server_url, &self.prefix) {
                tracing::warn!("san mateo: {warning}");
            }
            let unused: Vec<&str> = [
                ("registry", self.registry.is_some()),
                ("catalog", self.catalog.is_some()),
                ("store", self.store.is_some()),
                ("transport", self.transport.is_some()),
            ]
            .into_iter()
            .filter_map(|(name, set)| set.then_some(name))
            .collect();
            if !unused.is_empty() {
                return Err(PluginError::config(format!(
                    "these plugin options do not apply with .service(): {}. \
                     Set them on the SanMateo builder.",
                    unused.join(", ")
                )));
            }
            return Ok(san);
        }
        let secrets = Secrets::new(self.secrets);
        let core = san_mateo::Config {
            server_url: format!("{}{}", config.server_url.trim_end_matches('/'), self.prefix),
            connect_url: config.connect_url.clone(),
            environments: config.environment_configs(&secrets)?,
            proxy: config.proxy.settings(),
            ..san_mateo::Config::default()
        };
        let mut builder = SanMateo::builder()
            .config(core)
            .key_ring(config.key_ring(&secrets)?)
            .catalog(config.catalog(self.catalog)?);
        if let Some(store) = self.store {
            builder = builder.store(store);
        } else {
            tracing::warn!("san mateo: a memory store; all data is lost at restart");
        }
        if let Some(transport) = self.transport {
            builder = builder.transport(transport);
        }
        if let Some(registry) = self.registry {
            builder = builder.registry(registry);
        }
        Ok(builder.build()?)
    }
}

/// `"/san-mateo/"` and `"san-mateo"` become `"/san-mateo"`. `"/"` becomes
/// `""` (the root).
fn normalize_prefix(path: &str) -> String {
    let trimmed = path.trim().trim_matches('/');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("/{trimmed}")
    }
}

/// Same rule as autumn: an exact match, or a `/`-delimited subtree.
fn is_exempt(path: &str, exempt: &[String]) -> bool {
    exempt.iter().any(|p| {
        path == p
            || path
                .strip_prefix(p.as_str())
                .is_some_and(|rest| p.ends_with('/') || rest.starts_with('/'))
    })
}

/// The routes that change data: the CSRF and CAPTCHA layers check them.
fn mutating_routes(cfg: &AutumnConfig, prefix: &str) -> Vec<(&'static str, String)> {
    routes::mounted_specs(prefix.is_empty())
        .into_iter()
        .filter(|r| {
            r.method == "ANY"
                || !cfg
                    .security
                    .csrf
                    .safe_methods
                    .iter()
                    .any(|m| m.eq_ignore_ascii_case(r.method))
        })
        .map(|r| (r.method, format!("{prefix}{}", r.path)))
        .collect()
}

/// A warning when the `server_url` of a given service does not end with the
/// mount prefix: the OAuth callback URL then gives 404.
fn server_url_warning(server_url: &str, prefix: &str) -> Option<String> {
    let path = url::Url::parse(server_url).map_or_else(
        |_| String::new(),
        |u| u.path().trim_end_matches('/').to_owned(),
    );
    (!path.ends_with(prefix)).then(|| {
        format!(
            "the server_url {server_url} of the given service does not end with the mount \
             prefix {prefix}: OAuth callbacks and connect links will give 404"
        )
    })
}

/// The exempt list to add. At the root, name the static prefix of each
/// route that changes data. `"/"` or a top segment such as `"/auth"` would
/// also exempt routes of the app.
fn exemption_fix(prefix: &str) -> String {
    if !prefix.is_empty() {
        return format!("[\"{prefix}\"]");
    }
    let statics: BTreeSet<String> = routes::mounted_specs(true)
        .into_iter()
        .filter(|r| r.method != "GET")
        .map(|r| {
            let end = r.path.find("/{").unwrap_or(r.path.len());
            r.path[..end].to_owned()
        })
        .collect();
    // Leave out a path that a shorter one in the list covers.
    let all: Vec<String> = statics.iter().cloned().collect();
    let fix: Vec<String> = statics
        .into_iter()
        .filter(|p| {
            !all.iter()
                .any(|q| q != p && is_exempt(p, std::slice::from_ref(q)))
        })
        .map(|p| format!("\"{p}\""))
        .collect();
    format!("[{}]", fix.join(", "))
}

/// Fails when CSRF or CAPTCHA blocks a San Mateo route. A plugin cannot
/// add an exemption, so the error gives the config fix.
fn check_security(cfg: &AutumnConfig, prefix: &str) -> Result<(), PluginError> {
    let webhook_paths = cfg
        .security
        .webhooks
        .endpoints
        .iter()
        .map(|e| e.path.clone());
    let mutating = mutating_routes(cfg, prefix);
    if cfg.security.csrf.enabled {
        let mut exempt = cfg.security.csrf.exempt_paths.clone();
        exempt.extend(webhook_paths.clone());
        if let Some((method, path)) = mutating.iter().find(|(_, p)| !is_exempt(p, &exempt)) {
            return Err(PluginError::config(format!(
                "CSRF protection blocks {method} {path}. San Mateo clients send a secret key \
                 or a connect session token, not a CSRF cookie. Add {} to [security.csrf] \
                 exempt_paths. San Mateo checks its own tokens on these routes.",
                exemption_fix(prefix)
            )));
        }
    }
    // autumn checks CAPTCHA only on form bodies, and not with `dev_bypass`.
    let bot = &cfg.bot_protection;
    if bot.enabled && !bot.dev_bypass {
        let mut exempt = cfg.security.captcha_exempt_paths.clone();
        exempt.extend(webhook_paths);
        if let Some((method, path)) = mutating.iter().find(|(_, p)| !is_exempt(p, &exempt)) {
            return Err(PluginError::config(format!(
                "Bot protection blocks {method} {path} for form bodies, for example a proxy \
                 call. San Mateo clients send no CAPTCHA token. Add {} to [security] \
                 captcha_exempt_paths.",
                exemption_fix(prefix)
            )));
        }
    }
    Ok(())
}

/// A warning when the autumn CSP blocks the script of the hosted connect
/// page.
fn csp_warning(cfg: &AutumnConfig) -> Option<&'static str> {
    let headers = &cfg.security.headers;
    let nonce_in_policy = headers.csp_nonce.enabled
        && headers.content_security_policy
            == autumn_web::security::default_content_security_policy();
    (!nonce_in_policy).then_some(
        "the autumn Content-Security-Policy blocks the script of the hosted connect page. \
         Set [security.headers.csp_nonce] enabled = true and keep the default \
         content_security_policy, or use your own connect page.",
    )
}

/// A started plugin.
pub struct SanMateoRuntime {
    runtime: Arc<Runtime>,
    cell: Cell,
    health: Arc<SanMateoHealth>,
}

impl std::fmt::Debug for SanMateoRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SanMateoRuntime")
            .field("runtime", &self.runtime)
            .finish_non_exhaustive()
    }
}

impl SanMateoRuntime {
    /// The service.
    #[must_use]
    pub fn service(&self) -> SanMateoService {
        SanMateoService::new(self.runtime.san.clone())
    }

    /// The routes, relative to the prefix. Use it in tests, or to mount the
    /// routes yourself.
    pub fn router(&self) -> Router {
        routes::mounted_router(&self.cell, false)
    }

    /// The health indicator.
    #[must_use]
    pub fn health(&self) -> Arc<SanMateoHealth> {
        Arc::clone(&self.health)
    }

    /// `true` while the background loops run.
    #[must_use]
    pub fn background_running(&self) -> bool {
        self.runtime.background_running()
    }

    /// Stops the routes and the background loops. Then waits for webhooks
    /// and syncs, up to `drain_timeout_secs`. After this, the routes reply
    /// 503.
    pub async fn shutdown(&self) {
        self.runtime.shutdown().await;
    }
}

impl Plugin for SanMateoPlugin {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed(PLUGIN_NAME)
    }

    fn contract(&self) -> Option<PluginContract> {
        Some(plugin_contract())
    }

    fn build(self, app: AppBuilder) -> AppBuilder {
        let prefix = self.prefix.clone();
        let cell: Cell = Arc::new(OnceLock::new());
        let health = Arc::new(SanMateoHealth::new(Arc::clone(&cell)));
        let failure: Arc<Mutex<Option<PluginError>>> = Arc::default();
        let router = routes::mounted_router(&cell, prefix.is_empty());
        let app = app
            .config_section(SECTION)
            .health_indicator(
                INDICATOR_NAME,
                Arc::clone(&health) as Arc<dyn HealthIndicator>,
            )
            .declare_plugin_routes(routes::route_infos(&prefix));
        // axum rejects a nest at the root, so the root uses a merge.
        let app = if prefix.is_empty() {
            app.merge(router)
        } else {
            app.nest(&prefix, router)
        };
        let (init_cell, init_failure) = (Arc::clone(&cell), Arc::clone(&failure));
        let (check_cell, stop_cell) = (Arc::clone(&cell), Arc::clone(&cell));
        // Start before job workers, so a `#[job]` can use `SanMateoService`.
        app.state_initializer(move |state| match self.start_inner(state, state.role()) {
            Ok(rt) => {
                let _ = init_cell.set(rt);
            }
            Err(e) => {
                // Also log: some autumn modes run no startup hook.
                tracing::error!(error = %e, "san mateo plugin did not start");
                health.mark_failed();
                *init_failure
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(e);
            }
        })
        // An initializer cannot fail the boot. This hook reports its error.
        .on_startup(move |_state| {
            let failed = failure
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            let runtime = check_cell.get();
            if let Some(rt) = runtime {
                rt.start_loops();
            }
            let started = runtime.is_some();
            async move {
                match failed {
                    Some(e) => Err(AutumnError::internal_server_error(e)),
                    None if !started => Err(AutumnError::internal_server_error(
                        PluginError::config("the san_mateo plugin did not start"),
                    )),
                    None => Ok(()),
                }
            }
        })
        .on_shutdown(move || shutdown_hook(&stop_cell))
    }
}

/// The autumn shutdown hook: stop the started runtime, if any.
fn shutdown_hook(cell: &Cell) -> impl Future<Output = ()> + Send + use<> {
    let runtime = cell.get().cloned();
    async move {
        if let Some(runtime) = runtime {
            runtime.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use super::*;
    use crate::test_support::service;

    fn ex(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn prefix_normalizes() {
        assert_eq!(normalize_prefix("/san-mateo/"), "/san-mateo");
        assert_eq!(normalize_prefix("api/sm"), "/api/sm");
        assert_eq!(normalize_prefix(" /x "), "/x");
        assert_eq!(normalize_prefix("/"), "");
        assert_eq!(normalize_prefix(""), "");
        assert_eq!(SanMateoPlugin::new().route_prefix(), DEFAULT_PREFIX);
        assert_eq!(SanMateoPlugin::new().prefix("/").route_prefix(), "");
    }

    #[test]
    fn exempt_rule_matches_autumn() {
        assert!(is_exempt("/sm/integrations", &ex(&["/sm"])));
        assert!(is_exempt("/sm/integrations", &ex(&["/sm/"])));
        assert!(is_exempt("/sm", &ex(&["/sm"])));
        assert!(!is_exempt("/sm/integrations", &ex(&["/s"])));
        assert!(!is_exempt("/smx/integrations", &ex(&["/sm"])));
        assert!(!is_exempt("/sm/integrations", &ex(&[])));
        // Like autumn: an empty entry exempts all paths.
        assert!(is_exempt("/sm/integrations", &ex(&[""])));
    }

    #[test]
    fn csrf_needs_an_exemption_and_the_error_gives_the_fix() {
        let mut cfg = AutumnConfig::default();
        check_security(&cfg, "/san-mateo").unwrap();
        cfg.security.csrf.enabled = true;
        let e = check_security(&cfg, "/san-mateo").unwrap_err().to_string();
        assert!(
            e.contains("exempt_paths") && e.contains("[\"/san-mateo\"]"),
            "{e}"
        );
        assert!(e.contains("[security.csrf]"), "{e}");
        cfg.security.csrf.exempt_paths = ex(&["/san-mateo"]);
        check_security(&cfg, "/san-mateo").unwrap();
        cfg.security.csrf.exempt_paths = ex(&["/san-mateo/"]);
        check_security(&cfg, "/san-mateo").unwrap();
        cfg.security.csrf.exempt_paths = ex(&["/san"]);
        assert!(check_security(&cfg, "/san-mateo").is_err());
        // Only one route exempt: the error names another route.
        cfg.security.csrf.exempt_paths = ex(&["/san-mateo/integrations"]);
        let e = check_security(&cfg, "/san-mateo").unwrap_err().to_string();
        assert!(e.contains("POST /san-mateo/connect/sessions"), "{e}");
        // Webhook endpoint paths are exempt in autumn too.
        cfg.security.csrf.exempt_paths.clear();
        cfg.security
            .webhooks
            .endpoints
            .push(autumn_web::webhook::WebhookEndpointConfig::slack(
                "s",
                "/san-mateo",
                "S",
            ));
        check_security(&cfg, "/san-mateo").unwrap();
    }

    #[test]
    fn csrf_at_the_root_names_each_route_prefix() {
        let mut cfg = AutumnConfig::default();
        cfg.security.csrf.enabled = true;
        let e = check_security(&cfg, "").unwrap_err().to_string();
        assert!(
            e.contains("\"/integrations\"") && e.contains("\"/proxy\""),
            "{e}"
        );
        assert!(!e.contains("[\"/\"]"), "{e}");
        // A top segment such as `/auth` would also exempt the app's own
        // `/auth/login`. The fix names the San Mateo prefixes only.
        assert!(e.contains("\"/auth/unauthenticated\""), "{e}");
        assert!(!e.contains("\"/auth\""), "{e}");
        let list = &e[e.find('[').unwrap() + 1..e.find(']').unwrap()];
        let fix: Vec<&str> = list.split(", ").map(|p| p.trim_matches('"')).collect();
        cfg.security.csrf.exempt_paths = ex(&fix);
        check_security(&cfg, "").unwrap();
        assert!(!is_exempt("/auth/login", &cfg.security.csrf.exempt_paths));
    }

    #[test]
    fn csrf_safe_methods_count() {
        let mut cfg = AutumnConfig::default();
        cfg.security.csrf.enabled = true;
        cfg.security.csrf.safe_methods = ex(&["GET", "POST", "PATCH", "DELETE"]);
        // `ANY` (the proxy) is never safe.
        let e = check_security(&cfg, "/sm").unwrap_err().to_string();
        assert!(e.contains("/sm/proxy"), "{e}");
    }

    #[test]
    fn captcha_needs_an_exemption() {
        let mut cfg = AutumnConfig::default();
        cfg.bot_protection.enabled = true;
        let e = check_security(&cfg, "/sm").unwrap_err().to_string();
        assert!(
            e.contains("captcha_exempt_paths") && e.contains("[\"/sm\"]"),
            "{e}"
        );
        cfg.bot_protection.dev_bypass = true;
        check_security(&cfg, "/sm").unwrap();
        cfg.bot_protection.dev_bypass = false;
        cfg.security.captcha_exempt_paths = ex(&["/sm"]);
        check_security(&cfg, "/sm").unwrap();
    }

    #[test]
    fn csp_warning_names_the_fix() {
        let mut cfg = AutumnConfig::default();
        let w = csp_warning(&cfg).unwrap();
        assert!(w.contains("csp_nonce"), "{w}");
        cfg.security.headers.csp_nonce.enabled = true;
        assert!(csp_warning(&cfg).is_none());
        cfg.security.headers.content_security_policy = "default-src 'self'".to_owned();
        assert!(csp_warning(&cfg).is_some());
    }

    /// The declared range is the `autumn-web` requirement in `Cargo.toml`.
    #[test]
    fn contract_range_matches_the_cargo_toml_requirement() {
        let manifest: toml::Table = toml::from_str(include_str!("../Cargo.toml")).unwrap();
        let requirement = manifest["dependencies"]["autumn-web"]["version"]
            .as_str()
            .unwrap();
        assert_eq!(AUTUMN_WEB_REQUIREMENT, requirement);
    }

    /// The contract accepts the `autumn-web` that this crate compiles against.
    #[test]
    fn contract_accepts_the_compiled_autumn_web() {
        use autumn_web::plugin_contract::{AUTUMN_WEB_VERSION, ContractVerdict, evaluate};
        let contract = SanMateoPlugin::new().contract().unwrap();
        assert_eq!(contract, plugin_contract());
        assert_eq!(contract.plugin, PLUGIN_NAME);
        assert_eq!(
            contract.plugin_version.as_deref(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(
            evaluate(&contract, AUTUMN_WEB_VERSION),
            ContractVerdict::Compatible
        );
        assert_eq!(SanMateoPlugin::new().name(), PLUGIN_NAME);
    }

    #[tokio::test]
    async fn shutdown_hook_stops_the_started_runtime() {
        // No runtime: the hook ends at once.
        shutdown_hook(&Arc::new(OnceLock::new())).await;
        let cell: Cell = Arc::new(OnceLock::new());
        let san = service();
        let rt = Arc::new(Runtime::new(
            san,
            Some(san_mateo::BackgroundSettings::default()),
            std::time::Duration::from_secs(1),
            1 << 20,
        ));
        rt.start_loops();
        let _ = cell.set(Arc::clone(&rt));
        shutdown_hook(&cell).await;
        assert!(rt.is_closed());
        assert!(!rt.background_running());
    }

    #[test]
    fn service_option_rejects_build_options() {
        let state = AppState::for_test();
        let e = SanMateoPlugin::with_config(SanMateoConfig::default())
            .service(service())
            .catalog(Catalog::default())
            .registry(Registry::new())
            .start_with_role(&state, ProcessRole::Worker)
            .unwrap_err()
            .to_string();
        assert!(e.contains("catalog") && e.contains("registry"), "{e}");
    }

    #[tokio::test]
    async fn loops_start_at_startup_not_in_the_state_initializer() {
        // autumn runs state initializers also in modes that serve nothing,
        // for example `autumn build`. Loops must not run there.
        let state = AppState::for_test();
        let rt = SanMateoPlugin::with_config(SanMateoConfig::default())
            .service(service())
            .start_inner(&state, ProcessRole::Worker)
            .unwrap();
        assert!(!rt.background_running());
        rt.start_loops();
        assert!(rt.background_running());
        rt.start_loops();
        assert!(rt.background_running(), "a second start does nothing");
        rt.shutdown().await;
    }

    #[test]
    fn a_given_service_must_use_the_prefix_in_its_server_url() {
        assert!(server_url_warning("https://sm.example.com", "/san-mateo").is_some());
        assert!(server_url_warning("https://app.example.com/san-mateo", "/san-mateo").is_none());
        assert!(server_url_warning("https://app.example.com/san-mateo/", "/san-mateo").is_none());
        assert!(server_url_warning("https://app.example.com", "").is_none());
    }

    #[test]
    fn debug_hides_secret_values() {
        let p = SanMateoPlugin::new()
            .secret_env("SM_KEY", "very-secret-value")
            .background(false);
        let d = format!("{p:?}");
        assert!(d.contains("SanMateoPlugin") && d.contains("SM_KEY"), "{d}");
        assert!(!d.contains("very-secret-value"), "{d}");
    }
}
