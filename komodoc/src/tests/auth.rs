use base64::Engine;
use serde_json::json;

use super::*;
use crate::auth::{now_unix, read_session, sign, sign_session, Identity, Policy, TokenCache};
use crate::config::Configuration;
use crate::util::first_of;

#[test]
fn policies() {
    for (value, login, allowed) in [
        ("anyone", "", true),
        ("anyone", "someone", true),
        ("any", "", false),
        ("any", "someone", true),
        ("vincent", "vincent", true),
        ("vincent", "Vincent", true), // GitHub logins are case-insensitive
        ("vincent", "stranger", false),
        ("vincent", "", false),
        ("alice, bob", "bob", true),
        ("alice, bob", "carol", false),
        ("", "anyone at all", false), // unconfigured allows nobody
    ] {
        assert_eq!(
            Policy::parse(value).allows(login),
            allowed,
            "Policy::parse({value:?}).allows({login:?})"
        );
    }
}

#[test]
fn session_cookies() {
    let key = b"0123456789abcdef0123456789abcdef";
    let id = Identity {
        login: "vincent".into(),
        id: "42".into(),
    };
    let valid = sign_session(key, &id, now_unix() + 3600);
    assert_eq!(read_session(key, &valid), id);
    assert!(
        !read_session(key, &sign_session(key, &id, now_unix() - 3600)).is_signed_in(),
        "an expired session was accepted"
    );
    let other = b"ffffffffffffffffffffffffffffffff";
    assert!(
        !read_session(other, &valid).is_signed_in(),
        "a cookie signed with another key was accepted"
    );
    // Flipping a character of the payload must invalidate the signature.
    let tampered = format!("X{}", &valid[1..]);
    assert!(
        !read_session(key, &tampered).is_signed_in(),
        "a tampered cookie was accepted"
    );
    // The old cookie shape carried only login|expiry. It must not be accepted
    // as though the missing id were merely empty.
    let old_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(format!("vincent|{}", now_unix() + 3600));
    let old_cookie = format!("{old_payload}.{}", sign(key, &old_payload));
    assert!(
        !read_session(key, &old_cookie).is_signed_in(),
        "an old two-field cookie was accepted"
    );
}

#[tokio::test]
async fn comment_policy_refuses_and_attributes() {
    let server = test_server_with(
        Configuration::default(),
        Policy::parse(TEST_PUBLISHER),
        Policy::parse("any"),
        true,
    )
    .await;
    let slug = text(&publish_test_document(&server.url).await, "slug");
    let path = format!("/api/documents/{slug}/comments");
    let comment = json!({"type": "comment", "exact": "hello", "body": "hi", "creator": "Impostor"});

    let (status, payload) = post_as("", &server.url, &path, comment.clone()).await;
    assert_eq!(status, 400, "anonymous comment got {status} {payload}");
    assert_eq!(text(&payload, "message"), "sign in with GitHub to comment");

    // Signed in: the name on the comment is the verified login, not the one
    // the client asked for.
    let (status, payload) = post_as(&session_as("someone"), &server.url, &path, comment).await;
    assert_eq!(status, 200, "signed-in comment got {status} {payload}");
    assert_eq!(payload["comment"]["creator"], "someone");
}

// Rule F's caching half: a verified token is not re-checked against GitHub on
// every request, and neither is one that fails, though a real deployment
// trusts the failure for a much shorter time.
#[tokio::test]
async fn token_cache_caches_positive_and_negative_answers() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let cache = TokenCache::new();
    let calls = std::sync::Arc::new(AtomicUsize::new(0));
    let good = Identity {
        login: "vincent".into(),
        id: "1".into(),
    };
    let check = |calls: std::sync::Arc<AtomicUsize>, good: Identity| {
        move |token: String| {
            calls.fetch_add(1, Ordering::SeqCst);
            async move { (token == "good-token").then_some(good) }
        }
    };

    assert_eq!(
        cache
            .verify(check(calls.clone(), good.clone()), "good-token")
            .await,
        good
    );
    assert_eq!(
        cache
            .verify(check(calls.clone(), good.clone()), "good-token")
            .await,
        good
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a cached positive answer made a second call"
    );

    assert!(!cache
        .verify(check(calls.clone(), good.clone()), "bad-token")
        .await
        .is_signed_in());
    assert!(!cache
        .verify(check(calls.clone(), good.clone()), "bad-token")
        .await
        .is_signed_in());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "a cached negative answer made a third call"
    );

    // An empty token is never even asked about: there is nothing to verify.
    assert!(!cache
        .verify(check(calls.clone(), good.clone()), "")
        .await
        .is_signed_in());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn describe_policy() {
    for (value, want) in [
        ("anyone", "anyone"),
        ("any", "any GitHub account"),
        ("vincent", "@vincent"),
        ("alice,bob", "@alice, @bob"),
        ("", "nobody (unconfigured)"),
    ] {
        assert_eq!(Policy::parse(value).describe(), want);
    }
    assert!(Policy::parse("anyone").public);
    assert!(
        !Policy::parse("any").public,
        "any is not public: it still needs an account"
    );
}

#[tokio::test]
async fn auth_endpoints() {
    let server = new_test_server().await;
    // The client id is public, so the CLI can ask for it before signing in.
    let (_, payload) = get_json(&server.url, "/api/auth/config").await;
    assert_eq!(payload["client_id"], "test-client");

    let (status, payload) = get_json_as(&session_as(TEST_PUBLISHER), &server.url, "/api/me").await;
    assert!(
        status == 200 && payload["login"] == TEST_PUBLISHER && payload["can_publish"] == true,
        "{payload}"
    );
    let (status, payload) = get_json(&server.url, "/api/me").await;
    assert!(
        status == 200 && payload["login"] == "" && payload["can_publish"] == false,
        "{payload}"
    );
    assert!(
        text(&payload, "publishers").contains(TEST_PUBLISHER),
        "/api/me should say who may publish"
    );
}

#[tokio::test]
async fn signed_in_comments_are_named_by_the_account() {
    // Commenting is open to anyone here, so a name may be typed. Signing in
    // should still override it: the account is the author.
    let server = new_test_server().await;
    let slug = text(&publish_test_document(&server.url).await, "slug");
    let path = format!("/api/documents/{slug}/comments");
    let comment = json!({"type": "comment", "exact": "hello", "body": "hi", "creator": "Impostor"});

    let (status, payload) =
        post_as(&session_as("someone"), &server.url, &path, comment.clone()).await;
    assert_eq!(status, 200);
    assert_eq!(payload["comment"]["creator"], "someone");

    // Anonymous readers still name themselves.
    let (status, payload) = post_as("", &server.url, &path, comment).await;
    assert_eq!(status, 200);
    assert_eq!(payload["comment"]["creator"], "Impostor");
}

#[tokio::test]
async fn annotation_kinds() {
    let server = new_test_server().await;
    let slug = text(&publish_test_document(&server.url).await, "slug");
    let path = format!("/api/documents/{slug}/comments");

    // A highlight is the passage itself, so it needs no words.
    let (status, payload) = post(
        &server.url,
        &path,
        json!({"type": "comment", "exact": "hello", "motivation": "highlighting", "body": ""}),
    )
    .await;
    assert_eq!(status, 200, "a bodyless highlight was refused: {payload}");
    assert_eq!(payload["comment"]["motivation"], "highlighting");

    // Every other kind still needs something said.
    let (status, payload) = post(
        &server.url,
        &path,
        json!({"type": "comment", "exact": "hello", "motivation": "commenting", "body": ""}),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(text(&payload, "message"), "comment body is required");

    // A motivation this deployment does not know is stored as the default
    // rather than refused.
    let (status, payload) = post(
        &server.url,
        &path,
        json!({"type": "comment", "exact": "hello", "motivation": "editing", "body": "a remark"}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(payload["comment"]["motivation"], "commenting");
    assert!(payload["comment"].get("replacement").is_none());
}

#[tokio::test]
async fn tags_are_normalised() {
    let server = new_test_server().await;
    let slug = text(&publish_test_document(&server.url).await, "slug");
    let path = format!("/api/documents/{slug}/comments");
    let (status, payload) = post(
        &server.url,
        &path,
        json!({"type": "comment", "exact": "hello", "body": "x",
            // Mixed case, padding, a duplicate, and more than the cap allows.
            "tags": ["Methods", " methods ", "TYPO", "a", "b", "c", "d", "e", "f"]}),
    )
    .await;
    assert_eq!(status, 200, "tagged comment got {status} {payload}");
    let tags = payload["comment"]["tags"].as_array().unwrap();
    assert_eq!(tags.len(), Configuration::default().max_tags, "{tags:?}");
    assert!(
        tags[0] == "methods" && tags[1] == "typo",
        "tags were not normalised or deduplicated: {tags:?}"
    );
}

#[tokio::test]
async fn region_annotations() {
    let server = new_test_server().await;
    let slug = text(&publish_test_document(&server.url).await, "slug");
    let path = format!("/api/documents/{slug}/comments");

    // A rectangle on a figure anchors an annotation, with no quotation at all.
    let (status, payload) = post(
        &server.url,
        &path,
        json!({"type": "comment", "exact": "", "body": "the axis is unlabelled",
            "region": {"image_digest": "abc123", "image_index": 1, "x": 10.5, "y": 20, "w": 30, "h": 25}}),
    )
    .await;
    assert_eq!(status, 200, "region annotation got {status} {payload}");
    let stored = &payload["comment"]["region"];
    assert!(
        stored["image_digest"] == "abc123" && stored["image_index"] == 1 && stored["x"] == 10.5,
        "{stored}"
    );

    // Neither words nor a figure is nothing to point at.
    let (status, payload) = post(
        &server.url,
        &path,
        json!({"type": "comment", "exact": "", "body": "about what?"}),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(
        text(&payload, "message"),
        "select some text or part of a figure to comment on"
    );

    // A rectangle outside the image, or too small to see, is not one.
    for (name, spot) in [
        (
            "off the image",
            json!({"image_index": 0, "x": 90, "y": 10, "w": 30, "h": 10}),
        ),
        (
            "negative",
            json!({"image_index": 0, "x": -5, "y": 10, "w": 10, "h": 10}),
        ),
        (
            "a click",
            json!({"image_index": 0, "x": 10, "y": 10, "w": 0.1, "h": 0.1}),
        ),
        (
            "no image",
            json!({"image_index": -1, "x": 10, "y": 10, "w": 10, "h": 10}),
        ),
    ] {
        let (status, payload) = post(
            &server.url,
            &path,
            json!({"type": "comment", "exact": "", "body": "x", "region": spot}),
        )
        .await;
        assert_eq!(status, 400, "{name}: got {status} {payload}");
    }
}

#[test]
fn settings_may_be_quoted() {
    // A .env read by make keeps the quotes a shell would strip, and a client id
    // wearing quotation marks is one GitHub has never heard of.
    for (value, want) in [
        ("\"Ov23li\"", "Ov23li"),
        ("'Ov23li'", "Ov23li"),
        ("  Ov23li ", "Ov23li"),
        ("Ov23li", "Ov23li"),
        ("\"", "\""),
        ("", ""),
    ] {
        assert_eq!(first_of(&[value]), want, "first_of({value:?})");
    }
    // The first value that is not empty still wins.
    assert_eq!(first_of(&["", "\"second\"", "third"]), "second");
}
