//! The reader shell, compiled in: real .html, .css and .js files under
//! src/shell, served by the server with the shared limits from config.rs
//! substituted where the pages need them.

use std::collections::HashMap;

use include_dir::{include_dir, Dir};
use sha2::{Digest, Sha256};

use crate::config::Configuration;

static SHELL: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../src/shell");

/// Maps a request path to the file that answers it.
const SHELL_ROUTES: &[(&str, &str)] = &[
    // Pico CSS, vendored under vendor/ so the shell keeps its no-build,
    // no-CDN property. See vendor/pico-LICENSE.md.
    ("/pico.css", "vendor/pico.css"),
    ("/index.html", "index.html"),
    ("/reader.html", "reader.html"),
    ("/404.html", "404.html"),
    ("/komodoc.css", "komodoc.css"),
    ("/reader.js", "reader.js"),
    ("/editor.js", "editor.js"),
    ("/collab.js", "collab.js"),
    ("/sync.js", "sync.js"),
    // Yjs, vendored whole under vendor/ for the same reason. See
    // vendor/yjs-LICENSE.txt.
    ("/vendor/yjs.js", "vendor/yjs.js"),
    ("/agent.js", "agent.js"),
    ("/anchor.js", "anchor.js"),
    ("/documentation", "documentation.html"),
    ("/assets/komodo-logo.svg", "assets/komodo-logo.svg"),
    ("/docs/commenting.png", "assets/commenting.png"),
    ("/docs/sandbox.png", "assets/sandbox.png"),
];

/// The renderers, which are build outputs rather than sources and are
/// addressed by a digest of their own bytes: a module is fetched once per
/// browser and cached for a year, so a route that outlived a rebuild would
/// hand a stale module to a loader that no longer speaks to it. The editor
/// learns the current URLs by substitution, the way the pages learn the
/// limits.
const MODULES: &[(&str, &str, &str)] = &[
    ("markdown", "wasm/markdown.wasm", "__MARKDOWN_WASM__"),
    // Optional: `make typst` builds it, and a build without it simply does not
    // list typst among its renderers.
    ("typst", TYPST_FILE, "__TYPST_WASM__"),
];

/// The typst compiler is thirty megabytes of WebAssembly -- typst itself, and
/// the fonts it sets documents in -- so it is built separately and a
/// deployment may not have it.
const TYPST_FILE: &str = "wasm/typst.wasm";

/// Where one renderer is served from: its name under /wasm/, stamped with a
/// digest of the bytes, so the URL changes whenever the module does.
fn module_route(name: &str, body: &[u8]) -> String {
    let digest = hex::encode(Sha256::digest(body));
    format!("/wasm/{name}.{}.wasm", &digest[..16])
}

/// Served on its own, rather than inlined into the stylesheet, because a font
/// never changes and a stylesheet does.
pub const FONT_ROUTE: &str = "/fonts/ibm-plex-sans-600.woff2";
const FONT_FILE: &str = "vendor/fonts/IBMPlexSans-SemiBold.woff2";

pub fn content_type(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("wasm") => "application/wasm",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[derive(Clone, Debug)]
pub struct ShellFile {
    pub kind: &'static str,
    pub body: Vec<u8>,
    /// A file whose bytes never change, so it can be cached for a year
    /// instead of five minutes.
    pub immutable: bool,
}

impl ShellFile {
    /// The file as text, for the pages that are text and the tests that read
    /// them.
    #[allow(dead_code)]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

fn file(name: &str) -> Option<&'static [u8]> {
    SHELL.get_file(name).map(|f| f.contents())
}

/// The project README rendered into the HTML the documentation page is built
/// around. The README is the only copy of that text: it is embedded from the
/// shell directory, where the build puts it, so the page cannot drift from
/// what the repository says.
fn documentation() -> Result<String, String> {
    let source = file("README.md")
        .ok_or("missing README.md in the shell: run make build, which copies it in")?;
    let body = komodoc_engine::markdown::render_body(&String::from_utf8_lossy(source));
    // The page opens with the mark and the wordmark, as the landing page does,
    // so the README's own title would say the name twice.
    let leading = regex::Regex::new(r"(?s)^\s*<h1[^>]*>.*?</h1>").expect("a constant pattern");
    Ok(leading.replace(&body, "").to_string())
}

/// Every file the server answers with, ready to serve.
pub fn load_shell(config: &Configuration) -> Result<HashMap<String, ShellFile>, String> {
    let settings = serde_json::to_string(config)
        .map_err(|err| format!("could not encode the configuration: {err}"))?;
    let prose = documentation()?;
    let mut shell = HashMap::new();

    // The renderers first, because the pages have to be told where they are.
    let mut modules: HashMap<&str, String> = HashMap::new();
    for (name, path, placeholder) in MODULES {
        let Some(body) = file(path) else { continue };
        let route = module_route(name, body);
        modules.insert(placeholder, route.clone());
        shell.insert(
            route,
            ShellFile {
                kind: "application/wasm",
                body: body.to_vec(),
                immutable: true,
            },
        );
    }

    for (route, name) in SHELL_ROUTES {
        let body = file(name).ok_or_else(|| format!("missing shell file: {name}"))?;
        let kind = content_type(name);
        // Binary, and unchanging: cached for a year because the bytes are the
        // same until the next build replaces them.
        if name.ends_with(".png") {
            shell.insert(
                route.to_string(),
                ShellFile {
                    kind,
                    body: body.to_vec(),
                    immutable: true,
                },
            );
            continue;
        }
        // The page checks the same size and file-type limits the server does,
        // so __CONFIG__ is substituted here too rather than duplicated in
        // HTML. The README goes in after the configuration, so nothing in it
        // is mistaken for a placeholder the shell was meant to fill in.
        let mut source = String::from_utf8_lossy(body)
            .replace("__CONFIG__", &settings)
            .replace("__README__", &prose);
        // And where this build serves its renderers, which is a digest of
        // their bytes rather than a fixed path: a module is cached for a year,
        // so a route that outlived a rebuild would hand a stale module to a
        // loader that no longer speaks to it.
        for (_, _, placeholder) in MODULES {
            source = source.replace(
                placeholder,
                modules.get(placeholder).map_or("", String::as_str),
            );
        }
        shell.insert(
            route.to_string(),
            ShellFile {
                kind,
                body: source.into_bytes(),
                immutable: false,
            },
        );
    }
    // What a rendered document looks like, served so the browser's renderers
    // wrap their output in exactly the page the command line wraps it in.
    shell.insert(
        "/document.css".to_string(),
        ShellFile {
            kind: content_type("x.css"),
            body: komodoc_engine::page::DOCUMENT_CSS.as_bytes().to_vec(),
            immutable: false,
        },
    );
    let font = file(FONT_FILE).ok_or("missing the wordmark font")?;
    shell.insert(
        FONT_ROUTE.to_string(),
        ShellFile {
            kind: "font/woff2",
            body: font.to_vec(),
            immutable: true,
        },
    );
    Ok(shell)
}

/// The compiled typst renderer, if this build has one.
pub fn typst_module() -> Option<&'static [u8]> {
    file(TYPST_FILE)
}

/// The source formats this process can render in a reader, and so offer an
/// editor for. Markdown always; typst when the module was built.
pub fn renderers() -> Vec<String> {
    let mut list = vec!["markdown".to_string()];
    if typst_module().is_some() {
        list.push("typst".to_string());
    }
    list
}

/// The number of routed files, for the test that counts what the shell holds.
#[allow(dead_code)]
pub fn route_count() -> usize {
    SHELL_ROUTES.len()
}

/// Where a renderer is served from in this build, for the tests that fetch
/// one.
#[allow(dead_code)]
pub fn module_url(name: &str) -> Option<String> {
    let path = MODULES.iter().find(|(known, ..)| *known == name)?.1;
    Some(module_route(name, file(path)?))
}
