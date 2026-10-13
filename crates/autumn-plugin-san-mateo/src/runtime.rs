//! The started state that routes, health and shutdown share.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use autumn_web::reexports::axum::Router;
use san_mateo::{Background, SanMateo};

/// The runtime, set once at startup. Routes reply 503 before that.
pub(crate) type Cell = Arc<OnceLock<Arc<Runtime>>>;

/// A started San Mateo service.
pub(crate) struct Runtime {
    /// The service.
    pub(crate) san: SanMateo,
    /// The core API router, built once.
    pub(crate) api: Router,
    /// The background loops, if they run.
    background: Mutex<Option<Background>>,
    /// `true` after shutdown starts. Routes then reply 503.
    closed: AtomicBool,
    /// The longest wait for webhooks and syncs at shutdown.
    drain: Duration,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("san", &self.san)
            .field("background", &self.background_running())
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl Runtime {
    /// Makes a runtime. The core router comes from `san`.
    pub(crate) fn new(san: SanMateo, background: Option<Background>, drain: Duration) -> Self {
        let api = san_mateo::api::router(san.clone());
        Self {
            san,
            api,
            background: Mutex::new(background),
            closed: AtomicBool::new(false),
            drain,
        }
    }

    /// Starts the background loops, if the config asks for them.
    pub(crate) fn start_loops(&self) {}

    /// `true` after shutdown starts.
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// `true` while the background loops run.
    pub(crate) fn background_running(&self) -> bool {
        self.loops().is_some()
    }

    fn loops(&self) -> std::sync::MutexGuard<'_, Option<Background>> {
        self.background
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Stops the routes and the background loops. Then waits for webhooks and
    /// syncs, up to the drain time.
    pub(crate) async fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        let loops = self.loops().take();
        if let Some(loops) = loops {
            loops.stop().await;
        }
        if tokio::time::timeout(self.drain, self.san.flush())
            .await
            .is_err()
        {
            tracing::warn!("san mateo: webhooks or syncs still run after the drain time");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::service;

    #[tokio::test]
    async fn shutdown_closes_and_stops_background() {
        let san = service();
        let bg = san.start_background(san_mateo::BackgroundSettings::default());
        let rt = Runtime::new(san, Some(bg), Duration::from_secs(1));
        assert!(!rt.is_closed());
        assert!(rt.background_running());
        assert!(format!("{rt:?}").contains("background: true"), "{rt:?}");
        rt.shutdown().await;
        assert!(rt.is_closed());
        assert!(!rt.background_running());
        // A second shutdown does nothing.
        rt.shutdown().await;
        let quiet = Runtime::new(service(), None, Duration::from_secs(1));
        assert!(!quiet.background_running());
        quiet.shutdown().await;
        assert!(quiet.is_closed());
    }
}
