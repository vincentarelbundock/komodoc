//! Where publishing takes a GitHub account, a publisher sees the reserved
//! examples and their own uploads, and nobody else's document is theirs to
//! replace or delete.

use serde_json::json;

use super::*;
use crate::auth::Policy;
use crate::config::Configuration;
use crate::store::{digest_of, Publication};

/// The sandbox shape: any signed-in GitHub account may publish, so several
/// publishers share one deployment.
async fn any_publisher_server() -> TestServer {
    test_server_with(
        Configuration::default(),
        Policy::parse("any"),
        Policy::parse("anyone"),
        true,
    )
    .await
}

/// Uploads a document as one publisher, returning its slug. An explicit slug
/// is what asks to replace an existing document.
async fn publish_as(base: &str, login: &str, title: &str, slug: Option<&str>) -> String {
    publish_with(base, &session_as(login), title, slug).await
}

/// Uploads carrying whatever cookie is given, including none.
async fn publish_with(base: &str, cookie: &str, title: &str, slug: Option<&str>) -> String {
    let mut body = json!({"title": title, "html": format!("<p>{title}</p>")});
    if let Some(slug) = slug {
        body["slug"] = json!(slug);
    }
    let (status, document) = post_as(cookie, base, "/api/documents", body).await;
    assert_eq!(status, 201, "upload returned {status}: {document}");
    text(&document, "slug")
}

/// What /api/list shows a caller carrying one cookie.
async fn slugs_visible_with(base: &str, cookie: &str) -> Vec<String> {
    let (status, payload) = post_as(cookie, base, "/api/list", json!(null)).await;
    assert_eq!(status, 200, "/api/list returned {status}: {payload}");
    payload["documents"]
        .as_array()
        .expect("documents")
        .iter()
        .map(|d| text(d, "slug"))
        .collect()
}

#[tokio::test]
async fn listing_shows_only_your_own_uploads() {
    let server = any_publisher_server().await;
    let mine = publish_as(&server.url, "alice", "Alice Paper", None).await;
    let theirs = publish_as(&server.url, "bob", "Bob Paper", None).await;

    // An example belongs to everyone, and a document published before
    // ownership was recorded belongs to no one in particular.
    let store = &server.instance.store;
    store
        .put(Publication {
            slug: "example-doc".into(),
            title: "Example".into(),
            digest: digest_of("<p>e</p>"),
            html: "<p>e</p>".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    {
        let mut state = store.state.lock().await;
        state.entries.get_mut("example-doc").unwrap().example = true;
    }
    store
        .put(Publication {
            slug: "legacy-doc".into(),
            title: "Legacy".into(),
            digest: digest_of("<p>l</p>"),
            html: "<p>l</p>".into(),
            ..Default::default()
        })
        .await
        .unwrap();

    let visible = slugs_visible_with(&server.url, &session_as("alice")).await;
    assert!(
        visible.contains(&mine),
        "@alice should see her own document, got {visible:?}"
    );
    assert!(
        !visible.contains(&theirs),
        "@alice should not see @bob's document, got {visible:?}"
    );
    assert!(
        visible.contains(&"example-doc".to_string()) && visible.contains(&"legacy-doc".to_string()),
        "{visible:?}"
    );
}

// Publishing without any sign-in still belongs to the browser that did it, so
// one visitor's uploads are not listed to the next.
#[tokio::test]
async fn anonymous_visitors_do_not_see_each_other() {
    let server = test_server_with(
        Configuration::default(),
        Policy::parse("anyone"),
        Policy::parse("anyone"),
        true,
    )
    .await;
    let mine = publish_with(&server.url, &visitor_as("alpha"), "First Paper", None).await;
    let theirs = publish_with(&server.url, &visitor_as("beta"), "Second Paper", None).await;
    let visible = slugs_visible_with(&server.url, &visitor_as("alpha")).await;
    assert!(
        visible.contains(&mine) && !visible.contains(&theirs),
        "{visible:?}"
    );
}

// The shell is what hands a browser its name, so the first page load carries
// the cookie every later upload is owned by.
#[tokio::test]
async fn the_shell_names_a_new_browser() {
    let server = new_test_server().await;
    let response = client()
        .get(format!("{}/", server.url))
        .send()
        .await
        .unwrap();
    let cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    assert!(
        cookies.iter().any(
            |c| c.starts_with(&format!("{}=", crate::auth::VISITOR_COOKIE))
                && c.contains("HttpOnly")
        ),
        "the index page set no visitor cookie: {cookies:?}"
    );
}

// Deleting another browser's document is a not-found, exactly as it is for
// another GitHub account's.
#[tokio::test]
async fn anonymous_visitor_cannot_delete_anothers_document() {
    let server = test_server_with(
        Configuration::default(),
        Policy::parse("anyone"),
        Policy::parse("anyone"),
        true,
    )
    .await;
    let theirs = publish_with(&server.url, &visitor_as("beta"), "Second Paper", None).await;
    let (status, payload) = post_as(
        &visitor_as("alpha"),
        &server.url,
        &format!("/api/documents/{theirs}/delete"),
        json!(null),
    )
    .await;
    assert_eq!(
        status, 404,
        "deleting another browser's document returned {status}: {payload}"
    );
    assert!(
        server.instance.store.get(&theirs).await.is_some(),
        "another browser's document was deleted"
    );
}

// The CLI publishing to a deployment open to everyone carries no cookie and
// no token, so its uploads belong to nobody and stay shared.
#[tokio::test]
async fn uploads_with_no_identity_stay_shared() {
    let server = test_server_with(
        Configuration::default(),
        Policy::parse("anyone"),
        Policy::parse("anyone"),
        true,
    )
    .await;
    let slug = publish_with(&server.url, "", "Command Line Paper", None).await;
    let entry = server
        .instance
        .store
        .get(&slug)
        .await
        .expect("the document");
    assert!(
        entry.publisher.is_empty(),
        "an unidentified upload should own nothing: {entry:?}"
    );
    assert!(
        slugs_visible_with(&server.url, &visitor_as("alpha"))
            .await
            .contains(&slug),
        "unowned documents stay shared"
    );
}

#[tokio::test]
async fn another_publishers_slug_is_not_replaced() {
    let server = any_publisher_server().await;
    let theirs = publish_as(&server.url, "bob", "Bob Paper", None).await;
    // Guessing the slug is the whole attack: an upload aimed straight at it
    // becomes a new document of @alice's instead of a replacement.
    let mine = publish_as(&server.url, "alice", "Alice Paper", Some(&theirs)).await;
    assert_ne!(mine, theirs, "@alice took over @bob's slug");
    let entry = server
        .instance
        .store
        .get(&theirs)
        .await
        .expect("bob's document");
    assert_eq!(entry.publisher, "bob", "@bob's document changed hands");
    let body = server
        .instance
        .store
        .read(&theirs, &entry.sha)
        .await
        .unwrap();
    assert_eq!(body, b"<p>Bob Paper</p>", "@bob's bytes were overwritten");
}

#[tokio::test]
async fn your_own_slug_is_still_replaced_in_place() {
    let server = any_publisher_server().await;
    let first = publish_as(&server.url, "alice", "Alice Paper", None).await;
    let again = publish_as(&server.url, "alice", "Alice Paper", Some(&first)).await;
    assert_eq!(first, again, "republishing should keep the URL");
}

#[tokio::test]
async fn deleting_another_publishers_document_is_a_not_found() {
    let server = any_publisher_server().await;
    let theirs = publish_as(&server.url, "bob", "Bob Paper", None).await;
    let (status, payload) = post_as(
        &session_as("alice"),
        &server.url,
        &format!("/api/documents/{theirs}/delete"),
        json!(null),
    )
    .await;
    assert_eq!(
        status, 404,
        "@alice deleting @bob's document returned {status}: {payload}"
    );
    assert!(server.instance.store.get(&theirs).await.is_some());
    let (status, payload) = post_as(
        &session_as("bob"),
        &server.url,
        &format!("/api/documents/{theirs}/delete"),
        json!(null),
    )
    .await;
    assert_eq!(
        status, 200,
        "@bob deleting his own document returned {status}: {payload}"
    );
}
