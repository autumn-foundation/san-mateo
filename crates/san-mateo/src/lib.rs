//! San Mateo: an integration auth platform in Rust.
//!
//! San Mateo keeps OAuth tokens and API keys for the users of a product. It
//! refreshes tokens. It sends authorized calls to provider APIs through a
//! proxy. It runs Rust syncs and actions, and it keeps the records that syncs
//! make. The API follows the Nango public API.
//!
//! ```no_run
//! use san_mateo::{EnvironmentConfig, KeyRing, SanMateo};
//!
//! # async fn demo() -> san_mateo::Result<()> {
//! let san = SanMateo::builder()
//!     .server_url("https://sm.example.com")
//!     .environment(EnvironmentConfig::new("prod", "sk_live_..."))
//!     .key_ring(KeyRing::from_base64("k1", "<32 random bytes, base64>")?)
//!     .build()?;
//! let app = san_mateo::api::router(san);
//! # let _ = app;
//! # Ok(())
//! # }
//! ```

pub mod api;
pub mod clock;
pub mod credentials;
pub mod crypto;
pub mod error;
pub mod functions;
pub mod model;
pub mod oauth2;
pub mod paginate;
pub mod policy;
pub mod provider;
pub mod proxy;
pub mod records;
pub mod secret;
pub mod service;
pub mod store;
pub mod template;
pub mod transport;
pub mod webhook;

pub use credentials::Credentials;
pub use crypto::KeyRing;
pub use error::{Error, Result};
pub use functions::{
    ActionContext, ActionFunction, ActionSpec, PaginateOptions, Registry, SyncContext,
    SyncFunction, SyncSpec,
};
pub use model::{ConnectionKey, EndUser, IntegrationDefaults, Organization, Tags};
pub use provider::{AuthMode, Catalog, Provider};
pub use proxy::{ProxyRequest, ProxyResponse, ProxySettings};
pub use secret::Secret;
pub use service::{
    Background, BackgroundSettings, Config, ConnectOptions, ConnectSession, ConnectSessionCreated,
    ConnectSessionInput, Connection, ConnectionPatch, ConnectionSummary, EnvironmentConfig,
    GetOptions, ImportConnection, Integration, IntegrationPatch, NewIntegration, RecordPage,
    RecordsRequest, RefreshReport, SanMateo, SanMateoBuilder, SyncRunReport, SyncStatusEntry,
};
pub use store::{MemoryStore, Store};
pub use transport::{HttpTransport, MemoryTransport, ReqwestTransport};
pub use webhook::WebhookSettings;
