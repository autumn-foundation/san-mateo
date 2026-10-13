//! Shared helpers for the unit tests.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use san_mateo::{Catalog, EnvironmentConfig, KeyRing, MemoryTransport, SanMateo};

/// The secret key of the `dev` environment.
pub(crate) const SECRET: &str = "sk_test_unit_0123456789";

/// A service with one environment, no providers and a fake network.
pub(crate) fn service() -> SanMateo {
    SanMateo::builder()
        .server_url("https://sm.example.com")
        .environment(EnvironmentConfig::new("dev", SECRET))
        .key_ring(KeyRing::new("k1", [7; 32]).unwrap())
        .catalog(Catalog::default())
        .transport(Arc::new(MemoryTransport::new()))
        .build()
        .unwrap()
}
