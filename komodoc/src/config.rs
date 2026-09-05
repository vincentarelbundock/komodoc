//! Every rule the deployment enforces lives here, and only here. The shell
//! gets these values injected into its source in place of `__CONFIG__`; the
//! server reads them directly. A limit changed here changes everywhere on the
//! next build.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct Configuration {
    pub max_html: usize,
    pub max_comments: usize,
    pub rate_per_hour: i64,
    pub caps: CapLimit,

    /// Storage is what keeps a deployment's bill bounded no matter who shows
    /// up: a ceiling on everything stored, a ceiling per publisher, and a cap
    /// on how many documents and how many uploads an hour one publisher gets.
    /// Sizes are bytes of stored HTML; every index entry records its own.
    pub storage: StorageLimit,

    /// Caps the serialized seed annotations a reserved example carries, in
    /// bytes.
    pub max_annotations: usize,

    /// The only file types the reader can frame and anchor comments into. The
    /// upload page checks them before sending, and the server checks them
    /// again.
    pub extensions: Vec<String>,

    /// The markups a document may be kept as, beside the HTML it was rendered
    /// to, so it can be reopened and edited. A source in anything else is
    /// dropped rather than refused: the document is fine, it simply cannot be
    /// reopened.
    pub source_formats: Vec<String>,

    /// The W3C Web Annotation motivations an annotation may carry. Using the
    /// standard vocabulary rather than an invented one means an exported
    /// annotation says the same thing to any tool that reads the spec.
    ///
    /// Two of them: a remark about a passage, and a passage marked as worth
    /// returning to. The shades of saying something -- a question, a
    /// judgement, a proposed rewording -- are the comment's own words, not a
    /// taxonomy to pick from before writing one.
    pub motivations: Vec<String>,
    pub default_motivation: String,

    /// How many labels one annotation may carry. Tags are what make a long
    /// review navigable, but a dozen on one comment is a filing system, not a
    /// label.
    pub max_tags: usize,

    /// Caps a document title, in characters. Titles live in the index, which is
    /// read on nearly every request, so an unbounded title is a way to sink the
    /// whole deployment.
    pub max_title: usize,
    /// Caps replies on one comment, so a thread cannot grow without bound and a
    /// room stays small enough to load and rewrite whole.
    pub max_replies: usize,

    /// The shape of a valid slug, as a RegExp source string.
    pub slug_pattern: String,
    pub slug_max: usize,

    /// Documents are unlisted, so the URL is the only way in and the slug has
    /// to be unguessable. 10 characters from a 32-symbol alphabet is 50 bits,
    /// drawn from a CSPRNG; look-alike characters are left out so a link
    /// survives being read aloud or retyped.
    pub suffix_alphabet: String,
    pub suffix_length: usize,
}

/// Bounds what a deployment will hold. `total` and `per_owner` are bytes;
/// `documents_per_owner` and `uploads_per_hour` are counts.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct StorageLimit {
    pub total: i64,
    pub per_owner: i64,
    pub documents_per_owner: usize,
    pub uploads_per_hour: usize,
}

/// The maximum length of each free-text field on an annotation.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct CapLimit {
    pub body: usize,
    pub creator: usize,
    pub exact: usize,
    pub context: usize,
    pub tag: usize,
}

impl Default for Configuration {
    fn default() -> Self {
        Configuration {
            max_html: 4 * 1024 * 1024,
            max_comments: 500,
            rate_per_hour: 20,
            storage: StorageLimit {
                total: 5 * 1024 * 1024 * 1024,
                per_owner: 100 * 1024 * 1024,
                documents_per_owner: 50,
                uploads_per_hour: 30,
            },
            max_annotations: 256 * 1024,
            extensions: [".html", ".htm", ".md", ".markdown"]
                .map(String::from)
                .to_vec(),
            source_formats: ["markdown", "typst"].map(String::from).to_vec(),
            caps: CapLimit {
                body: 5000,
                creator: 80,
                exact: 1000,
                context: 64,
                tag: 24,
            },
            motivations: ["commenting", "highlighting"].map(String::from).to_vec(),
            default_motivation: "commenting".to_string(),
            max_tags: 6,
            max_title: 200,
            max_replies: 100,
            slug_pattern: r"^[a-z0-9]+(?:-[a-z0-9]+)*$".to_string(),
            slug_max: 80,
            suffix_alphabet: "abcdefghijkmnpqrstuvwxyz23456789".to_string(),
            suffix_length: 10,
        }
    }
}

impl Configuration {
    /// Keeps an unknown motivation out of storage, falling back to the default
    /// rather than rejecting the annotation.
    pub fn allowed_motivation(&self, value: &str) -> String {
        if self.motivations.iter().any(|known| known == value) {
            value.to_string()
        } else {
            self.default_motivation.clone()
        }
    }

    /// Says whether a source in this format is worth keeping beside the
    /// document it rendered to. Whether it can be rendered again *here* is a
    /// separate question, answered by `renderers`.
    pub fn storable_source(&self, format: &str) -> bool {
        self.source_formats.iter().any(|known| known == format)
    }

    /// Overrides the document size ceiling, in megabytes. Zero leaves the
    /// default alone.
    pub fn set_max_html(&mut self, megabytes: usize) -> Result<(), String> {
        if megabytes == 0 {
            return Ok(());
        }
        if !(1..=100).contains(&megabytes) {
            return Err("--max-size must be between 1 and 100 MB".into());
        }
        self.max_html = megabytes * 1024 * 1024;
        Ok(())
    }

    /// Overrides the storage ceilings, in megabytes: how much one publisher may
    /// hold across all their documents, and how much the whole deployment will
    /// hold. Zero leaves a default alone.
    pub fn set_storage(&mut self, quota_mb: i64, total_mb: i64) -> Result<(), String> {
        if quota_mb < 0 || total_mb < 0 {
            return Err("--quota and --storage must be positive".into());
        }
        if quota_mb > 0 {
            self.storage.per_owner = quota_mb * 1024 * 1024;
        }
        if total_mb > 0 {
            self.storage.total = total_mb * 1024 * 1024;
        }
        if self.storage.per_owner > self.storage.total {
            return Err(format!(
                "--quota ({} MB) cannot exceed --storage ({} MB)",
                self.storage.per_owner >> 20,
                self.storage.total >> 20
            ));
        }
        Ok(())
    }

    /// Overrides the per-publisher counts: how many documents one publisher may
    /// hold, and how many uploads they may make in an hour. Zero leaves a
    /// default alone.
    pub fn set_counts(&mut self, documents: i64, uploads_per_hour: i64) -> Result<(), String> {
        if documents < 0 || uploads_per_hour < 0 {
            return Err("--max-documents and --uploads-per-hour must be positive".into());
        }
        if documents > 0 {
            self.storage.documents_per_owner = documents as usize;
        }
        if uploads_per_hour > 0 {
            self.storage.uploads_per_hour = uploads_per_hour as usize;
        }
        Ok(())
    }
}
