//! The `san-mateo` binary. See the crate docs of `san_mateo_server` for the
//! environment variables.

#[tokio::main]
async fn main() -> std::process::ExitCode {
    san_mateo_server::init_tracing();
    match san_mateo_server::ServerConfig::from_env(|k| std::env::var(k).ok()) {
        Ok(config) => match san_mateo_server::run(config, san_mateo_server::shutdown_signal()).await {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                tracing::error!(code = e.code(), error = %e, "the server stopped with an error");
                std::process::ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("san-mateo: {e}");
            std::process::ExitCode::from(2)
        }
    }
}
