use serde_json::json;

use super::*;
use crate::config::Configuration;
use crate::store::{digest_of, Publication};

#[tokio::test]
async fn upload_needs_a_signed_in_publisher() {
    let server = new_test_server().await;
    let (status, payload) = post_as(
        "",
        &server.url,
        "/api/documents",
        json!({"title": "x", "html": "<p>x</p>"}),
    )
    .await;
    assert_eq!(status, 401, "anonymous upload got {status} {payload}");
    assert_eq!(text(&payload, "error"), "sign in with GitHub to publish");

    // Signed in, but not one of the allowed logins.
    let (status, payload) = post_as(
        &session_as("stranger"),
        &server.url,
        "/api/documents",
        json!({"title": "x", "html": "<p>x</p>"}),
    )
    .await;
    assert_eq!(status, 403, "stranger's upload got {status} {payload}");
    assert!(text(&payload, "error").contains("@stranger may not publish"));

    // A forged cookie is not a sign-in.
    let (status, _) = post_as(
        &format!("{}=nonsense.nonsense", crate::auth::SESSION_COOKIE),
        &server.url,
        "/api/documents",
        json!({"title": "x", "html": "<p>x</p>"}),
    )
    .await;
    assert_eq!(status, 401, "forged cookie got {status}");
}

#[tokio::test]
async fn publish_then_serve_document() {
    let server = new_test_server().await;
    let document = publish_test_document(&server.url).await;
    let config = Configuration::default();

    let slug = text(&document, "slug");
    assert!(
        slug.starts_with("my-paper-") && slug.len() == "my-paper-".len() + config.suffix_length,
        "slug {slug}"
    );
    assert_eq!(text(&document, "url"), format!("/docs/{slug}"));

    let sha = text(&document, "sha");
    let version = format!("/raw/{slug}/{sha}.html");

    // Asking the reader's own host for a document sends you to the other one.
    let redirect = client()
        .get(format!("{}{version}", server.url))
        .send()
        .await
        .unwrap();
    assert_eq!(redirect.status().as_u16(), 302);
    let location = redirect
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.contains("docs."), "redirected to {location}");

    // On the document host it is served, with the agent added and a CSP that
    // lets it run its own scripts while pinning who may frame it.
    let response = on_docs_host(&server.url, &version).await;
    let csp = response
        .headers()
        .get("content-security-policy")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let body = response.text().await.unwrap();
    assert!(body.contains("hello world"), "document body was {body}");
    assert!(
        body.contains("<script src=\"/agent.js?reader="),
        "the in-frame agent was not injected: {body}"
    );
    assert!(
        csp.contains("script-src 'self'") && csp.contains("frame-ancestors http://"),
        "document CSP was {csp}"
    );

    // The agent itself is served from the document origin, and nothing else is.
    assert_eq!(
        on_docs_host(&server.url, "/agent.js")
            .await
            .status()
            .as_u16(),
        200
    );
    for path in ["/", "/api/me", &format!("/docs/{slug}"), "/komodoc.css"] {
        assert_eq!(
            on_docs_host(&server.url, path).await.status().as_u16(),
            404,
            "{path} exists on the document origin"
        );
    }

    // The stable URL redirects to the current version, on the document origin.
    let redirect = client()
        .get(format!("{}/raw/{slug}", server.url))
        .send()
        .await
        .unwrap();
    assert_eq!(redirect.status().as_u16(), 302);
    let location = redirect
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(location.ends_with(&version), "got {location}");
}

#[tokio::test]
async fn republish_keeps_slug_and_comments() {
    let server = new_test_server().await;
    let first = publish_test_document(&server.url).await;
    let slug = text(&first, "slug");

    let mut socket = dial_websocket(&server.url, &slug).await;
    socket.read().await; // hello
    socket
        .write(json!({"type": "comment", "exact": "hello", "body": "a note", "creator": "Reader"}))
        .await;
    socket.read().await;

    let (status, second) = post(
        &server.url,
        "/api/documents",
        json!({"title": "My Paper", "slug": slug, "html": "<!doctype html><p>hello revised world</p>"}),
    )
    .await;
    assert_eq!(status, 201, "republish returned {status}: {second}");
    assert_eq!(text(&second, "slug"), slug);
    assert_ne!(
        text(&second, "sha"),
        text(&first, "sha"),
        "republish should store a new version"
    );
    assert_eq!(
        text(&second, "created_at"),
        text(&first, "created_at"),
        "republish should keep the creation time"
    );

    let (_, document) = get_json(&server.url, &format!("/api/documents/{slug}")).await;
    assert_eq!(
        document["comment_count"],
        json!(1),
        "comment did not survive the republish: {document}"
    );
}

#[tokio::test]
async fn comments_broadcast_to_every_reader() {
    let server = new_test_server().await;
    let slug = text(&publish_test_document(&server.url).await, "slug");

    let mut author = dial_websocket(&server.url, &slug).await;
    let mut watcher = dial_websocket(&server.url, &slug).await;
    for socket in [&mut author, &mut watcher] {
        let hello = socket.read().await;
        assert_eq!(hello["type"], "hello");
    }

    author
        .write(json!({"type": "comment", "exact": "hello", "body": "a note", "creator": "Reader", "temp_id": "t1"}))
        .await;

    // The sender gets the echo too, which is how it reconciles the row it
    // drew optimistically.
    for socket in [&mut author, &mut watcher] {
        let event = socket.read().await;
        assert_eq!(event["type"], "comment", "got {event}");
        let comment = &event["comment"];
        assert_eq!(comment["body"], "a note");
        assert_eq!(comment["creator"], "Reader");
        assert_eq!(comment["exact"], "hello");
        assert_eq!(comment["resolved"], false);
        assert_eq!(comment["seq"], 1);
    }

    // Replies and resolves reach everyone the same way.
    let (_, listing) = get_json(&server.url, &format!("/api/documents/{slug}/comments")).await;
    let comments = listing["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 1);
    let identifier = text(&comments[0], "id");

    author
        .write(json!({"type": "resolve", "comment_id": identifier, "resolved": true}))
        .await;
    let event = watcher.read().await;
    assert_eq!(event["type"], "resolve");
    assert_eq!(event["resolved"], true);
    assert!(
        !event["resolved_at"].is_null(),
        "resolve came back as {event}"
    );
}

#[tokio::test]
async fn comment_validation() {
    let server = new_test_server().await;
    let slug = text(&publish_test_document(&server.url).await, "slug");
    let mut socket = dial_websocket(&server.url, &slug).await;
    socket.read().await;

    let cases = [
        (
            "no body",
            json!({"type": "comment", "exact": "hello"}),
            "comment body is required",
        ),
        (
            "no anchor",
            json!({"type": "comment", "body": "x"}),
            "select some text or part of a figure to comment on",
        ),
        (
            "unknown type",
            json!({"type": "wat", "body": "x"}),
            "unknown message type",
        ),
        (
            "unknown comment",
            json!({"type": "reply", "comment_id": "nope", "body": "x"}),
            "unknown comment",
        ),
    ];
    for (name, message, want) in cases {
        socket.write(message).await;
        let event = socket.read().await;
        assert_eq!(event["type"], "error", "{name}: got {event}");
        assert_eq!(event["message"], want, "{name}: got {event}");
    }
}

#[tokio::test]
async fn listing_and_delete() {
    let server = new_test_server().await;
    let slug = text(&publish_test_document(&server.url).await, "slug");

    // Knowing a document's link must not reveal the others.
    let (status, payload) = post_as("", &server.url, "/api/list", json!({})).await;
    assert_eq!(status, 401, "anonymous listing returned {status} {payload}");
    let (status, payload) = post(&server.url, "/api/list", json!({})).await;
    assert_eq!(status, 200);
    assert_eq!(payload["documents"].as_array().unwrap().len(), 1);

    let (status, deleted) = post(
        &server.url,
        &format!("/api/documents/{slug}/delete"),
        json!({}),
    )
    .await;
    assert_eq!(status, 200, "delete returned {status} {deleted}");
    assert_eq!(deleted["deleted"], slug);
    assert_eq!(deleted["versions_removed"], 1);
    let (status, _) = get_json(&server.url, &format!("/api/documents/{slug}")).await;
    assert_eq!(status, 404, "document still present after delete");
}

#[tokio::test]
async fn shell_routes() {
    let server = new_test_server().await;
    // The reader is only served for a document that exists.
    let slug = "a-paper-abcdefghij";
    server
        .instance
        .store
        .put(Publication {
            slug: slug.into(),
            title: "A Paper".into(),
            digest: digest_of("<p>p</p>"),
            html: "<p>p</p>".into(),
            ..Publication::default()
        })
        .await
        .unwrap();
    for (path, want) in [
        ("/", "<!doctype html"),
        (&format!("/docs/{slug}"), "<!doctype html"),
        ("/anchor.js", "export"),
    ] {
        let response = client()
            .get(format!("{}{path}", server.url))
            .send()
            .await
            .unwrap();
        let status = response.status().as_u16();
        let body = response.text().await.unwrap().to_lowercase();
        assert!(
            status == 200 && body.contains(want),
            "{path} returned {status}, body did not contain {want:?}"
        );
    }
}

// A link nobody can follow used to answer 200 with the reader shell, which
// left the reader looking broken rather than the link. It gets the 404 page,
// with the status to match.
#[tokio::test]
async fn unknown_paths_get_the_not_found_page() {
    let server = new_test_server().await;
    for path in ["/docs/no-such-document", "/nothing-here"] {
        let response = client()
            .get(format!("{}{path}", server.url))
            .header("accept", "text/html")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 404, "{path}");
        let body = response.text().await.unwrap();
        assert!(
            body.contains("Nothing here"),
            "{path} did not serve the 404 page: {:.80}",
            body
        );
    }
}

#[tokio::test]
async fn oversized_upload_is_refused_by_size() {
    let server = new_test_server().await;
    let html = format!("<p>{}</p>", "x".repeat(Configuration::default().max_html));
    let (status, payload) = post(
        &server.url,
        "/api/documents",
        json!({"title": "Big", "html": html}),
    )
    .await;
    assert_eq!(status, 413, "got {status} {payload}");
    assert_eq!(text(&payload, "error"), "document too large");
}

#[tokio::test]
async fn document_origin_is_isolated() {
    let server = new_test_server().await;
    let slug = text(&publish_test_document(&server.url).await, "slug");

    // The reader is told where to frame documents from, and it is not itself.
    let (_, payload) = get_json(&server.url, &format!("/api/documents/{slug}")).await;
    let origin = text(&payload, "docs_origin");
    assert!(
        origin.starts_with("http://docs."),
        "docs_origin was {origin}"
    );
    assert!(!origin.trim_start_matches("http://docs.").is_empty() && origin != server.url);

    // A session cookie counts for nothing on the document origin: it serves
    // no API at all, so there is nothing there to authorise.
    let host = format!("docs.{}", server.url.trim_start_matches("http://"));
    let refused = client()
        .post(format!("{}/api/documents", server.url))
        .header("host", host)
        .header("cookie", session_as(TEST_PUBLISHER))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status().as_u16(), 404);
}

#[tokio::test]
async fn markdown_upload_is_rendered_by_serve() {
    let server = new_test_server().await;
    let part = reqwest::multipart::Part::text("# Notes\n\nA *point* worth making.\n")
        .file_name("notes.md");
    let form = reqwest::multipart::Form::new().part("file", part);
    let response = client()
        .post(format!("{}/api/documents", server.url))
        .header("cookie", session_as(TEST_PUBLISHER))
        .header("x-komodoc-client", "1")
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status().as_u16(),
        201,
        "markdown upload returned {}",
        response.status()
    );
    let document: serde_json::Value = response.json().await.unwrap();
    // The first heading names it when no title was given.
    assert_eq!(document["title"], "Notes");
    // What is stored is HTML, not the markdown source.
    let stored = on_docs_host(
        &server.url,
        &format!(
            "/raw/{}/{}.html",
            text(&document, "slug"),
            text(&document, "sha")
        ),
    )
    .await;
    let page = stored.text().await.unwrap();
    assert!(
        page.contains("<em>point</em>") && !page.contains("# Notes"),
        "the stored document is not rendered HTML: {page}"
    );
}

// The editor is useless without the renderer, so the server has to serve it:
// as WebAssembly, and cacheable for a year.
#[tokio::test]
async fn the_renderer_is_served_as_wasm() {
    let server = new_test_server().await;
    let response = client()
        .get(format!(
            "{}{}",
            server.url,
            crate::assets::module_url("markdown").expect("a markdown module")
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/wasm"
    );
    assert!(response
        .headers()
        .get("cache-control")
        .unwrap()
        .to_str()
        .unwrap()
        .contains("immutable"));
    let raw = response.bytes().await.unwrap();
    assert!(
        raw.len() > 8 && &raw[..4] == b"\0asm",
        "the module is not WebAssembly"
    );
    assert!(
        raw.len() < 3 * 1024 * 1024,
        "the markdown module is {} bytes; it should be small",
        raw.len()
    );
}
