//! What the shell is made of, checked before a browser has to find out.

use crate::assets::{load_shell, module_url, renderers, typst_module};
use crate::config::Configuration;

fn shell() -> std::collections::HashMap<String, crate::assets::ShellFile> {
    load_shell(&Configuration::default()).expect("the shell loads")
}

// Every page the server can answer with is in the build. A missing one is not
// a missing feature but a blank window, and the build is a bundler's output
// rather than a list kept by hand, so this is where a page that stopped being
// built is noticed.
#[test]
fn every_page_is_built() {
    let shell = shell();
    for page in [
        "/index.html",
        "/reader.html",
        "/documentation.html",
        "/404.html",
        "/agent.js",
    ] {
        assert!(
            shell.contains_key(page),
            "{page} is not in the build; run `make web`"
        );
    }
}

// Each page loads its own bundle, and every asset a page names has to be
// there: the bundler decides those names, so a page pointing at a file that
// was not written is a page that does nothing at all.
#[test]
fn every_asset_a_page_names_is_served() {
    let shell = shell();
    let reference =
        regex::Regex::new(r#"(?:src|href)="(/assets/[^"]+)""#).expect("a constant pattern");
    let mut checked = 0;
    for (route, asset) in &shell {
        if !route.ends_with(".html") {
            continue;
        }
        for found in reference.captures_iter(&asset.text()) {
            checked += 1;
            let wanted = found[1].to_string();
            assert!(
                shell.contains_key(&wanted),
                "{route} names {wanted}, which nothing serves"
            );
        }
    }
    assert!(checked > 0, "no page named a bundle; the build did not run");
}

// The bundler names an asset for a digest of its own contents, so one can be
// kept for a year; a page is rewritten in place by the next build and cannot
// be, or a browser would hold yesterday's page against today's bundles.
#[test]
fn bundles_are_immutable_and_pages_are_not() {
    let shell = shell();
    for (route, asset) in &shell {
        if route.starts_with("/assets/") || route.starts_with("/fonts/") {
            assert!(
                asset.immutable,
                "{route} is named for its contents but is not cached as such"
            );
        }
        if route.ends_with(".html") {
            assert!(
                !asset.immutable,
                "{route} is rewritten by every build and must not be cached for a year"
            );
        }
    }
}

// The wordmark's font is served from this deployment, never from a font host,
// so a page still reaches out to nobody.
#[test]
fn the_wordmark_font_is_served_from_here() {
    let shell = shell();
    let css = shell
        .iter()
        .find(|(route, _)| route.starts_with("/assets/") && route.ends_with(".css"))
        .map(|(_, asset)| asset.text())
        .expect("the stylesheet is in the build");
    assert!(
        css.contains("/fonts/ibm-plex-sans-600.woff2"),
        "the stylesheet does not point at the font route"
    );
    assert!(
        !css.contains("fonts.googleapis") && !css.contains("fonts.gstatic"),
        "an external font host"
    );

    let font = shell
        .get("/fonts/ibm-plex-sans-600.woff2")
        .expect("the font is served");
    assert_eq!(font.kind, "font/woff2");
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
fn the_documentation_page_carries_the_readme() {
    let shell = shell();
    let page = shell["/documentation.html"].text();
    assert!(
        !page.contains("__README__"),
        "the README placeholder was not filled in"
    );
    assert!(
        page.contains("<h2"),
        "the rendered README has no headings to build a contents list from"
    );
}

// The renderer the editor previews with is served as WebAssembly at a URL that
// carries a digest of the module, so a browser holding the previous build's
// copy cannot be handed it: the address changed with the bytes.
#[test]
fn the_renderers_are_served_as_wasm() {
    let shell = shell();
    let url = module_url("markdown").expect("this build has no markdown module");
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

// The reader is told where the renderers are when it is served, so the editor
// never has to ask and can never ask for a path this build does not serve.
#[test]
fn the_reader_is_told_where_the_renderers_are() {
    let shell = shell();
    let page = shell["/reader.html"].text();
    assert!(
        !page.contains("__MODULES__"),
        "the module URLs were not substituted into the reader"
    );
    for name in ["markdown", "typst"] {
        let Some(url) = module_url(name) else {
            continue;
        };
        assert!(page.contains(&url), "the reader does not point at {url}");
        assert!(shell.contains_key(&url), "{url} is named but not served");
    }
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
