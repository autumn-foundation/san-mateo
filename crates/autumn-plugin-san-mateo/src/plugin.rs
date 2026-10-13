//! `SanMateoPlugin` and the started runtime.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use autumn_web::app::AppBuilder;
use autumn_web::config::AutumnConfig;
use autumn_web::plugin::Plugin;
use autumn_web::plugin_contract::PluginContract;
use autumn_web::reexports::axum::Router;
use autumn_web::{AppState, ProcessRole};
use san_mateo::{
    ActionFunction, Catalog, HttpTransport, Registry, SanMateo, Store, SyncFunction,
};

use crate::config::SanMateoConfig;
use crate::error::PluginError;
use crate::health::SanMateoHealth;
use crate::runtime::{Cell, Runtime};
use crate::service::SanMateoService;

/// Plugin name for duplicate detection.
pub const PLUGIN_NAME: &str = "autumn-plugin-san-mateo";
/// Default route prefix.
pub const DEFAULT_PREFIX: &str = "/san-mateo";
/// The `autumn-web` versions that this plugin supports.
///
/// It must equal the `autumn-web` requirement in `Cargo.toml`. A unit test
/// checks this.
pub const AUTUMN_WEB_REQUIREMENT: &str = "0.8";

/// The contract that [`SanMateoPlugin`] declares to autumn.
#[must_use]
pub fn plugin_contract() -> PluginContract {
    todo!()
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
        let _ = f;
        todo!()
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
        todo!()
    }

    /// Makes the plugin with this config. It reads no config files.
    #[must_use]
    pub fn with_config(config: SanMateoConfig) -> Self {
        let _ = config;
        todo!()
    }

    /// Sets the route prefix. Default: `/san-mateo`. `"/"` mounts at the root.
    ///
    /// autumn mounts routes before it loads config. So this is a builder
    /// setting, not a config key.
    #[must_use]
    pub fn prefix(self, path: &str) -> Self {
        let _ = path;
        todo!()
    }

    /// Uses this service. The plugin then builds no service and reads no
    /// secrets. It still reads `[san_mateo]` for the background loops.
    #[must_use]
    pub fn service(self, san: SanMateo) -> Self {
        let _ = san;
        todo!()
    }

    /// Sets all syncs and actions.
    #[must_use]
    pub fn registry(self, registry: Registry) -> Self {
        let _ = registry;
        todo!()
    }

    /// Adds a sync.
    #[must_use]
    pub fn sync(self, function: impl SyncFunction) -> Self {
        let _ = function;
        todo!()
    }

    /// Adds an action.
    #[must_use]
    pub fn action(self, function: impl ActionFunction) -> Self {
        let _ = function;
        todo!()
    }

    /// Uses this catalog, not the built-in one. A `providers_file` still
    /// merges over it.
    #[must_use]
    pub fn catalog(self, catalog: Catalog) -> Self {
        let _ = catalog;
        todo!()
    }

    /// Uses this store. Default: a memory store, which loses all data at
    /// restart.
    #[must_use]
    pub fn store(self, store: Arc<dyn Store>) -> Self {
        let _ = store;
        todo!()
    }

    /// Uses this transport for provider calls and webhooks. Default: the
    /// reqwest transport.
    #[must_use]
    pub fn transport(self, transport: Arc<dyn HttpTransport>) -> Self {
        let _ = transport;
        todo!()
    }

    /// Turns the background loops on or off. This replaces
    /// `[san_mateo.background] enabled`.
    #[must_use]
    pub fn background(self, on: bool) -> Self {
        let _ = on;
        todo!()
    }

    /// Uses `value` for the env var `name`. The plugin then does not read
    /// `name` from the environment. Use it for tests, or for secrets from a
    /// vault.
    #[must_use]
    pub fn secret_env(self, name: impl Into<String>, value: impl Into<String>) -> Self {
        let _ = (name.into(), value.into());
        todo!()
    }

    /// The route prefix after normalization, for example `/san-mateo`. The
    /// root is `""`.
    #[must_use]
    pub fn route_prefix(&self) -> &str {
        todo!()
    }

    /// Starts the plugin outside an app build. Use it in tests. The app
    /// build does the same at startup.
    ///
    /// # Errors
    /// Returns a config error, or an error from the service build.
    pub fn start(self, state: &AppState) -> Result<SanMateoRuntime, PluginError> {
        let _ = state;
        todo!()
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
        let _ = (state, role);
        todo!()
    }
}

/// `"/san-mateo/"` and `"san-mateo"` become `"/san-mateo"`. `"/"` becomes
/// `""` (the root).
fn normalize_prefix(path: &str) -> String {
    let _ = path;
    todo!()
}

/// Same rule as autumn: an exact match, or a `/`-delimited subtree.
fn is_exempt(path: &str, exempt: &[String]) -> bool {
    let _ = (path, exempt);
    todo!()
}

/// Fails when CSRF or CAPTCHA would block a San Mateo route. A plugin cannot
/// add an exemption, so the error gives the config fix.
fn check_security(cfg: &AutumnConfig, prefix: &str) -> Result<(), PluginError> {
    let _ = (cfg, prefix);
    todo!()
}

/// A warning when the autumn CSP blocks the script of the hosted connect
/// page.
fn csp_warning(cfg: &AutumnConfig) -> Option<&'static str> {
    let _ = cfg;
    todo!()
}

/// A started plugin.
pub struct SanMateoRuntime {
    runtime: Arc<Runtime>,
    cell: Cell,
    health: Arc<SanMateoHealth>,
}

impl std::fmt::Debug for SanMateoRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl SanMateoRuntime {
    /// The service.
    #[must_use]
    pub fn service(&self) -> SanMateoService {
        todo!()
    }

    /// The routes, relative to the prefix. Use it in tests, or to mount the
    /// routes yourself.
    #[must_use]
    pub fn router(&self) -> Router {
        todo!()
    }

    /// The health indicator.
    #[must_use]
    pub fn health(&self) -> Arc<SanMateoHealth> {
        todo!()
    }

    /// `true` while the background loops run.
    #[must_use]
    pub fn background_running(&self) -> bool {
        todo!()
    }

    /// Stops the routes and the background loops. Then waits for webhooks
    /// and syncs, up to `drain_timeout_secs`. After this, the routes reply
    /// 503.
    pub async fn shutdown(&self) {
        todo!()
    }
}

impl Plugin for SanMateoPlugin {
    fn name(&self) -> Cow<'static, str> {
        todo!()
    }

    fn contract(&self) -> Option<PluginContract> {
        todo!()
    }

    fn build(self, app: AppBuilder) -> AppBuilder {
        let _ = app;
        todo!()
    }
}

/// The autumn shutdown hook: stop the started runtime, if any.
fn shutdown_hook(cell: &Cell) -> impl Future<Output = ()> + Send + use<> {
    let _ = cell;
    async {}
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
        assert!(e.contains("exempt_paths") && e.contains("[\"/san-mateo\"]"), "{e}");
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
    fn csrf_at_the_root_names_each_top_path() {
        let mut cfg = AutumnConfig::default();
        cfg.security.csrf.enabled = true;
        let e = check_security(&cfg, "").unwrap_err().to_string();
        assert!(e.contains("\"/integrations\"") && e.contains("\"/proxy\""), "{e}");
        assert!(!e.contains("[\"/\"]"), "{e}");
        cfg.security.csrf.exempt_paths = ex(&[
            "/integrations",
            "/connect",
            "/oauth2",
            "/api-auth",
            "/auth",
            "/connections",
            "/connection",
            "/proxy",
            "/sync",
            "/action",
        ]);
        check_security(&cfg, "").unwrap();
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
        assert!(e.contains("captcha_exempt_paths") && e.contains("[\"/sm\"]"), "{e}");
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
        let bg = san.start_background(san_mateo::BackgroundSettings::default());
        let rt = Arc::new(Runtime::new(san, Some(bg), std::time::Duration::from_secs(1)));
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
