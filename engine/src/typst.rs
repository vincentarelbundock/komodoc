//! Typst, compiled to the page a document is stored as.
//!
//! The editor anchors comments into rendered text, so PDF and SVG are both
//! useless to it: neither has text nodes to walk, and a comment has nothing to
//! attach to. Typst's HTML export does have them, which is what makes a typst
//! document annotable the way a markdown one is.
//!
//! Everything the compiler may read is handed to it here. The main source is
//! the document being edited; anything it imports is asked of a `Files`
//! reader the caller supplies -- the folder beside the document on the command
//! line, nothing at all in the browser -- and packages are not resolved. There
//! is no network and no clock the document did not get from its host, so a
//! document cannot reach anything it was not given. The sandboxing that a
//! subprocess would need arranging -- a scratch directory, a root, a timeout --
//! is a property of this design rather than a thing to remember.

use std::path::Path;
use std::sync::OnceLock;

use typst::diag::{FileError, FileResult};
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Feature, Features, Library, LibraryExt, World};

use crate::page;

/// The compiler this crate is built against, for the version a publisher is
/// told when their own typst differs.
pub const VERSION: &str = "0.15";

/// Reads a file a document imports, by its path relative to the document's
/// root. `None` is "not found", which is what an import of something outside
/// the root, or of anything at all in the browser, gets.
pub type Files<'a> = &'a (dyn Fn(&Path) -> Option<Vec<u8>> + Sync);

/// The fonts every compile uses: the set typst itself ships, embedded, so a
/// document sets in the browser exactly as it does under the typst binary --
/// including maths, which needs a maths font and fails outright without one.
/// Parsed once, because parsing them on every keystroke would dwarf the
/// compile itself.
struct Fonts {
    book: LazyHash<FontBook>,
    faces: Vec<Font>,
}

fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        let mut book = FontBook::new();
        let mut faces = Vec::new();
        for data in typst_assets::fonts() {
            for font in Font::iter(Bytes::new(data.to_vec())) {
                book.push(font.info().clone());
                faces.push(font);
            }
        }
        Fonts {
            book: LazyHash::new(book),
            faces,
        }
    })
}

/// The library, built once. Typst's HTML export sits behind a feature flag,
/// exactly as it does in the command-line compiler.
fn library() -> &'static LazyHash<Library> {
    static LIBRARY: OnceLock<LazyHash<Library>> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        LazyHash::new(
            Library::builder()
                .with_features(Features::from_iter([Feature::Html]))
                .build(),
        )
    })
}

/// One document, the files it may import, and the date.
struct DocumentWorld<'a> {
    main: Source,
    files: Files<'a>,
    today: Option<Datetime>,
}

impl DocumentWorld<'_> {
    fn read(&self, id: FileId) -> FileResult<Vec<u8>> {
        let path = Path::new(id.vpath().get_without_slash());
        if !matches!(id.root(), VirtualRoot::Project) {
            return Err(FileError::NotFound(path.into()));
        }
        (self.files)(path).ok_or_else(|| FileError::NotFound(path.into()))
    }
}

impl World for DocumentWorld<'_> {
    fn library(&self) -> &LazyHash<Library> {
        library()
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &fonts().book
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            return Ok(self.main.clone());
        }
        let bytes = self.read(id)?;
        let text = String::from_utf8(bytes).map_err(|_| FileError::InvalidUtf8)?;
        Ok(Source::new(id, text))
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.read(id).map(Bytes::new)
    }

    fn font(&self, index: usize) -> Option<Font> {
        fonts().faces.get(index).cloned()
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        self.today
    }
}

/// The document being compiled, named for its imports' sake: a sibling file
/// resolves relative to it, at the root of the world.
fn main_id(name: &str) -> FileId {
    let name = if name.is_empty() { "main.typ" } else { name };
    RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new(name).expect("a document name is a valid path"),
    )
    .intern()
}

/// A world with nothing to read but the document: what the browser compiles
/// in, and the right default for a single-file document anywhere.
pub fn no_files(_: &Path) -> Option<Vec<u8>> {
    None
}

/// A calendar date, for `datetime.today()`; the host supplies it, since the
/// engine has no clock of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Today {
    pub year: i32,
    pub month: u8,
    pub day: u8,
}

impl Today {
    /// Today's date in UTC, from the system clock. Civil-from-days, so it
    /// needs no calendar crate.
    pub fn now() -> Option<Today> {
        let seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs() as i64;
        let days = seconds.div_euclid(86_400);
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
        let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
        Some(Today {
            year: (if m <= 2 { y + 1 } else { y }) as i32,
            month: m,
            day: d,
        })
    }
}

/// Compiles a source to the HTML typst exports: a whole page of its own, with
/// the styling its maths needs. A document that does not compile is an
/// ordinary state of an editor rather than an exceptional one, so a diagnostic
/// comes back as an ordinary result and the caller decides how to show it.
pub fn compile_html(
    source: &str,
    name: &str,
    files: Files,
    today: Option<Today>,
) -> Result<String, String> {
    let world = DocumentWorld {
        main: Source::new(main_id(name), source.to_string()),
        files,
        today: today.and_then(|t| Datetime::from_ymd(t.year, t.month, t.day)),
    };

    let first_of = |errors: ecow::EcoVec<typst::diag::SourceDiagnostic>| {
        errors
            .first()
            .map(|diagnostic| diagnostic.message.to_string())
            .unwrap_or_else(|| "typst could not compile this".to_string())
    };

    let document = typst::compile::<typst_html::HtmlDocument>(&world)
        .output
        .map_err(first_of)?;
    typst_html::html(&document, &typst_html::HtmlOptions { pretty: false }).map_err(first_of)
}

/// Compiles a source to the page every Komodoc document is stored as, so a
/// typst document and a markdown one look like the same application rather
/// than two. What typst brings of its own -- the styling its maths is laid out
/// by -- is kept, and placed after the shared stylesheet so it can override.
pub fn render(
    source: &str,
    title: &str,
    name: &str,
    files: Files,
    today: Option<Today>,
) -> Result<String, String> {
    let rendered = compile_html(source, name, files, today)?;
    Ok(wrap(&rendered, title))
}

/// Puts typst's output in the shared page: its `<style>` blocks into the head,
/// what it wrote between the body tags as the body.
pub fn wrap(rendered: &str, title: &str) -> String {
    let mut head = Vec::new();
    let mut rest = rendered;
    while let Some(start) = rest.find("<style>") {
        let after = &rest[start..];
        match after.find("</style>") {
            Some(end) => {
                head.push(&after[..end + "</style>".len()]);
                rest = &after[end + "</style>".len()..];
            }
            None => break,
        }
    }
    let body = body_of(rendered).unwrap_or(rendered);
    page::page(title, &head.join("\n"), body)
}

/// What sits between `<body ...>` and the last `</body>`, if the output is a
/// whole page.
fn body_of(rendered: &str) -> Option<&str> {
    let start = rendered.find("<body")?;
    let open = start + rendered[start..].find('>')? + 1;
    let close = rendered.rfind("</body>")?;
    (close >= open).then(|| &rendered[open..close])
}

/// Says whether a filename is one this renders.
pub fn is_typst(name: &str) -> bool {
    name.to_lowercase().ends_with(".typ")
}

/// The document's first level-one heading, which is the obvious title when
/// none was given: typst has no metadata this side can see without compiling.
pub fn title_of(source: &str) -> String {
    page::first_heading(source, '=')
}

#[cfg(test)]
mod tests {
    use super::*;

    // Prose with no maths in it still needs a font to be set in, which is why
    // a text face is embedded alongside the maths one.
    #[test]
    fn prose_needs_no_maths() {
        let html = compile_html(
            "= Notes\n\nJust prose, no maths at all.\n",
            "",
            &no_files,
            None,
        )
        .expect("typst could not compile prose");
        assert!(
            html.contains("Just prose"),
            "the prose is not in the output"
        );
    }

    // The editor anchors comments into text nodes, so what matters about the
    // output is not that it exists but that it is text: headings as headings,
    // emphasis as elements, maths as MathML rather than as pictures of maths.
    #[test]
    fn exports_html_a_comment_can_be_anchored_into() {
        let html = compile_html(
            "= A Paper\n\nProse with *emphasis* and a passage worth annotating.\n\n\
             == Method\n\n$ sum_(k=1)^n k = (n(n+1))/2 $\n\nWe measured the thing.\n",
            "",
            &no_files,
            None,
        )
        .expect("typst could not compile the sample");
        for wanted in [
            "<h2>A Paper</h2>",
            "<strong>emphasis</strong>",
            "worth annotating",
            "We measured the thing.",
            "<math",
        ] {
            assert!(html.contains(wanted), "the output is missing {wanted:?}");
        }
    }

    // A typst document and a markdown one are the same application, so they
    // are dressed in the same stylesheet -- and typst keeps the styling its
    // maths needs, which nothing else provides.
    #[test]
    fn wears_the_shared_page_and_keeps_its_own_styling() {
        let page = render("= T\n\n$ x^2 $\n", "T", "", &no_files, None).expect("compile");
        assert!(page.starts_with("<!doctype html>"));
        assert_eq!(
            page.matches("<html").count(),
            1,
            "more than one document in the page"
        );
        assert!(
            page.contains("max-width: 46rem"),
            "not wearing the shared stylesheet"
        );
        assert!(
            page.contains("<math"),
            "the maths did not survive the wrapping"
        );
        let shared = page.find("max-width: 46rem").unwrap();
        let own = page
            .find("mtable")
            .expect("typst's own styling was dropped");
        assert!(
            shared < own,
            "typst's styling comes before the shared sheet, so it cannot override it"
        );
    }

    // A document may import what sits beside it, through the reader the host
    // supplies, and nothing the reader does not know.
    #[test]
    fn imports_resolve_through_the_reader() {
        let files = |path: &Path| -> Option<Vec<u8>> {
            (path == Path::new("lib.typ")).then(|| b"#let greeting = \"hello from lib\"".to_vec())
        };
        let html = compile_html(
            "#import \"lib.typ\": greeting\n#greeting\n",
            "main.typ",
            &files,
            None,
        )
        .expect("the import did not resolve");
        assert!(html.contains("hello from lib"));
        assert!(compile_html("#import \"missing.typ\": x\n", "main.typ", &files, None).is_err());
    }

    #[test]
    fn today_is_what_the_host_says() {
        let today = Today {
            year: 2026,
            month: 9,
            day: 4,
        };
        let html = compile_html("#datetime.today().display()\n", "", &no_files, Some(today))
            .expect("compile");
        assert!(html.contains("2026-09-04"), "{html}");
        assert!(compile_html("#datetime.today().display()\n", "", &no_files, None).is_err());
        assert!(Today::now().is_some());
    }

    #[test]
    fn titles_and_names() {
        assert_eq!(title_of("== Sub\n= The Title\n"), "The Title");
        assert!(is_typst("paper.TYP"));
        assert!(!is_typst("paper.md"));
    }
}
