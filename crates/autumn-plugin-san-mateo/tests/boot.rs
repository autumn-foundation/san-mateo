//! Boots a real app (not `TestApp`) with config files and env vars.
//!
//! The test runs this test binary again as a child process, with the env
//! vars set. So it changes no env in this process.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use autumn_plugin_san_mateo::{SanMateoPlugin, SanMateoService};

const CHILD_MARK: &str = "SAN_MATEO_BOOT_TEST_CHILD";
const SECRET: &str = "sk_test_boot_prod_0123456789";
const KEY_B64: &str = "ZGVmZ2hpamtsbW5vcHFyc3R1dnd4eXp7fH1+f4CBgoM=";

#[autumn_web::get("/")]
async fn index() -> &'static str {
    "app"
}

/// The server. It runs only in the child process.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "runs only as the child of the boot tests"]
async fn boot_child_server() {
    if std::env::var(CHILD_MARK).is_err() {
        return;
    }
    autumn_web::app()
        .routes(autumn_web::routes![index])
        .plugin(SanMateoPlugin::new())
        // autumn runs state initializers before job workers and startup
        // hooks. The service must be ready here, so a `#[job]` can use it.
        .state_initializer(|state| {
            let expect = std::env::var("SAN_MATEO_BOOT_EXPECT_SERVICE").is_ok();
            if expect && SanMateoService::from_state(state).is_none() {
                eprintln!("no SanMateoService before job workers start");
                std::process::exit(3);
            }
        })
        .run()
        .await;
}

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn app_dir(name: &str, port: u16) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sm-boot-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("autumn.toml"),
        format!(
            "[san_mateo]\nserver_url = \"http://127.0.0.1:{port}\"\nproviders_file = \"providers.yaml\"\n\n[san_mateo.environments.prod]\nsecret_key_env = \"BOOT_PROD_SECRET\"\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.join("providers.yaml"),
        "boot:\n    display_name: Boot API\n    auth_mode: API_KEY\n    proxy:\n        base_url: https://api.boot.example.com\n",
    )
    .unwrap();
    dir
}

/// Starts the child with a clean env, so a developer's `AUTUMN_*` vars,
/// `.env` or `autumn.toml` cannot change the result.
fn spawn(dir: &Path, port: u16, env: &[(&str, &str)]) -> KillOnDrop {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([
        "--exact",
        "boot_child_server",
        "--include-ignored",
        "--nocapture",
    ])
    .env_clear()
    .env("PATH", std::env::var_os("PATH").unwrap_or_default())
    .env("AUTUMN_MANIFEST_DIR", dir)
    .current_dir(dir)
    .env(CHILD_MARK, "1")
    .env("AUTUMN_SERVER__PORT", port.to_string())
    .env("AUTUMN_SERVER__HOST", "127.0.0.1")
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    KillOnDrop(cmd.spawn().unwrap())
}

fn output_of(child: &mut Child) -> String {
    let mut out = String::new();
    if let Some(mut e) = child.stderr.take() {
        std::io::Read::read_to_string(&mut e, &mut out).unwrap();
    }
    if let Some(mut o) = child.stdout.take() {
        std::io::Read::read_to_string(&mut o, &mut out).unwrap();
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_boots_with_config_files_and_env_vars() {
    let port = free_port();
    let dir = app_dir("ok", port);
    let mut child = spawn(
        &dir,
        port,
        &[
            ("BOOT_PROD_SECRET", SECRET),
            ("SAN_MATEO_BOOT_EXPECT_SERVICE", "1"),
            // The env overlay names the env var with the key.
            ("AUTUMN_SAN_MATEO__ENCRYPTION_KEY_ENV", "BOOT_KEY"),
            ("BOOT_KEY", KEY_B64),
            ("AUTUMN_SAN_MATEO__BACKGROUND__TICK_SECS", "1"),
        ],
    );
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let start = Instant::now();
    loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            panic!("the app stopped at boot ({status}):\n{}", output_of(&mut child.0));
        }
        assert!(start.elapsed() < Duration::from_secs(60), "the app did not answer");
        if let Ok(res) = client.get(format!("{base}/san-mateo/health")).send().await
            && res.status().as_u16() == 200
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let auth = format!("Bearer {SECRET}");
    let res = client
        .get(format!("{base}/san-mateo/providers/boot"))
        .header("authorization", &auth)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let v: serde_json::Value = serde_json::from_str(&res.text().await.unwrap()).unwrap();
    assert_eq!(v["data"]["display_name"], "Boot API");
    let res = client
        .post(format!("{base}/san-mateo/integrations"))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(r#"{"unique_key":"boot","provider":"boot"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let res = client
        .post(format!("{base}/san-mateo/connect/sessions"))
        .header("authorization", &auth)
        .header("content-type", "application/json")
        .body(r#"{"end_user":{"id":"u1"}}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status().as_u16(), 201);
    let v: serde_json::Value = serde_json::from_str(&res.text().await.unwrap()).unwrap();
    let link = v["data"]["connect_link"].as_str().unwrap();
    assert!(link.starts_with(&format!("{base}/san-mateo/connect?")), "{link}");
    let res = client.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(res.text().await.unwrap(), "app");
    let res = client
        .get(format!("{base}/actuator/health"))
        .send()
        .await
        .unwrap();
    let body = res.text().await.unwrap();
    assert!(body.contains("san_mateo"), "{body}");
    assert!(!body.contains(SECRET) && !body.contains(KEY_B64), "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_secret_stops_the_boot_with_the_env_var_name() {
    let port = free_port();
    let dir = app_dir("missing", port);
    let mut child = spawn(&dir, port, &[("SAN_MATEO_ENCRYPTION_KEY", KEY_B64)]);
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "the app did not stop"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let out = output_of(&mut child.0);
    assert!(!status.success(), "{out}");
    assert!(out.contains("BOOT_PROD_SECRET"), "{out}");
}
