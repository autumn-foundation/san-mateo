//! Integrations, connect sessions and auth flows (AC3 to AC7).

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::*;
use san_mateo::credentials::Credentials;
use san_mateo::crypto::aad;
use san_mateo::model::{ConnectionKey, IntegrationDefaults};
use san_mateo::oauth2::pkce_challenge;
use san_mateo::store::Store;
use san_mateo::transport::HttpResponse;
use san_mateo::{
    Catalog, ConnectOptions, ConnectSessionInput, EnvironmentConfig, Error, GetOptions,
    ImportConnection, IntegrationPatch, KeyRing, NewIntegration, SanMateo, Secret,
};
use serde_json::{Map, json};

#[test]
fn builder_checks_its_input() {
    let base = || SanMateo::builder().catalog(Catalog::from_yaml_str(CATALOG).unwrap());
    let env = || EnvironmentConfig::new("dev", "sk_one_0123456789");
    assert!(matches!(base().environment(env()).build(), Err(Error::Invalid(_))), "no key ring");
    assert!(matches!(base().key_ring(keys()).build(), Err(Error::Invalid(_))), "no environment");
    assert!(matches!(base().key_ring(keys()).environment(env()).environment(env()).build(), Err(Error::Invalid(_))), "same id");
    assert!(
        matches!(
            base().key_ring(keys()).environment(env()).environment(EnvironmentConfig::new("prod", "sk_one_0123456789")).build(),
            Err(Error::Invalid(_))
        ),
        "same key"
    );
    assert!(matches!(base().key_ring(keys()).environment(EnvironmentConfig::new("dev", "")).build(), Err(Error::Invalid(_))), "empty key");
    assert!(matches!(base().key_ring(keys()).environment(env()).server_url("not a url").build(), Err(Error::Invalid(_))), "bad url");
    assert!(base().key_ring(keys()).environment(env()).build().is_ok());
}

#[test]
fn secret_keys_select_the_environment() {
    let h = harness();
    assert_eq!(h.san.authenticate(SECRET).unwrap().id, "dev");
    assert_eq!(h.san.authenticate(PROD_SECRET).unwrap().id, "prod");
    assert!(matches!(h.san.authenticate("sk_wrong"), Err(Error::Unauthorized(_))));
    assert!(matches!(h.san.authenticate(""), Err(Error::Unauthorized(_))));
    assert_eq!(h.san.environment("prod").unwrap().id, "prod");
    assert!(h.san.environment("nope").is_err());
    assert!(h.san.catalog().get("crm").is_some());
    assert_eq!(h.san.config().server_url, SERVER);
}

#[tokio::test]
async fn integrations_crud_hides_and_seals_the_secret() {
    let h = harness();
    let input = NewIntegration {
        unique_key: "my-crm".into(),
        provider: "crm".into(),
        client_id: Some("cid".into()),
        client_secret: Some("csecret-plain".into()),
        scopes: vec!["read".into()],
        ..NewIntegration::default()
    };
    let i = h.san.create_integration("dev", input.clone()).await.unwrap();
    assert_eq!(i.unique_key, "my-crm");
    assert_eq!(i.display_name, "Test CRM");
    assert!(i.has_client_secret);
    let row = h.store.get_integration("dev", "my-crm").await.unwrap().unwrap();
    let sealed = row.client_secret.unwrap();
    assert!(sealed.starts_with("sm1.") && !sealed.contains("csecret-plain"));

    assert!(matches!(h.san.create_integration("dev", input.clone()).await, Err(Error::Conflict(_))));
    let unknown = NewIntegration { unique_key: "x".into(), provider: "nope".into(), ..NewIntegration::default() };
    assert!(matches!(h.san.create_integration("dev", unknown).await, Err(Error::UnknownProvider(_))));
    let bad_key = NewIntegration { unique_key: "bad key/../".into(), ..input.clone() };
    assert!(matches!(h.san.create_integration("dev", bad_key).await, Err(Error::Invalid(_))));
    assert!(h.san.create_integration("nope", input).await.is_err());

    let up = h
        .san
        .update_integration("dev", "my-crm", IntegrationPatch { scopes: Some(vec!["a".into(), "b".into()]), display_name: Some("Mine".into()), ..IntegrationPatch::default() })
        .await
        .unwrap();
    assert_eq!(up.scopes, vec!["a", "b"]);
    assert_eq!(up.display_name, "Mine");
    assert!(up.has_client_secret);
    assert_eq!(h.san.list_integrations("dev").await.unwrap().len(), 1);
    assert!(h.san.list_integrations("prod").await.unwrap().is_empty());
    assert!(matches!(h.san.get_integration("prod", "my-crm").await, Err(Error::UnknownIntegration(_))));
    h.san.delete_integration("dev", "my-crm").await.unwrap();
    assert!(matches!(h.san.get_integration("dev", "my-crm").await, Err(Error::UnknownIntegration(_))));
    assert!(matches!(h.san.delete_integration("dev", "my-crm").await, Err(Error::UnknownIntegration(_))));
    assert!(matches!(h.san.update_integration("dev", "my-crm", IntegrationPatch::default()).await, Err(Error::UnknownIntegration(_))));
}

#[tokio::test]
async fn connect_sessions_store_only_a_hash_and_expire() {
    let h = ready().await;
    let created = h.san.create_connect_session("dev", ConnectSessionInput { end_user: Some(end_user()), allowed_integrations: vec!["crm".into()], ..ConnectSessionInput::default() }).await.unwrap();
    let token = created.token.expose();
    assert!(token.starts_with("smcs_") && token.len() == 69 && token[5..].chars().all(|c| c.is_ascii_hexdigit()), "{token}");
    assert_eq!(created.connect_link, format!("{SERVER}/connect?session_token={token}"));
    assert_eq!(created.expires_at, start() + chrono::Duration::minutes(30));
    assert!(h.store.get_connect_session(token).await.unwrap().is_none(), "the store must not keep the token");
    assert!(h.store.get_connect_session(&h.keys.lookup_hash(token)).await.unwrap().is_some());

    let s = h.san.connect_session(token).await.unwrap();
    assert_eq!(s.environment, "dev");
    assert_eq!(s.allowed_integrations, vec!["crm"]);
    assert_eq!(s.end_user.unwrap().id, "user-1");

    assert!(matches!(h.san.connect_session("smcs_nope").await, Err(Error::InvalidConnectSession)));
    assert!(matches!(h.san.connect_session(SECRET).await, Err(Error::InvalidConnectSession)));
    h.clock.advance(chrono::Duration::minutes(30));
    assert!(matches!(h.san.connect_session(token).await, Err(Error::InvalidConnectSession)));

    let other = session(&h).await;
    h.san.delete_connect_session(&other).await.unwrap();
    assert!(matches!(h.san.connect_session(&other).await, Err(Error::InvalidConnectSession)));

    let unknown = ConnectSessionInput { allowed_integrations: vec!["nope".into()], ..ConnectSessionInput::default() };
    assert!(matches!(h.san.create_connect_session("dev", unknown).await, Err(Error::UnknownIntegration(_))));
    let plain_hook = ConnectSessionInput { webhook_url_override: Some("http://hooks.example.com".into()), ..ConnectSessionInput::default() };
    assert!(matches!(h.san.create_connect_session("dev", plain_hook).await, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn oauth2_flow_end_to_end() {
    let h = ready().await;
    h.http.reply("POST", "https://auth.crm.example.com/token", token_reply("at-1", Some("rt-1"), Some(3_600)));
    let token = session(&h).await;
    let url = h.san.oauth_start(&token, "crm", ConnectOptions::default()).await.unwrap();
    assert!(url.starts_with("https://auth.crm.example.com/authorize?"), "{url}");
    let q = query_of(&url);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], "crm-client");
    assert_eq!(q["redirect_uri"], format!("{SERVER}/oauth/callback"));
    assert_eq!(q["scope"], "read,write");
    assert_eq!(q["prompt"], "consent");
    assert_eq!(q["code_challenge_method"], "S256");
    let state = q["state"].clone();
    assert!(state.len() >= 43, "{state}");

    let cb = BTreeMap::from([("code".to_owned(), "code-1".to_owned()), ("state".to_owned(), state.clone())]);
    let conn = h.san.oauth_callback(&cb).await.unwrap();

    let req = &h.http.requests_to("https://auth.crm.example.com/token")[0];
    let form: BTreeMap<String, String> = url::form_urlencoded::parse(&req.body).into_owned().collect();
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "code-1");
    assert_eq!(form["redirect_uri"], format!("{SERVER}/oauth/callback"));
    assert_eq!(form["client_id"], "crm-client");
    assert_eq!(form["client_secret"], "crm-secret");
    assert_eq!(pkce_challenge(&form["code_verifier"]), q["code_challenge"]);

    assert_eq!(conn.provider_config_key, "crm");
    assert_eq!(conn.provider, "crm");
    assert_eq!(conn.environment, "dev");
    assert!(!conn.connection_id.is_empty());
    assert_eq!(conn.connection_config["instance_id"], "inst-7");
    assert_eq!(conn.end_user.as_ref().unwrap().id, "user-1");
    assert_eq!(conn.tags["plan"], "pro");
    match &conn.credentials {
        Credentials::OAuth2(c) => {
            assert_eq!(c.access_token.expose(), "at-1");
            assert_eq!(c.refresh_token.as_ref().unwrap().expose(), "rt-1");
            assert_eq!(c.expires_at, Some(start() + chrono::Duration::seconds(3_600)));
        }
        other => panic!("{other:?}"),
    }

    // The state is single use.
    assert!(matches!(h.san.oauth_callback(&cb).await, Err(Error::InvalidOAuthState)));
    let no_state = BTreeMap::from([("code".to_owned(), "x".to_owned())]);
    assert!(matches!(h.san.oauth_callback(&no_state).await, Err(Error::InvalidOAuthState)));

    h.san.flush().await;
    let hooks = webhooks_sent(&h);
    assert_eq!(hooks.len(), 1);
    assert_eq!(hooks[0]["type"], "auth");
    assert_eq!(hooks[0]["operation"], "creation");
    assert_eq!(hooks[0]["success"], true);
    assert_eq!(hooks[0]["connectionId"], conn.connection_id);
    let sent = &h.http.requests_to(HOOK)[0];
    assert_eq!(sent.header_value("x-san-mateo-hmac-sha256").unwrap(), san_mateo::webhook::sign(WHSEC, &sent.body));
}

#[tokio::test]
async fn credentials_are_sealed_and_bound_to_the_row() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-secret-1", Some("rt-secret-1"), Some(3_600)).await;
    let row = h.store.get_connection("dev", "crm", &conn.connection_id).await.unwrap().unwrap();
    assert!(row.credentials.starts_with("sm1.k1."));
    assert!(!row.credentials.contains("at-secret-1"));
    // The ciphertext of one row does not open as another row.
    let wrong = aad(&["connection", "dev", "crm", "other-id"]);
    assert!(h.keys.open(&row.credentials, &wrong).is_err());
    // The service gives a decryption error for a moved ciphertext.
    let mut moved = h.store.upsert_connection(san_mateo::store::ConnectionRow { id: uuid::Uuid::new_v4(), connection_id: "other-id".into(), ..row.clone() }).await.unwrap().row;
    moved.credentials = row.credentials.clone();
    h.store.update_connection(&moved).await.unwrap();
    let err = h.san.get_connection("dev", &ConnectionKey::new("crm", "other-id"), GetOptions::default()).await.unwrap_err();
    assert_eq!(err, Error::Crypto);
    assert!(!format!("{conn:?}").contains("at-secret-1"));
}

#[tokio::test]
async fn oauth_errors_are_reported() {
    let h = ready().await;
    let token = session(&h).await;
    // The end user denies access.
    let url = h.san.oauth_start(&token, "crm", ConnectOptions::default()).await.unwrap();
    let state = query_of(&url)["state"].clone();
    let denied = BTreeMap::from([("error".to_owned(), "access_denied".to_owned()), ("state".to_owned(), state)]);
    assert!(matches!(h.san.oauth_callback(&denied).await, Err(Error::Provider { code: "authorization_denied", .. })));
    // The token exchange fails.
    h.http.reply("POST", "https://auth.crm.example.com/token", HttpResponse::json(400, &json!({"error": "invalid_grant"})));
    let url = h.san.oauth_start(&token, "crm", ConnectOptions::default()).await.unwrap();
    let cb = BTreeMap::from([("code".to_owned(), "bad".to_owned()), ("state".to_owned(), query_of(&url)["state"].clone())]);
    let err = h.san.oauth_callback(&cb).await.unwrap_err();
    assert!(matches!(err, Error::Provider { code: "token_exchange_failed", status: Some(400), .. }), "{err:?}");
    // An expired state fails.
    let url = h.san.oauth_start(&token, "crm", ConnectOptions::default()).await.unwrap();
    h.clock.advance(chrono::Duration::minutes(16));
    let cb = BTreeMap::from([("code".to_owned(), "c".to_owned()), ("state".to_owned(), query_of(&url)["state"].clone())]);
    assert!(matches!(h.san.oauth_callback(&cb).await, Err(Error::InvalidOAuthState)));
    h.san.flush().await;
    let hooks = webhooks_sent(&h);
    assert_eq!(hooks.len(), 2);
    assert!(hooks.iter().all(|w| w["success"] == false && w["operation"] == "creation"));
    assert_eq!(hooks[1]["error"]["type"], "token_exchange_failed");
}

#[tokio::test]
async fn session_rules_apply_to_auth() {
    let h = ready().await;
    let only_keyed = session_with(&h, ConnectSessionInput { allowed_integrations: vec!["keyed".into()], ..ConnectSessionInput::default() }).await;
    assert!(matches!(h.san.oauth_start(&only_keyed, "crm", ConnectOptions::default()).await, Err(Error::IntegrationNotAllowed(_))));
    let token = session(&h).await;
    assert!(matches!(h.san.oauth_start(&token, "nope", ConnectOptions::default()).await, Err(Error::UnknownIntegration(_))));
    assert!(matches!(h.san.oauth_start(&token, "keyed", ConnectOptions::default()).await, Err(Error::UnsupportedAuthMode(_))));
    assert!(matches!(h.san.oauth_start(&token, "twostep", ConnectOptions::default()).await, Err(Error::UnsupportedAuthMode(_))));
    assert!(matches!(h.san.oauth_start("smcs_bad", "crm", ConnectOptions::default()).await, Err(Error::InvalidConnectSession)));
}

#[tokio::test]
async fn session_defaults_replace_end_user_values() {
    let h = ready().await;
    let defaults = IntegrationDefaults {
        user_scopes: Some("admin,read".into()),
        authorization_params: BTreeMap::from([("prompt".to_owned(), "none".to_owned())]),
        connection_config: Map::from_iter([("subdomain".to_owned(), json!("backend-set"))]),
    };
    let token = session_with(&h, ConnectSessionInput { integrations_config_defaults: BTreeMap::from([("crm".to_owned(), defaults.clone()), ("zen".to_owned(), defaults)]), ..ConnectSessionInput::default() }).await;
    let opts = ConnectOptions {
        connection_config: Map::from_iter([("subdomain".to_owned(), json!("user-set"))]),
        authorization_params: BTreeMap::from([("prompt".to_owned(), "login".to_owned())]),
        user_scopes: Some("ignored".into()),
    };
    let url = h.san.oauth_start(&token, "crm", opts.clone()).await.unwrap();
    let q = query_of(&url);
    assert_eq!(q["scope"], "admin,read");
    assert_eq!(q["prompt"], "none");
    let url = h.san.oauth_start(&token, "zen", opts).await.unwrap();
    assert!(url.starts_with("https://backend-set.zen.example.com/oauth/authorize?"), "{url}");
}

#[tokio::test]
async fn interpolated_provider_with_basic_client_auth() {
    let h = ready().await;
    let token = session(&h).await;
    assert!(matches!(h.san.oauth_start(&token, "zen", ConnectOptions::default()).await, Err(Error::MissingField(f)) if f == "subdomain"));
    let bad = ConnectOptions { connection_config: Map::from_iter([("subdomain".to_owned(), json!("evil.com/x"))]), ..ConnectOptions::default() };
    assert!(matches!(h.san.oauth_start(&token, "zen", bad).await, Err(Error::Invalid(_))));

    let opts = ConnectOptions { connection_config: Map::from_iter([("subdomain".to_owned(), json!("acme"))]), ..ConnectOptions::default() };
    let url = h.san.oauth_start(&token, "zen", opts).await.unwrap();
    let q = query_of(&url);
    assert!(!q.contains_key("code_challenge"), "PKCE is off for zen");
    h.http.reply("POST", "https://acme.zen.example.com/oauth/token", token_reply("zat", None, None));
    let cb = BTreeMap::from([("code".to_owned(), "zc".to_owned()), ("state".to_owned(), q["state"].clone())]);
    let conn = h.san.oauth_callback(&cb).await.unwrap();
    assert_eq!(conn.connection_config["subdomain"], "acme");
    let req = &h.http.requests_to("https://acme.zen.example.com/oauth/token")[0];
    assert_eq!(req.header_value("authorization"), Some("Basic emVuLWNsaWVudDp6ZW4tc2VjcmV0"));
    let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
    assert_eq!(body, json!({"grant_type": "authorization_code", "code": "zc", "redirect_uri": format!("{SERVER}/oauth/callback"), "expires_in": "1800"}));
}

#[tokio::test]
async fn reconnect_replaces_credentials() {
    let h = ready().await;
    let first = connect_crm(&h, "at-1", Some("rt-1"), Some(3_600)).await;
    let key = first.key();
    let created = h.san.create_reconnect_session("dev", &key, ConnectSessionInput::default()).await.unwrap();
    h.http.reply("POST", "https://auth.crm.example.com/token", token_reply("at-2", Some("rt-2"), Some(3_600)));
    let url = h.san.oauth_start(created.token.expose(), "crm", ConnectOptions::default()).await.unwrap();
    let cb = BTreeMap::from([("code".to_owned(), "c2".to_owned()), ("state".to_owned(), query_of(&url)["state"].clone())]);
    let second = h.san.oauth_callback(&cb).await.unwrap();
    assert_eq!(second.connection_id, first.connection_id);
    assert_eq!(second.id, first.id);
    assert_eq!(second.credentials.bearer_token(), Some("at-2"));
    // A reconnect session cannot connect a different integration.
    assert!(matches!(h.san.oauth_start(created.token.expose(), "zen", ConnectOptions::default()).await, Err(Error::IntegrationNotAllowed(_))));
    assert!(matches!(h.san.create_reconnect_session("dev", &ConnectionKey::new("crm", "nope"), ConnectSessionInput::default()).await, Err(Error::UnknownConnection(_))));
    h.san.flush().await;
    let ops: Vec<String> = webhooks_sent(&h).iter().map(|w| w["operation"].as_str().unwrap().to_owned()).collect();
    assert_eq!(ops, vec!["creation", "override"]);
}

#[tokio::test]
async fn api_key_connections_are_verified() {
    let h = ready().await;
    let token = session(&h).await;
    h.http.reply("GET", "https://api.keyed.example.com/v1/me", HttpResponse::new(401, "no"));
    let err = h.san.connect_api_key(&token, "keyed", Secret::new("bad-key"), ConnectOptions::default()).await.unwrap_err();
    assert!(matches!(err, Error::Provider { code: "verification_failed", status: Some(401), .. }), "{err:?}");
    assert!(h.san.list_connections("dev", &Default::default()).await.unwrap().is_empty());

    h.http.reply("GET", "https://api.keyed.example.com/v1/me", HttpResponse::new(200, "{}"));
    let conn = h.san.connect_api_key(&token, "keyed", Secret::new("good-key"), ConnectOptions::default()).await.unwrap();
    assert_eq!(conn.credentials, Credentials::ApiKey { api_key: "good-key".into() });
    let checks = h.http.requests_to("https://api.keyed.example.com/v1/me");
    assert_eq!(checks.last().unwrap().header_value("x-api-key"), Some("good-key"));
    assert!(matches!(h.san.connect_api_key(&token, "keyed", Secret::new(""), ConnectOptions::default()).await, Err(Error::MissingField(_))));
    assert!(matches!(h.san.connect_api_key(&token, "crm", Secret::new("k"), ConnectOptions::default()).await, Err(Error::UnsupportedAuthMode(_))));
}

#[tokio::test]
async fn basic_cc_and_unauthenticated_connections() {
    let h = ready().await;
    let token = session(&h).await;
    let b = h.san.connect_basic(&token, "basic", "ann".into(), Secret::new("pw"), ConnectOptions::default()).await.unwrap();
    assert_eq!(b.credentials, Credentials::Basic { username: "ann".into(), password: "pw".into() });
    assert!(matches!(h.san.connect_basic(&token, "basic", String::new(), Secret::new("pw"), ConnectOptions::default()).await, Err(Error::MissingField(_))));

    h.http.reply("POST", "https://auth.machine.example.com/token", HttpResponse::json(200, &json!({"access_token": "cc-1", "expires_in": 600})));
    let cc = h.san.connect_client_credentials(&token, "machine", "m-id".into(), Secret::new("m-secret"), ConnectOptions::default()).await.unwrap();
    match &cc.credentials {
        Credentials::OAuth2ClientCredentials(c) => {
            assert_eq!(c.token.expose(), "cc-1");
            assert_eq!(c.client_id, "m-id");
            assert_eq!(c.expires_at, Some(start() + chrono::Duration::seconds(600)));
        }
        other => panic!("{other:?}"),
    }
    let req = &h.http.requests_to("https://auth.machine.example.com/token")[0];
    let form: BTreeMap<String, String> = url::form_urlencoded::parse(&req.body).into_owned().collect();
    assert_eq!(form["grant_type"], "client_credentials");
    assert_eq!(form["scope"], "jobs");
    assert_eq!(form["client_secret"], "m-secret");

    let o = h.san.connect_unauthenticated(&token, "open", ConnectOptions::default()).await.unwrap();
    assert_eq!(o.credentials, Credentials::Unauthenticated);
    assert!(matches!(h.san.connect_unauthenticated(&token, "basic", ConnectOptions::default()).await, Err(Error::UnsupportedAuthMode(_))));
    assert_eq!(h.san.list_connections("dev", &Default::default()).await.unwrap().len(), 3);
}

#[tokio::test]
async fn import_checks_the_auth_mode() {
    let h = ready().await;
    let input = ImportConnection {
        connection_id: Some("imp-1".into()),
        provider_config_key: "keyed".into(),
        credentials: Credentials::ApiKey { api_key: "k".into() },
        connection_config: Map::new(),
        metadata: Map::from_iter([("a".to_owned(), json!(1))]),
        tags: BTreeMap::new(),
        end_user: None,
        webhook_url_override: None,
    };
    let c = h.san.import_connection("dev", input.clone()).await.unwrap();
    assert_eq!(c.connection_id, "imp-1");
    assert_eq!(c.metadata["a"], 1);
    let wrong = ImportConnection { credentials: Credentials::Basic { username: "u".into(), password: "p".into() }, ..input.clone() };
    assert!(matches!(h.san.import_connection("dev", wrong).await, Err(Error::Invalid(_))));
    let unknown = ImportConnection { provider_config_key: "nope".into(), ..input.clone() };
    assert!(matches!(h.san.import_connection("dev", unknown).await, Err(Error::UnknownIntegration(_))));
    let generated = ImportConnection { connection_id: None, ..input };
    assert!(!h.san.import_connection("dev", generated).await.unwrap().connection_id.is_empty());
}

#[tokio::test]
async fn key_rotation_keeps_connections_readable() {
    let h = ready().await;
    let conn = connect_crm(&h, "at-1", None, None).await;
    let rotated = KeyRing::new("k1", [42; 32]).unwrap().with_key("k2", [7; 32]).unwrap().activate("k2").unwrap();
    let san = SanMateo::builder()
        .server_url(SERVER)
        .environment(EnvironmentConfig::new("dev", SECRET))
        .catalog(Catalog::from_yaml_str(CATALOG).unwrap())
        .store(h.store.clone())
        .key_ring(rotated)
        .transport(Arc::new(h.http.clone()))
        .build()
        .unwrap();
    let got = san.get_connection("dev", &conn.key(), GetOptions { refresh_if_needed: false, force_refresh: false }).await.unwrap();
    assert_eq!(got.credentials.bearer_token(), Some("at-1"));
}
