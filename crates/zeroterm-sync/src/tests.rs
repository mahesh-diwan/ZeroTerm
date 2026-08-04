use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::client::SyncClient;
use crate::crypto::CryptoKey;
use crate::daemon::SyncDaemon;
use crate::store::{merge, DocEntry, DocStore};

fn entry(rev: u64, data: &str, timestamp_ms: u64) -> DocEntry {
    DocEntry {
        rev,
        data: data.as_bytes().to_vec(),
        timestamp_ms,
    }
}

#[test]
fn crypto_roundtrip() {
    let key = CryptoKey::generate();
    let plaintext = b"secret settings";
    let encrypted = key.encrypt(plaintext).unwrap();
    assert_ne!(encrypted, plaintext);
    assert_eq!(key.decrypt(&encrypted).unwrap(), plaintext);
}

#[test]
fn crypto_roundtrip_rejects_tampered() {
    let key = CryptoKey::generate();
    let mut encrypted = key.encrypt(b"secret").unwrap();
    encrypted[12] ^= 0xff;
    assert!(key.decrypt(&encrypted).is_err());
}

#[test]
fn store_put_get_highest_rev_wins() {
    let mut store = DocStore::new();
    assert_eq!(store.get("config"), None);
    store.put("config", b"v1");
    store.put("config", b"v2");
    assert_eq!(store.rev("config"), 2);
    assert_eq!(store.get("config"), Some(&b"v2"[..]));
    assert_eq!(store.entries("config").len(), 2);
}

#[test]
fn merge_server_newer_wins() {
    let server = vec![entry(1, "server", 20)];
    let local = vec![entry(1, "local", 10)];
    assert_eq!(merge(&server, &local), vec![entry(1, "server", 20)]);
}

#[test]
fn merge_local_newer_wins() {
    let server = vec![entry(1, "server", 10)];
    let local = vec![entry(1, "local", 20)];
    assert_eq!(merge(&server, &local), vec![entry(1, "local", 20)]);
}

#[test]
fn merge_disjoint_revisions_both_kept() {
    let server = vec![entry(1, "server", 10)];
    let local = vec![entry(2, "local", 20)];
    assert_eq!(
        merge(&server, &local),
        vec![entry(1, "server", 10), entry(2, "local", 20)]
    );
}

#[test]
fn merge_same_timestamp_higher_rev_wins() {
    let server = vec![entry(1, "server", 30)];
    let local = vec![entry(2, "local", 30)];
    assert_eq!(merge(&server, &local), vec![entry(2, "local", 30)]);
}

#[test]
fn merge_result_sorted_by_rev() {
    let server = vec![entry(3, "s3", 30), entry(2, "s2", 20)];
    let local = vec![entry(1, "l1", 10)];
    let merged = merge(&server, &local);
    let revs: Vec<u64> = merged.iter().map(|e| e.rev).collect();
    assert_eq!(revs, vec![1, 2, 3]);
}

#[test]
fn server_post_merges_lww() {
    let server = crate::server::SyncServer::new();
    assert_eq!(server.get("hosts"), Vec::<DocEntry>::new());
    server.post("hosts", vec![entry(1, "a", 10)]);
    let merged = server.post("hosts", vec![entry(2, "b", 20)]);
    assert_eq!(merged.len(), 2);
    assert_eq!(server.get("hosts").len(), 2);
}

#[tokio::test]
async fn server_router_keeps_payload_encrypted() {
    let server = Arc::new(crate::server::SyncServer::new());
    let app = crate::server::router(server);

    let key = CryptoKey::generate();
    let secret = b"plaintext that never leaves the client";
    let wire = vec![DocEntry {
        rev: 1,
        data: key.encrypt(secret).unwrap(),
        timestamp_ms: 1,
    }];

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/docs/hosts")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_string(&wire).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/docs/hosts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let entries: Vec<DocEntry> = serde_json::from_slice(&body).unwrap();
    assert_eq!(entries.len(), 1);
    assert_ne!(entries[0].data, secret);
    assert_eq!(key.decrypt(&entries[0].data).unwrap(), secret);
}

async fn spawn_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Arc::new(crate::server::SyncServer::new());
    tokio::spawn(async move {
        axum::serve(listener, crate::server::router(server))
            .await
            .unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn client_sync_full_roundtrip() {
    let url = spawn_server().await;
    let key = CryptoKey::generate();
    let client = SyncClient::new(url, key);

    let mut store = DocStore::new();
    store.put("config", b"local settings");

    assert_eq!(
        client.sync(&mut store, "config").await.unwrap(),
        crate::client::SyncResult::Updated
    );
    assert_eq!(store.get("config"), Some(&b"local settings"[..]));

    let result = client.sync(&mut store, "config").await.unwrap();
    assert_eq!(result, crate::client::SyncResult::NoChange);
}

#[tokio::test]
async fn client_sync_conflict_local_wins_then_converges() {
    let url = spawn_server().await;
    let key = CryptoKey::generate();
    let client_a = SyncClient::new(url.clone(), key);
    let client_b = SyncClient::new(url.clone(), key);

    let mut store_a = DocStore::new();
    store_a.put("hosts", b"hosts from a");
    client_a.sync(&mut store_a, "hosts").await.unwrap();

    let mut store_b = DocStore::new();
    store_b.put("hosts", b"hosts from b");
    client_b.sync(&mut store_b, "hosts").await.unwrap();

    let mut store_c = DocStore::new();
    client_a.sync(&mut store_c, "hosts").await.unwrap();
    let merged = store_c.get("hosts").unwrap().to_vec();
    assert!(merged == b"hosts from a" || merged == b"hosts from b");

    client_b.sync(&mut store_c, "hosts").await.unwrap();
    assert_eq!(
        client_a.sync(&mut store_c, "hosts").await.unwrap(),
        crate::client::SyncResult::NoChange
    );
    assert_eq!(
        client_b.sync(&mut store_c, "hosts").await.unwrap(),
        crate::client::SyncResult::NoChange
    );
}

#[tokio::test]
async fn daemon_sync_all_with_persisted_key() {
    let url = spawn_server().await;
    let key = CryptoKey::generate();
    let daemon = SyncDaemon::new(url).with_key(key);

    let mut store = DocStore::new();
    store.put("config", b"cfg");
    store.put("hosts", b"h1");

    let results = daemon.sync_all(&mut store, &["config", "hosts"]).await;
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(Result::is_ok));
    assert!(daemon.sync_all(&mut store, &["config"]).await[0].is_ok());
}

#[tokio::test]
async fn daemon_sync_all_skips_empty_url() {
    let key = CryptoKey::generate();
    let daemon = SyncDaemon::new(String::new()).with_key(key);
    let mut store = DocStore::new();
    let results = daemon.sync_all(&mut store, &["config"]).await;
    assert_eq!(results.len(), 1);
    assert!(results[0].is_ok());
}
