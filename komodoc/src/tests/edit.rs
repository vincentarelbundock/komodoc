use serde_json::{json, Value};

use super::*;
use crate::blob::BlobError;
use crate::render::render_markdown_document;

pub const TEST_MARKDOWN: &str = "# My Paper\n\nHello *world*.\n";

/// Asks for a document's editable source carrying whatever cookie is given.
async fn get_source_as(cookie: &str, base: &str, slug: &str) -> (u16, Value) {
    get_json_as(cookie, base, &format!("/api/documents/{slug}/source")).await
}

/// Publishes a document the way the CLI publishes markdown: the rendered
/// HTML, and the source it was rendered from.
pub async fn publish_with_source(base: &str) -> Value {
    let html = render_markdown_document(TEST_MARKDOWN, "My Paper");
    let (status, document) = post(
        base,
        "/api/documents",
        json!({"title": "My Paper", "html": html, "source": TEST_MARKDOWN, "source_format": "markdown"}),
    )
    .await;
    assert_eq!(status, 201, "upload returned {status}: {document}");
    document
}

// What was published comes back byte for byte, so reopening a document shows
// what its author wrote rather than something reconstructed from the HTML.
#[tokio::test]
async fn source_round_trips() {
    let server = new_test_server().await;
    let document = publish_with_source(&server.url).await;
    let slug = text(&document, "slug");
    let (status, payload) = get_source_as(&session_as(TEST_PUBLISHER), &server.url, &slug).await;
    assert_eq!(status, 200, "source returned {status}: {payload}");
    assert_eq!(text(&payload, "source"), TEST_MARKDOWN);
    assert_eq!(text(&payload, "format"), "markdown");
    assert_eq!(text(&payload, "sha"), text(&document, "sha"));
}

// A document published as HTML has no source but itself, and says so rather
// than handing back an empty one an editor would then save over it.
#[tokio::test]
async fn source_absent_for_html_documents() {
    let server = new_test_server().await;
    let slug = text(&publish_test_document(&server.url).await, "slug");
    let (status, payload) = get_source_as(&session_as(TEST_PUBLISHER), &server.url, &slug).await;
    assert_eq!(status, 404);
    assert!(
        text(&payload, "error").contains("no stored source"),
        "{payload}"
    );
}

// Replacing a markdown document with HTML drops the source with it: the old
// markdown no longer says what the document says.
#[tokio::test]
async fn publishing_html_drops_a_stale_source() {
    let server = new_test_server().await;
    let slug = text(&publish_with_source(&server.url).await, "slug");
    let (status, document) = post(
        &server.url,
        "/api/documents",
        json!({"title": "My Paper", "slug": slug, "html": "<!doctype html><p>plain</p>"}),
    )
    .await;
    assert_eq!(status, 201, "replacement returned {status}: {document}");
    let (status, _) = get_source_as(&session_as(TEST_PUBLISHER), &server.url, &slug).await;
    assert_eq!(status, 404);
    assert!(
        matches!(
            server.instance.store.read_source(&slug).await,
            Err(BlobError::NotFound)
        ),
        "the stale source is still stored"
    );
}

// The source is stored, so it is charged: an entry's size is what the
// document actually occupies.
#[tokio::test]
async fn source_counts_toward_the_quota() {
    let server = new_test_server().await;
    let document = publish_with_source(&server.url).await;
    let slug = text(&document, "slug");
    let entry = server
        .instance
        .store
        .get(&slug)
        .await
        .expect("the document is in the index");
    let stored = server.instance.store.read(&slug, &entry.sha).await.unwrap();
    assert_eq!(
        entry.size,
        (stored.len() + TEST_MARKDOWN.len()) as i64,
        "entry size should be html plus source"
    );
}

// A source is not published: only the account that may replace the document
// may read what it was written from.
#[tokio::test]
async fn source_is_owner_only() {
    let server = new_test_server().await;
    let slug = text(&publish_with_source(&server.url).await, "slug");
    for cookie in ["", &session_as("stranger")] {
        let (status, payload) = get_source_as(cookie, &server.url, &slug).await;
        assert_eq!(
            status, 404,
            "source read with cookie {cookie:?} got {status} {payload}"
        );
    }
}

// A source in a format this project knows nothing about is dropped rather
// than refused: the document itself is fine, it simply cannot be reopened.
#[tokio::test]
async fn unknown_source_format_is_dropped() {
    let server = new_test_server().await;
    let (status, document) = post(
        &server.url,
        "/api/documents",
        json!({"title": "My Paper", "html": "<!doctype html><p>x</p>", "source": "\\documentclass{article}", "source_format": "latex"}),
    )
    .await;
    assert_eq!(status, 201);
    let (status, _) = get_source_as(
        &session_as(TEST_PUBLISHER),
        &server.url,
        &text(&document, "slug"),
    )
    .await;
    assert_eq!(status, 404);
}

// Saving in the editor is publishing a revision, and a revision keeps the
// comments on the document: they re-anchor in the reader.
#[tokio::test]
async fn saving_a_revision_keeps_comments() {
    let server = new_test_server().await;
    let slug = text(&publish_with_source(&server.url).await, "slug");
    let (status, _) = post(
        &server.url,
        &format!("/api/documents/{slug}/comments"),
        json!({"type": "comment", "exact": "world", "body": "still here?", "creator": "Reader"}),
    )
    .await;
    assert_eq!(status, 200);

    let edited = "# My Paper\n\nHello *world*, and a new sentence.\n";
    let html = render_markdown_document(edited, "My Paper");
    let (status, document) = post(
        &server.url,
        "/api/documents",
        json!({"title": "My Paper", "slug": slug, "html": html, "source": edited, "source_format": "markdown"}),
    )
    .await;
    assert_eq!(status, 201, "saving returned {status}: {document}");

    let (_, listing) = get_json(&server.url, &format!("/api/documents/{slug}/comments")).await;
    let comments = listing["comments"].as_array().unwrap();
    assert!(
        comments.len() == 1 && comments[0]["body"] == "still here?",
        "comments after a save: {comments:?}"
    );

    // And the next edit reopens what was just saved.
    let (status, payload) = get_source_as(&session_as(TEST_PUBLISHER), &server.url, &slug).await;
    assert!(
        status == 200 && text(&payload, "source") == edited,
        "source after a save got {status} {payload}"
    );
}

// Two people editing the same document at once. Whoever saves second is
// saving against a version that no longer exists, and without this they would
// silently discard the first person's work.
#[tokio::test]
async fn a_save_against_an_old_version_is_refused() {
    let server = new_test_server().await;
    let document = publish_with_source(&server.url).await;
    let slug = text(&document, "slug");
    let opened = text(&document, "sha");

    let first = "# My Paper\n\nHello *world*, says the first editor.\n";
    let (status, saved) = post(
        &server.url,
        "/api/documents",
        json!({"title": "My Paper", "slug": slug, "html": render_markdown_document(first, "My Paper"),
            "source": first, "source_format": "markdown", "base_sha": opened}),
    )
    .await;
    assert_eq!(status, 201, "the first save returned {status}: {saved}");

    let second = "# My Paper\n\nHello *world*, says the second editor.\n";
    let html = render_markdown_document(second, "My Paper");
    let (status, refused) = post(
        &server.url,
        "/api/documents",
        json!({"title": "My Paper", "slug": slug, "html": html, "source": second, "source_format": "markdown", "base_sha": opened}),
    )
    .await;
    assert_eq!(status, 409, "the stale save returned {status} {refused}");
    assert!(
        text(&refused, "error").contains("published again while you were editing"),
        "{refused}"
    );

    // And nothing was written: the first editor's work is what the document is.
    let (status, current) = get_source_as(&session_as(TEST_PUBLISHER), &server.url, &slug).await;
    assert!(status == 200 && text(&current, "source") == first);

    // Saving against what the document actually is now goes through.
    let (status, accepted) = post(
        &server.url,
        "/api/documents",
        json!({"title": "My Paper", "slug": slug, "html": html, "source": second, "source_format": "markdown", "base_sha": text(&saved, "sha")}),
    )
    .await;
    assert_eq!(
        status, 201,
        "saving against the current version returned {status}: {accepted}"
    );
}

// Publishing a file is a deliberate replacement and says nothing about what
// it replaces, so it carries no base and is never refused for staleness.
#[tokio::test]
async fn publishing_without_a_base_is_not_refused() {
    let server = new_test_server().await;
    let slug = text(&publish_with_source(&server.url).await, "slug");
    for round in 0..2 {
        let (status, document) = post(
            &server.url,
            "/api/documents",
            json!({"title": "My Paper", "slug": slug, "html": "<!doctype html><p>from the command line</p>"}),
        )
        .await;
        assert_eq!(status, 201, "publish {round} returned {status}: {document}");
    }
}
