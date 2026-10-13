//! The `san-mateo` binary. See the crate docs of `san_mateo_server` for the
//! environment variables.

#[tokio::main]
async fn main() -> std::process::ExitCode {
    san_mateo_server::init_tracing();
    let code = san_mateo_server::main_exit_code(
        |k| std::env::var(k).ok(),
        san_mateo_server::shutdown_signal(),
    )
    .await;
    std::process::ExitCode::from(code)
}
