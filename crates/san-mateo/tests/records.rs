//! Records (AC14).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::too_many_lines)]

mod common;

use common::*;
use san_mateo::model::ConnectionKey;
use san_mateo::records::{Cursor, RecordQuery};
use san_mateo::store::Store;
use san_mateo::{Error, RecordsRequest};
use serde_json::json;

fn req(model: &str) -> RecordsRequest {
    RecordsRequest {
        model: model.to_owned(),
        ..RecordsRequest::default()
    }
}

#[tokio::test]
async fn save_classify_and_read_back() {
    let h = ready().await;
    let key = connect_crm(&h, "at", None, None).await.key();
    let out = h
        .san
        .save_records(
            "dev",
            &key,
            "Contact",
            vec![
                json!({"id": "1", "name": "Ann"}),
                json!({"id": 2, "name": "Bob"}),
            ],
        )
        .await
        .unwrap();
    assert_eq!(out.added, vec!["1", "2"]);
    h.clock.advance(chrono::Duration::seconds(10));
    let out = h
        .san
        .save_records(
            "dev",
            &key,
            "Contact",
            vec![
                json!({"name": "Ann", "id": "1"}),
                json!({"id": 2, "name": "Bobby"}),
            ],
        )
        .await
        .unwrap();
    assert_eq!(out.unchanged, vec!["1"], "key order does not matter");
    assert_eq!(out.updated, vec!["2"]);
    h.clock.advance(chrono::Duration::seconds(10));
    let out = h
        .san
        .delete_records(
            "dev",
            &key,
            "Contact",
            vec![json!({"id": "1"}), json!({"id": "404"})],
        )
        .await
        .unwrap();
    assert_eq!(out.deleted, vec!["1"]);

    let page = h
        .san
        .list_records("dev", &key, req("Contact"))
        .await
        .unwrap();
    assert_eq!(page.next_cursor, None);
    assert_eq!(page.records.len(), 2);
    let bob = &page.records[0];
    assert_eq!(bob["name"], "Bobby");
    assert_eq!(bob["_nango_metadata"]["last_action"], "UPDATED");
    assert_eq!(
        bob["_nango_metadata"]["first_seen_at"],
        "2023-11-14T22:13:20Z"
    );
    assert_eq!(
        bob["_nango_metadata"]["last_modified_at"],
        "2023-11-14T22:13:30Z"
    );
    assert_eq!(
        bob["_nango_metadata"]["deleted_at"],
        serde_json::Value::Null
    );
    assert!(bob["_nango_metadata"]["cursor"].is_string());
    let ann = &page.records[1];
    assert_eq!(ann["_nango_metadata"]["last_action"], "DELETED");
    assert_eq!(ann["_nango_metadata"]["deleted_at"], "2023-11-14T22:13:40Z");
}

#[tokio::test]
async fn records_are_sealed_at_rest() {
    let h = ready().await;
    let conn = connect_crm(&h, "at", None, None).await;
    h.san
        .save_records(
            "dev",
            &conn.key(),
            "Contact",
            vec![json!({"id": "1", "email": "secret@example.com"})],
        )
        .await
        .unwrap();
    let rows = h
        .store
        .list_records(
            conn.id,
            &RecordQuery {
                model: "Contact".into(),
                limit: 10,
                ..RecordQuery::default()
            },
        )
        .await
        .unwrap();
    assert!(rows[0].data.starts_with("sm1."));
    assert!(!rows[0].data.contains("secret@example.com"));
    assert!(!rows[0].data_hash.is_empty());
}

#[tokio::test]
async fn cursor_pages_and_filters() {
    let h = ready().await;
    let key = connect_crm(&h, "at", None, None).await.key();
    let batch: Vec<_> = (0..5)
        .map(|i| json!({"id": i.to_string(), "n": i}))
        .collect();
    h.san.save_records("dev", &key, "M", batch).await.unwrap();
    h.clock.advance(chrono::Duration::seconds(5));
    h.san
        .save_records("dev", &key, "M", vec![json!({"id": "0", "n": 100})])
        .await
        .unwrap();

    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = h
            .san
            .list_records(
                "dev",
                &key,
                RecordsRequest {
                    limit: Some(2),
                    cursor: cursor.clone(),
                    ..req("M")
                },
            )
            .await
            .unwrap();
        seen.extend(
            page.records
                .iter()
                .map(|r| r["id"].as_str().unwrap().to_owned()),
        );
        match page.next_cursor {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    assert_eq!(seen.len(), 5);
    assert_eq!(seen.last().unwrap(), "0", "the updated record comes last");

    let modified = h
        .san
        .list_records(
            "dev",
            &key,
            RecordsRequest {
                modified_after: Some("2023-11-14T22:13:21Z".into()),
                ..req("M")
            },
        )
        .await
        .unwrap();
    assert_eq!(modified.records.len(), 1);
    let updated = h
        .san
        .list_records(
            "dev",
            &key,
            RecordsRequest {
                filter: Some("updated".into()),
                ..req("M")
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.records.len(), 1);
    let added = h
        .san
        .list_records(
            "dev",
            &key,
            RecordsRequest {
                filter: Some("added,deleted".into()),
                ..req("M")
            },
        )
        .await
        .unwrap();
    assert_eq!(added.records.len(), 4);
    let ids = h
        .san
        .list_records(
            "dev",
            &key,
            RecordsRequest {
                ids: vec!["1".into(), "3".into()],
                ..req("M")
            },
        )
        .await
        .unwrap();
    assert_eq!(ids.records.len(), 2);
}

#[tokio::test]
async fn bad_queries_and_inputs() {
    let h = ready().await;
    let key = connect_crm(&h, "at", None, None).await.key();
    for bad in [
        RecordsRequest {
            limit: Some(0),
            ..req("M")
        },
        RecordsRequest {
            limit: Some(10_001),
            ..req("M")
        },
        RecordsRequest {
            cursor: Some("bad".into()),
            ..req("M")
        },
        RecordsRequest {
            modified_after: Some("yesterday".into()),
            ..req("M")
        },
        RecordsRequest {
            filter: Some("nope".into()),
            ..req("M")
        },
        RecordsRequest {
            ids: (0..101).map(|i| i.to_string()).collect(),
            ..req("M")
        },
        req(""),
    ] {
        assert!(
            matches!(
                h.san.list_records("dev", &key, bad.clone()).await,
                Err(Error::Invalid(_))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        h.san
            .save_records("dev", &key, "M", vec![json!({"no_id": 1})])
            .await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        h.san
            .save_records("dev", &key, "", vec![json!({"id": 1})])
            .await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        h.san
            .save_records(
                "dev",
                &ConnectionKey::new("crm", "nope"),
                "M",
                vec![json!({"id": 1})]
            )
            .await,
        Err(Error::UnknownConnection(_))
    ));
    assert!(matches!(
        h.san
            .list_records("dev", &ConnectionKey::new("crm", "nope"), req("M"))
            .await,
        Err(Error::UnknownConnection(_))
    ));
    let _ = Cursor::decode("x");
}

#[tokio::test]
async fn the_last_record_with_one_id_wins_in_a_batch() {
    let h = ready().await;
    let key = connect_crm(&h, "at", None, None).await.key();
    let out = h
        .san
        .save_records(
            "dev",
            &key,
            "M",
            vec![json!({"id": "1", "v": 1}), json!({"id": "1", "v": 2})],
        )
        .await
        .unwrap();
    assert_eq!(out.added, vec!["1"]);
    let page = h.san.list_records("dev", &key, req("M")).await.unwrap();
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0]["v"], 2);
}
