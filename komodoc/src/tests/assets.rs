//! What the shell is made of, checked before a browser has to find out.

use crate::assets::{load_shell, module_url, renderers, route_count, typst_module, FONT_ROUTE};
use crate::config::Configuration;

fn shell() -> std::collections::HashMap<String, crate::assets::ShellFile> {
    load_shell(&Configuration::default()).expect("the shell loads")
}

#[test]
fn the_reader_offers_sign_in() {
    let shell = shell();
    assert!(
        shell["/reader.html"].text().contains("id=\"signIn\""),
        "reader navigation has no sign-in link"
    );
    assert!(
        shell["/reader.js"].text().contains("me.can_sign_in"),
        "reader does not use the deployment's sign-in availability"
    );
}

#[test]
fn the_wordmark_font_is_served_and_cached() {
    let shell = shell();
    let css = shell["/komodoc.css"].text();
    assert!(
        css.contains(&format!("url(\"{FONT_ROUTE}\")")),
        "the stylesheet does not point at the font route"
    );
    // Never a font host: a page reaches out to nobody.
    assert!(
        !css.contains("fonts.googleapis") && !css.contains("fonts.gstatic"),
        "the stylesheet references an external font host"
    );

    let font = shell.get(FONT_ROUTE).expect("the font is served");
    assert_eq!(font.kind, "font/woff2");
    // A font never changes, so it is cached for a year.
    assert!(font.immutable, "the font should be immutable");
    assert!(
        font.body.starts_with(b"wOF2"),
        "what is served is not a woff2 file"
    );
    // Complete and unmodified, so the reserved family name stays honest.
    assert!(
        font.body.len() > 40_000,
        "the font is {} bytes; a subset would raise an OFL naming question",
        font.body.len()
    );
}

#[test]
fn the_documentation_screenshot_is_served() {
    let shell = shell();
    let image = shell
        .get("/docs/commenting.png")
        .expect("documentation screenshot has no shell route");
    assert_eq!(image.kind, "image/png");
    assert!(
        image.body.starts_with(b"\x89PNG\r\n\x1a\n"),
        "documentation screenshot is not a PNG"
    );
}

// Every module the shell imports has to be served, and a missing one is not a
// missing feature but a blank page: the browser stops at the failed import and
// nothing after it runs. Static imports are checked here rather than in a
// browser.
#[test]
fn every_imported_module_is_served() {
    let shell = shell();
    let imports = regex::Regex::new(r#"(?m)^import[^"']*["']\.(/[^"']+)["']"#).unwrap();
    for (route, asset) in &shell {
        if !route.ends_with(".js") {
            continue;
        }
        for found in imports.captures_iter(&asset.text()) {
            // Written relative to the shell root, which is how they are served.
            let wanted = found[1].to_string();
            assert!(
                shell.contains_key(&wanted),
                "{route} imports {wanted}, which nothing serves"
            );
        }
    }
}

// The renderer the editor previews with is served as WebAssembly and cached
// for a year, since its bytes only change when the build does.
#[test]
fn the_renderers_are_served_as_wasm() {
    let shell = shell();
    let url = module_url("markdown").expect("this build has no markdown module");
    // The URL carries a digest of the module, so a browser holding the
    // previous build's copy cannot be handed it: the address changed with the
    // bytes.
    assert!(
        url.starts_with("/wasm/markdown.") && url.ends_with(".wasm"),
        "the module URL is {url}"
    );
    let module = shell.get(&url).expect("the markdown module is not served");
    assert_eq!(module.kind, "application/wasm");
    assert!(module.immutable, "the module is not marked immutable");
    assert!(
        module.body.starts_with(b"\0asm"),
        "the module is not WebAssembly"
    );
    // Built from Rust, so it carries no language runtime: a few hundred
    // kilobytes rather than the megabytes a Go build needed.
    assert!(
        module.body.len() < 3 * 1024 * 1024,
        "the markdown module is {} bytes",
        module.body.len()
    );
}

// Typst is optional: `make typst` builds its renderer, and a build without one
// simply does not offer typst editing. So this describes both states rather
// than requiring the artifact, which needs a longer build to produce.
#[test]
fn typst_is_offered_only_when_it_is_built() {
    let list = renderers();
    assert_eq!(
        list.first().map(String::as_str),
        Some("markdown"),
        "renderers are {list:?}, want markdown first"
    );
    let Some(module) = typst_module() else {
        assert!(
            !list.contains(&"typst".to_string()),
            "typst is offered without a module"
        );
        eprintln!("no typst renderer in this build; run `make typst`");
        return;
    };
    assert!(
        list.contains(&"typst".to_string()),
        "the typst module is built but serve does not offer it"
    );
    assert!(
        module.starts_with(b"\0asm"),
        "the typst module is not WebAssembly"
    );
    let url = module_url("typst").expect("this build has a typst module");
    let served = shell()
        .get(&url)
        .cloned()
        .expect("the typst module is not served");
    assert!(served.kind == "application/wasm" && served.immutable);
}

// The routed files, plus the three that are added rather than read from the
// route table: the wordmark's font, the stylesheet a rendered document is
// dressed in, and the markdown renderer -- with the typst renderer when a
// build has one.
#[test]
fn the_shell_holds_what_the_routes_name() {
    let added = 3 + usize::from(typst_module().is_some());
    assert_eq!(shell().len(), route_count() + added);
}

// The editor is told where the renderers are, so a page never asks for a path
// this build does not serve -- which is what a fixed path would eventually do,
// since a module is cached for a year.
#[test]
fn the_editor_is_told_where_the_renderers_are() {
    let shell = shell();
    let editor = shell["/editor.js"].text();
    assert!(
        !editor.contains("__MARKDOWN_WASM__"),
        "the module URL was not substituted into the editor"
    );
    for name in ["markdown", "typst"] {
        let Some(url) = module_url(name) else {
            continue;
        };
        assert!(editor.contains(&url), "the editor does not point at {url}");
        assert!(shell.contains_key(&url), "{url} is named but not served");
    }
}

// Both the storable formats are the ones the engine can actually render, and
// nothing else is kept beside a document.
#[test]
fn storable_source_formats() {
    let config = Configuration::default();
    for format in ["markdown", "typst"] {
        assert!(config.storable_source(format), "{format} is not storable");
    }
    for format in ["", "latex", "html", "docx"] {
        assert!(
            !config.storable_source(format),
            "{format:?} is storable and should not be"
        );
    }
}
