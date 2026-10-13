//! Health indicator for `/actuator/health` and `/ready`.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;

use autumn_web::actuator::{HealthCheckOutput, HealthIndicator};

use crate::runtime::Cell;

/// The registration name of the indicator.
pub const INDICATOR_NAME: &str = "san_mateo";

/// Shows the state of the San Mateo service.
///
/// - `Up`: the service started.
/// - `Down`: the service did not start, start failed, or shutdown started.
///
/// The details hold counts and a short reason only. They never hold secrets.
pub struct SanMateoHealth {
    cell: Cell,
    failed: AtomicBool,
}

impl std::fmt::Debug for SanMateoHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = f;
        todo!()
    }
}

impl SanMateoHealth {
    pub(crate) const fn new(cell: Cell) -> Self {
        Self {
            cell,
            failed: AtomicBool::new(false),
        }
    }

    /// Records that the start failed.
    pub(crate) fn mark_failed(&self) {
        todo!()
    }

    /// The current result.
    #[must_use]
    pub fn status(&self) -> HealthCheckOutput {
        todo!()
    }
}

impl HealthIndicator for SanMateoHealth {
    fn check(&self) -> Pin<Box<dyn Future<Output = HealthCheckOutput> + Send + '_>> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use autumn_web::actuator::{HealthStatus, IndicatorGroup};
    use serde_json::json;

    use super::*;
    use crate::runtime::Runtime;
    use crate::test_support::{SECRET, service};

    #[tokio::test]
    async fn down_before_start_and_after_a_failed_start() {
        let h = SanMateoHealth::new(Arc::new(OnceLock::new()));
        let out = h.check().await;
        assert_eq!(out.status, HealthStatus::Down);
        assert_eq!(out.details["reason"], json!("not started"));
        assert!(format!("{h:?}").contains("started: false"), "{h:?}");
        h.mark_failed();
        let out = h.check().await;
        assert_eq!(out.status, HealthStatus::Down);
        assert_eq!(out.details["reason"], json!("start failed"));
        assert!(matches!(h.group(), IndicatorGroup::Readiness));
    }

    #[tokio::test]
    async fn up_when_started_and_down_after_shutdown() {
        let cell: Cell = Arc::new(OnceLock::new());
        let h = SanMateoHealth::new(Arc::clone(&cell));
        let san = service();
        let bg = san.start_background(san_mateo::BackgroundSettings::default());
        let rt = Arc::new(Runtime::new(san, Some(bg), Duration::from_secs(1)));
        let _ = cell.set(Arc::clone(&rt));
        let out = h.check().await;
        assert_eq!(out.status, HealthStatus::Up);
        assert_eq!(out.details["environments"], json!(1));
        assert_eq!(out.details["providers"], json!(0));
        assert_eq!(out.details["background"], json!(true));
        let text = serde_json::to_string(&out.details).unwrap();
        assert!(!text.contains(SECRET), "{text}");
        rt.shutdown().await;
        let out = h.status();
        assert_eq!(out.status, HealthStatus::Down);
        assert_eq!(out.details["reason"], json!("stopped"));
    }
}
