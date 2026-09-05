use serde_json::Value;

use crate::cli::short_ids;
use crate::config::Configuration;
use crate::export::{render_jsonld, render_markdown, ANNOTATION_CONTEXT};
use crate::room::{Comment, Region, Reply};

fn sample_comments() -> Vec<Comment> {
    vec![Comment {
        id: "11111111-1111-4111-8111-111111111111".into(),
        seq: 1,
        motivation: "commenting".into(),
        exact: "the quick brown fox".into(),
        prefix: "before ".into(),
        suffix: " after".into(),
        body: "is this right?".into(),
        creator: "Vincent".into(),
        created: "2026-09-02T11:00:00Z".into(),
        resolved: true,
        resolved_at: Some("2026-09-02T12:00:00Z".into()),
        replies: vec![Reply {
            id: "22222222-2222-4222-8222-222222222222".into(),
            body: "yes".into(),
            creator: "Reader".into(),
            created: "2026-09-02T11:30:00Z".into(),
            author: String::new(),
        }],
        ..Comment::default()
    }]
}

#[test]
fn export_is_valid_web_annotation() {
    let config = Configuration::default();
    let rendered = render_jsonld(
        "My Paper",
        &sample_comments(),
        "https://example.test/docs/paper-abc",
        &config,
    );
    let page: Value = serde_json::from_str(&rendered).expect("export is not valid JSON");
    assert_eq!(page["@context"], ANNOTATION_CONTEXT);
    assert_eq!(page["type"], "AnnotationPage");
    let items = page["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "want the comment and its reply");

    let first = &items[0];
    assert_eq!(first["type"], "Annotation");
    assert_eq!(first["motivation"], "commenting");
    assert_eq!(first["created"], "2026-09-02T11:00:00Z");
    assert_eq!(first["creator"]["type"], "Person");
    assert_eq!(first["creator"]["name"], "Vincent");
    assert_eq!(first["body"]["type"], "TextualBody");
    assert_eq!(first["body"]["value"], "is this right?");
    assert_eq!(
        first["target"]["source"],
        "https://example.test/docs/paper-abc"
    );
    let selector = &first["target"]["selector"];
    assert_eq!(selector["type"], "TextQuoteSelector");
    assert_eq!(selector["exact"], "the quick brown fox");
    assert_eq!(selector["prefix"], "before ");
    assert_eq!(selector["suffix"], " after");
    // Resolution state is ours, so it must not squat on a spec property name.
    assert_eq!(first["komodoc:resolved"], true);

    // A reply is an annotation motivated by replying, targeting its parent.
    let second = &items[1];
    assert_eq!(second["motivation"], "replying");
    assert_eq!(
        second["target"]["source"],
        "urn:uuid:11111111-1111-4111-8111-111111111111"
    );
    assert!(
        second["target"].get("selector").is_none(),
        "a reply targets an annotation, so it needs no selector"
    );
}

#[test]
fn export_markdown() {
    let config = Configuration::default();
    let rendered = render_markdown(
        "My Paper",
        &sample_comments(),
        "https://example.test/docs/paper-abc",
        &config,
    );
    for want in [
        "# My Paper",
        "## commenting by Vincent (resolved)",
        "> the quick brown fox",
        "is this right?",
        "**Reader**: yes",
    ] {
        assert!(
            rendered.contains(want),
            "markdown export is missing {want:?}:\n{rendered}"
        );
    }
}

#[test]
fn motivation_falls_back_to_the_default() {
    let config = Configuration::default();
    assert_eq!(config.allowed_motivation("commenting"), "commenting");
    assert_eq!(
        config.allowed_motivation("mischief"),
        config.default_motivation
    );
    assert_eq!(config.allowed_motivation(""), config.default_motivation);
}

#[test]
fn export_carries_tags_and_highlights() {
    let items = vec![
        Comment {
            id: "1".into(),
            motivation: "commenting".into(),
            exact: "the quick fox".into(),
            body: "clearer".into(),
            tags: vec!["style".into(), "typo".into()],
            creator: "Vincent".into(),
            created: "2026-09-02T11:00:00Z".into(),
            ..Comment::default()
        },
        // A highlight says nothing, so it has no body at all.
        Comment {
            id: "2".into(),
            motivation: "highlighting".into(),
            exact: "worth returning to".into(),
            creator: "Reader".into(),
            created: "2026-09-02T12:00:00Z".into(),
            ..Comment::default()
        },
    ];
    let page: Value = serde_json::from_str(&render_jsonld(
        "P",
        &items,
        "https://x.test/d",
        &Configuration::default(),
    ))
    .unwrap();
    let all = page["items"].as_array().unwrap();

    // The remark and the two labels, each saying what it is for, which is how
    // the spec expresses this.
    let bodies = all[0]["body"].as_array().expect("three bodies");
    assert_eq!(bodies.len(), 3, "{bodies:?}");
    assert!(bodies
        .iter()
        .any(|b| b["value"] == "clearer" && b.get("purpose").is_none()));
    assert_eq!(
        bodies.iter().filter(|b| b["purpose"] == "tagging").count(),
        2
    );

    // A highlight is a target with nothing said about it.
    assert!(
        all[1].get("body").is_none(),
        "a highlight should export with no body"
    );
}

#[test]
fn export_region_as_fragment_selector() {
    let items = vec![Comment {
        id: "1".into(),
        motivation: "commenting".into(),
        body: "the axis is unlabelled".into(),
        region: Some(Region {
            image_digest: "abc123".into(),
            image_index: 2,
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 25.0,
        }),
        creator: "Vincent".into(),
        created: "2026-09-03T10:00:00Z".into(),
        ..Comment::default()
    }];
    let page: Value = serde_json::from_str(&render_jsonld(
        "P",
        &items,
        "https://x.test/d",
        &Configuration::default(),
    ))
    .unwrap();
    let selector = &page["items"][0]["target"]["selector"];
    // The spec's own way of pointing at part of an image.
    assert_eq!(selector["type"], "FragmentSelector");
    assert_eq!(selector["conformsTo"], "http://www.w3.org/TR/media-frags/");
    assert_eq!(selector["value"], "xywh=percent:10,20,30,25");
    // Which image has no vocabulary in the spec, so it goes under our prefix.
    assert_eq!(selector["komodoc:image_digest"], "abc123");
    assert_eq!(selector["komodoc:image_index"], 2);
}

/// Builds the shape /api/list returns, so a test can name documents by the
/// only field short_ids reads.
fn listing(slugs: &[&str]) -> Vec<Value> {
    slugs
        .iter()
        .map(|slug| serde_json::json!({"slug": slug}))
        .collect()
}

// A handle that is one character today collides with the next document
// published, and reads as a typo besides, so every handle is at least three
// characters wide.
#[test]
fn short_ids_are_at_least_three_characters() {
    let ids = short_ids(
        &listing(&["paper-abcdefghij", "notes-zyxwvutsrq"]),
        &Configuration::default(),
    );
    for (slug, id) in &ids {
        assert_eq!(id.len(), 3, "{slug} got the handle {id:?}");
    }
    assert_eq!(ids["paper-abcdefghij"], "abc");
    assert_eq!(ids["notes-zyxwvutsrq"], "zyx");
}

// Ragged handles are hard to read down a column, so documents that need a
// longer prefix widen every handle, not just their own.
#[test]
fn short_ids_share_one_width() {
    let ids = short_ids(
        &listing(&["a-abcdefghij", "b-abczefghij", "c-zyxwvutsrq"]),
        &Configuration::default(),
    );
    for (slug, id) in &ids {
        assert_eq!(id.len(), 4, "{slug} got the handle {id:?}");
    }
    assert_ne!(ids["a-abcdefghij"], ids["b-abczefghij"]);
}

// An explicit slug shorter than the common width is its own handle: there is
// nothing left to cut, and padding it would invent characters that do not
// address anything.
#[test]
fn short_ids_keep_short_slugs_whole() {
    let ids = short_ids(
        &listing(&["cv", "paper-abcdefghij"]),
        &Configuration::default(),
    );
    assert_eq!(ids["cv"], "cv");
}
