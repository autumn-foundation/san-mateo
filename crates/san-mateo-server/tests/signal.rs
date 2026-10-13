//! `shutdown_signal` ends on `SIGTERM`. This test has its own binary: the
//! signal goes to the whole process.

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

#[tokio::test]
async fn sigterm_ends_the_shutdown_future() {
    let waiter = tokio::spawn(san_mateo_server::shutdown_signal());
    // Wait until the handler is in place. With no handler, SIGTERM stops the
    // test process.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let status = std::process::Command::new("kill")
        .args(["-TERM", &std::process::id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
    tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .expect("the future ends")
        .unwrap();
}
