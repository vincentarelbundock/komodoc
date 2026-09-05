//! The storage rules are what keep a deployment's bill bounded no matter who
//! shows up. Each test below sets one storage limit low enough to trip on a
//! couple of small uploads.

use serde_json::json;

use super::*;
use crate::auth::Policy;
use crate::config::{Configuration, StorageLimit};

async fn with_storage(limit: StorageLimit) -> TestServer {
    let config = Configuration {
        storage: limit,
        ..Configuration::default()
    };
    test_server_with(
        config,
        Policy::parse(TEST_PUBLISHER),
        Policy::parse("anyone"),
        true,
    )
    .await
}

#[tokio::test]
async fn per_owner_byte_quota_is_refused() {
    let server = with_storage(StorageLimit {
        total: 1 << 30,
        per_owner: 20,
        documents_per_owner: 50,
        uploads_per_hour: 30,
    })
    .await;
    let (status, payload) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Too Big", "html": "x".repeat(21)}),
    )
    .await;
    assert_eq!(status, 507, "got {status} {payload}");
    assert_eq!(
        text(&payload, "error"),
        "your storage quota is used up; delete a document first"
    );
}

#[tokio::test]
async fn document_count_limit_is_refused() {
    let server = with_storage(StorageLimit {
        total: 1 << 30,
        per_owner: 1 << 20,
        documents_per_owner: 1,
        uploads_per_hour: 30,
    })
    .await;
    let (status, first) = post(
        &server.url,
        "/api/documents",
        json!({"title": "First", "html": "<p>first</p>"}),
    )
    .await;
    assert_eq!(status, 201, "first upload got {status}: {first}");
    let (status, payload) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Second", "html": "<p>second</p>"}),
    )
    .await;
    assert_eq!(status, 507, "got {status} {payload}");
    assert_eq!(
        text(&payload, "error"),
        "you have reached the document limit; delete one first"
    );
}

#[tokio::test]
async fn uploads_per_hour_is_refused() {
    let server = with_storage(StorageLimit {
        total: 1 << 30,
        per_owner: 1 << 20,
        documents_per_owner: 50,
        uploads_per_hour: 1,
    })
    .await;
    let (status, first) = post(
        &server.url,
        "/api/documents",
        json!({"title": "First", "html": "<p>first</p>"}),
    )
    .await;
    assert_eq!(status, 201, "first upload got {status}: {first}");
    // A second, distinct document, so this trips the hourly cap rather than
    // the document count.
    let (status, payload) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Second", "html": "<p>second</p>"}),
    )
    .await;
    assert_eq!(status, 429, "got {status} {payload}");
    assert_eq!(
        text(&payload, "error"),
        "too many uploads this hour; try later"
    );
}

#[tokio::test]
async fn global_total_quota_is_refused() {
    let server = with_storage(StorageLimit {
        total: 10,
        per_owner: 1 << 20,
        documents_per_owner: 50,
        uploads_per_hour: 30,
    })
    .await;
    let (status, payload) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Too Big", "html": "x".repeat(11)}),
    )
    .await;
    assert_eq!(status, 507, "got {status} {payload}");
    assert_eq!(text(&payload, "error"), "this deployment has no room left");
}

// Replacing a document is not a new document, and its old bytes are not still
// held once the new ones land: a quota exactly the size of one document
// should admit any number of republishes of it.
#[tokio::test]
async fn replacing_does_not_double_count_size() {
    let server = with_storage(StorageLimit {
        total: 1 << 30,
        per_owner: 30,
        documents_per_owner: 50,
        uploads_per_hour: 30,
    })
    .await;
    let html = "x".repeat(25);
    let (status, first) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Doc", "html": html}),
    )
    .await;
    assert_eq!(status, 201, "first upload got {status}: {first}");
    let slug = text(&first, "slug");
    let (status, second) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Doc", "slug": slug, "html": html}),
    )
    .await;
    assert_eq!(
        status, 201,
        "replacing the same document should not double-count its size, got {status}: {second}"
    );
}

// A replacement leaves the old version unreachable, so its bytes are removed
// rather than kept forever alongside the one the index actually names.
#[tokio::test]
async fn replacing_removes_the_superseded_version() {
    let server = new_test_server().await;
    let (status, first) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Doc", "html": "<p>version one</p>"}),
    )
    .await;
    assert_eq!(status, 201);
    let (slug, first_sha) = (text(&first, "slug"), text(&first, "sha"));
    let (status, second) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Doc", "slug": slug, "html": "<p>version two</p>"}),
    )
    .await;
    assert_eq!(status, 201);
    let second_sha = text(&second, "sha");
    assert_ne!(second_sha, first_sha);
    assert!(
        server.instance.store.read(&slug, &first_sha).await.is_err(),
        "the superseded version should have been removed"
    );
    let body = server
        .instance
        .store
        .read(&slug, &second_sha)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("version two"));
}

// The listing is how a publisher sees what is eating their quota, so the
// bytes recorded for a document need to be visible there.
#[tokio::test]
async fn list_includes_document_size() {
    let server = new_test_server().await;
    let html = format!("<p>{}</p>", "y".repeat(40));
    let (status, _) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Sized Doc", "html": html}),
    )
    .await;
    assert_eq!(status, 201);
    let (status, payload) = post(&server.url, "/api/list", json!({})).await;
    assert_eq!(status, 200);
    let documents = payload["documents"].as_array().unwrap();
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0]["size"], json!(html.len()));
}

// A refused upload must leave nothing behind: an over-quota publisher who
// keeps trying would otherwise fill the disk with orphaned versions.
#[tokio::test]
async fn refused_upload_writes_nothing() {
    let server = with_storage(StorageLimit {
        total: 1 << 30,
        per_owner: 10,
        documents_per_owner: 50,
        uploads_per_hour: 50,
    })
    .await;
    let (status, _) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Too Big", "html": "<p>this is more than ten bytes</p>"}),
    )
    .await;
    assert_eq!(status, 507);
    let stored = server
        .instance
        .store
        .blobs
        .list("documents/")
        .await
        .unwrap();
    assert!(
        stored.is_empty(),
        "a refused upload left objects behind: {stored:?}"
    );
}
