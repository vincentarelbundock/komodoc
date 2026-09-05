//! Rendering on this side of the network: what `publish` and `seed` do to a
//! markdown or typst file before it is stored. The browser renders through the
//! same engine crate, so a document published here and one edited there are
//! rendered by the same code.

use std::path::{Path, PathBuf};

use komodoc_engine::{markdown, typst};

pub fn is_markdown(name: &str) -> bool {
    markdown::is_markdown(name)
}

pub fn is_typst(name: &str) -> bool {
    typst::is_typst(name)
}

pub fn title_from_markdown(source: &str) -> String {
    markdown::title_of(source)
}

pub fn title_from_typst(source: &str) -> String {
    typst::title_of(source)
}

pub fn render_markdown_document(source: &str, title: &str) -> String {
    markdown::render(source, title)
}

/// Compiles a typst source to the page a document is stored as. The file's
/// own directory is the root: a document may import what sits beside it and
/// nothing above it, which a reader that refuses to leave the root is what
/// enforces.
pub fn render_typst_document(file: &Path, source: &str, title: &str) -> Result<String, String> {
    let root = std::path::absolute(file.parent().unwrap_or(Path::new(".")))
        .map_err(|err| err.to_string())?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let reader = move |path: &Path| -> Option<Vec<u8>> { read_within(&root, path) };
    typst::render(source, title, &name, &reader, typst::Today::now())
        .map_err(|message| format!("typst could not compile {name}:\n\n{message}"))
}

/// Reads a file under `root`, and nothing outside it. The path typst asks for
/// is already normalised -- no `..` survives its own resolution -- but the
/// containment is checked rather than trusted, the same as every key is.
fn read_within(root: &Path, path: &Path) -> Option<Vec<u8>> {
    let mut resolved = PathBuf::from(root);
    for part in path.components() {
        match part {
            std::path::Component::Normal(name) => resolved.push(name),
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            _ => {}
        }
    }
    if !resolved.starts_with(root) {
        return None;
    }
    std::fs::read(resolved).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_typst_document_may_import_a_sibling_and_nothing_above() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("lib.typ"), "#let word = \"sibling\"").unwrap();
        std::fs::write(
            dir.path().join("main.typ"),
            "#import \"lib.typ\": word\n= T\n#word\n",
        )
        .unwrap();
        let above = dir
            .path()
            .parent()
            .unwrap()
            .join("komodoc-above-the-root.typ");
        std::fs::write(&above, "#let word = \"escaped\"").unwrap();

        let main = dir.path().join("main.typ");
        let page = render_typst_document(&main, &std::fs::read_to_string(&main).unwrap(), "T")
            .expect("compile");
        assert!(page.contains("sibling"));

        let escaping = "#import \"../komodoc-above-the-root.typ\": word\n#word\n";
        assert!(render_typst_document(&main, escaping, "T").is_err());
        let _ = std::fs::remove_file(above);
    }
}
