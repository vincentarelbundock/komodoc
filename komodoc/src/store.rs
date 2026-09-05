//! The document store: the index of what exists, and the bytes of each
//! version.
//!
//! Where those bytes live is not its business -- see blob.rs. What is here is
//! the interesting half: who owns a document, what a deployment will hold, and
//! the compare-and-swap that keeps the index honest when two writes race.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::blob::{
    document_key, document_prefix, examples_key, room_key, room_lock_key, source_key, BlobError,
    BlobStore, BlobVersion, INDEX_KEY,
};
use crate::clock::{now_unix, parse_timestamp, timestamp};
use crate::config::Configuration;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IndexEntry {
    pub slug: String,
    pub title: String,
    pub sha: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub example: bool,
    /// The lowercased GitHub login that uploaded this version, and the only
    /// account that may replace or delete it. Empty on a reserved example, and
    /// on anything published before ownership was recorded or on a deployment
    /// where publishing needs no account at all.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub publisher: String,
    /// The GitHub account's numeric id, set alongside `publisher` on every new
    /// upload from a signed-in caller. A login can be renamed; the numeric id
    /// cannot, so `owned_by` prefers it when a document carries one. Never set
    /// for a visitor-owned or unowned document, since neither has a GitHub
    /// account behind it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub publisher_id: String,
    /// The bytes of the stored HTML -- plus the source beside it, when one is
    /// kept -- and what the storage quotas are measured against. An entry from
    /// before it was recorded reads back as zero, which `admit` treats as free
    /// rather than refusing every upload until each old document is replaced.
    #[serde(default)]
    pub size: i64,
    /// What the stored source is, when the document was published from one.
    /// Empty means there is no source to reopen.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_format: String,
}

impl IndexEntry {
    /// Whether a caller -- named by their owner key (see `Server::owner`) and,
    /// when signed in, their GitHub numeric id -- may replace or delete this
    /// document. An entry with no publisher belongs to no one in particular
    /// and stays shared. An entry carrying a publisher id compares against the
    /// id instead of the key, since the id survives a GitHub account being
    /// renamed and the key would not; a legacy entry, or one owned by a
    /// visitor: key, has no publisher id and falls back to comparing the key.
    pub fn owned_by(&self, owner_key: &str, caller_id: &str) -> bool {
        if self.publisher.is_empty() {
            true
        } else if !self.publisher_id.is_empty() {
            !caller_id.is_empty() && caller_id == self.publisher_id
        } else {
            self.publisher == owner_key.to_lowercase()
        }
    }

    /// When this document expires from, as seconds since the epoch.
    pub fn expiry_time(&self, from: &str) -> Option<i64> {
        parse_timestamp(if from == "created" {
            &self.created_at
        } else {
            &self.updated_at
        })
    }
}

pub struct Store {
    /// Where the bytes are: a directory, or somebody else's S3. The store does
    /// not care which.
    pub blobs: Arc<dyn BlobStore>,
    pub config: Arc<Configuration>,
    pub state: Mutex<StoreState>,
}

pub struct StoreState {
    pub entries: HashMap<String, IndexEntry>,
    /// The version of index.json these entries were read from, so a write can
    /// say what it expects to be replacing. Empty means "there was no index",
    /// which is how a fresh store starts.
    pub index_version: BlobVersion,
}

/// Reads index.json, with the version it was read at. No index yet is an
/// empty store, which is how a fresh one starts. An index that exists but
/// cannot be parsed is a different thing entirely, and an error rather than an
/// empty map: carrying on would present every stored document as gone, and
/// the next publish would overwrite the real index with a near-empty one.
pub async fn load_index(
    blobs: &dyn BlobStore,
) -> Result<(HashMap<String, IndexEntry>, BlobVersion), String> {
    match blobs.get_versioned(INDEX_KEY).await {
        Err(BlobError::NotFound) => Ok((HashMap::new(), String::new())),
        Err(err) => Err(format!(
            "could not read the index from {}: {err}",
            blobs.describe()
        )),
        Ok((raw, at)) => {
            let entries: HashMap<String, IndexEntry> =
                serde_json::from_slice(&raw).map_err(|err| {
                    format!(
                        "the index in {} is not readable ({err}); move it aside to start empty",
                        blobs.describe()
                    )
                })?;
            Ok((entries, at))
        }
    }
}

/// One version of a document: everything stored about it, and everything
/// needed to decide whether storing it is allowed.
#[derive(Clone, Debug, Default)]
pub struct Publication {
    pub slug: String,
    pub title: String,
    /// sha256 of html, and the name it is stored under.
    pub digest: String,
    pub html: String,
    /// The markup html was rendered from, kept so the document can be reopened
    /// in an editor; empty for a document published as HTML, which has no
    /// source but itself. Its bytes count against the quotas along with the
    /// HTML.
    pub source: String,
    pub source_format: String,
    pub owner: String,
    pub owner_id: String,
    /// The version this one was edited from. When it is set and the document
    /// has moved on since, the write is refused rather than applied: two
    /// people editing at once would otherwise mean whoever saved last silently
    /// discarded the other's work. Empty means "whatever is there".
    pub base_sha: String,
}

/// What `put` returns when a storage rule refuses an upload: the HTTP status
/// and message the rule names, so the handler answers with exactly what the
/// rule decided.
#[derive(Debug)]
pub enum PutError {
    Quota {
        status: u16,
        message: &'static str,
    },
    /// A write made against a version the document has since moved past. It
    /// is not a failure of the write; it is the write arriving too late to be
    /// the one that counts.
    Stale,
    Storage(String),
}

impl PutError {
    pub fn stale_message() -> &'static str {
        "this document was published again while you were editing it"
    }
}

impl std::fmt::Display for PutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PutError::Quota { message, .. } => write!(f, "{message}"),
            PutError::Stale => write!(f, "{}", PutError::stale_message()),
            PutError::Storage(message) => write!(f, "{message}"),
        }
    }
}

impl Store {
    pub async fn open(
        blobs: Arc<dyn BlobStore>,
        config: Arc<Configuration>,
    ) -> Result<Store, String> {
        let (entries, index_version) = load_index(blobs.as_ref()).await?;
        Ok(Store {
            blobs,
            config,
            state: Mutex::new(StoreState {
                entries,
                index_version,
            }),
        })
    }

    pub async fn get(&self, slug: &str) -> Option<IndexEntry> {
        self.state.lock().await.entries.get(slug).cloned()
    }

    /// Every document, newest first, as the listing endpoint wants.
    pub async fn list(&self) -> Vec<IndexEntry> {
        let state = self.state.lock().await;
        let mut documents: Vec<IndexEntry> = state.entries.values().cloned().collect();
        documents.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        documents
    }

    /// The stored source of a document, if it kept one. There is only ever one
    /// -- the source of the version the index names -- so its key carries no
    /// digest.
    pub async fn read_source(&self, slug: &str) -> Result<Vec<u8>, BlobError> {
        self.blobs.get(&source_key(slug)).await
    }

    pub async fn read(&self, slug: &str, digest: &str) -> Result<Vec<u8>, BlobError> {
        self.blobs.get(&document_key(slug, digest)).await
    }

    /// Writes a version and names it in the index, returning the stored entry.
    /// Admission, the staleness check and the index mutation all happen under
    /// the same lock, so two uploads racing for the last of a quota cannot both
    /// be admitted and two editors saving at once cannot both believe they won.
    pub async fn put(&self, v: Publication) -> Result<IndexEntry, PutError> {
        let size = (v.html.len() + v.source.len()) as i64;
        // The lock is held across the blob writes as well as the index update.
        // A refused upload must cost nothing in storage, so admission comes
        // first; and two uploads racing for the last of a quota cannot both be
        // admitted.
        let mut state = self.state.lock().await;
        self.admit(&state, &v.slug, &v.owner, size, now_unix())?;
        // Under the lock, so a check against the index cannot be overtaken by
        // the write it is guarding.
        if let Some(current) = state.entries.get(&v.slug) {
            if !v.base_sha.is_empty() && current.sha != v.base_sha {
                return Err(PutError::Stale);
            }
        }
        self.blobs
            .put(
                &document_key(&v.slug, &v.digest),
                v.html.into_bytes(),
                "text/html; charset=utf-8",
            )
            .await
            .map_err(|err| PutError::Storage(err.to_string()))?;
        // A version published from HTML replaces one published from markdown:
        // the stale source would otherwise be reopened by an editor as though
        // it were what the document now says.
        if v.source.is_empty() {
            let _ = self.blobs.delete(&[source_key(&v.slug)]).await;
        } else {
            self.blobs
                .put(
                    &source_key(&v.slug),
                    v.source.into_bytes(),
                    "text/plain; charset=utf-8",
                )
                .await
                .map_err(|err| PutError::Storage(err.to_string()))?;
        }
        let now = timestamp();
        let mut created = now.clone();
        let mut example = false;
        let mut replaced = String::new();
        let (mut owner, mut owner_id) = (v.owner, v.owner_id);
        if let Some(existing) = state.entries.get(&v.slug) {
            created = existing.created_at.clone();
            // A replacement keeps what the document already is: an example
            // stays an example, and its publisher -- and publisher id -- do not
            // change hands. A document with no publisher belongs to no one in
            // particular and stays that way: the first person to save it must
            // not become its owner, or everyone else loses the document they
            // were editing.
            example = existing.example;
            owner = existing.publisher.clone();
            owner_id = existing.publisher_id.clone();
            replaced = existing.sha.clone();
        }
        let entry = IndexEntry {
            slug: v.slug.clone(),
            title: v.title,
            sha: v.digest.clone(),
            size,
            created_at: created,
            updated_at: now,
            example,
            publisher: owner.to_lowercase(),
            publisher_id: owner_id,
            source_format: v.source_format,
        };
        let previous = state.entries.insert(v.slug.clone(), entry.clone());
        if let Err(err) = self.save_locked(&mut state).await {
            // The bytes are stored but the index naming them is not, so the
            // document does not exist as far as any later run is concerned.
            // Undo the in-memory half rather than report a success that will
            // vanish.
            match previous {
                Some(previous) => state.entries.insert(v.slug.clone(), previous),
                None => state.entries.remove(&v.slug),
            };
            return Err(PutError::Storage(err.to_string()));
        }
        // The reader only ever loads the entry's own digest, so a version this
        // replacement left behind is unreachable the moment the index above is
        // durable. Pruning it after that point, rather than before, means a
        // crash mid-write never leaves the current version missing.
        if !replaced.is_empty() && replaced != v.digest {
            self.prune_other_versions(&v.slug, &v.digest).await;
        }
        Ok(entry)
    }

    /// Enforces the storage ceilings a write must clear, under the lock that
    /// makes its index entry. `owner` is the caller's owner key exactly as
    /// `Server::owner` returns it -- a GitHub login, a visitor key, or "" for
    /// the one bucket every unidentified caller shares.
    fn admit(
        &self,
        state: &StoreState,
        slug: &str,
        owner: &str,
        size: i64,
        now: i64,
    ) -> Result<(), PutError> {
        let existing = state.entries.get(slug);
        let replacing = existing.is_some();
        let previous_size = existing.map(|e| e.size).unwrap_or(0);

        let (mut total_bytes, mut owner_bytes) = (0i64, 0i64);
        let mut owner_documents = 0usize;
        let mut owner_uploads_this_hour = 0usize;
        let cutoff = now - 3600;
        for (key, entry) in &state.entries {
            total_bytes += entry.size;
            if entry.publisher != owner {
                continue;
            }
            owner_bytes += entry.size;
            // The document being replaced is not a new document, and is not
            // counted again against the count it already counts toward.
            if key != slug {
                owner_documents += 1;
            }
            if parse_timestamp(&entry.updated_at).is_some_and(|updated| updated > cutoff) {
                owner_uploads_this_hour += 1;
            }
        }
        total_bytes += size - previous_size;
        owner_bytes += size - previous_size;

        let limits = self.config.storage;
        if total_bytes > limits.total {
            return Err(PutError::Quota {
                status: 507,
                message: "this deployment has no room left",
            });
        }
        if owner_bytes > limits.per_owner {
            return Err(PutError::Quota {
                status: 507,
                message: "your storage quota is used up; delete a document first",
            });
        }
        if !replacing && owner_documents >= limits.documents_per_owner {
            return Err(PutError::Quota {
                status: 507,
                message: "you have reached the document limit; delete one first",
            });
        }
        if owner_uploads_this_hour >= limits.uploads_per_hour {
            return Err(PutError::Quota {
                status: 429,
                message: "too many uploads this hour; try later",
            });
        }
        Ok(())
    }

    /// Removes every stored version of slug except `keep`. A failure to remove
    /// one is not an upload failure; it leaves an unreachable object behind for
    /// a future cleanup to find, nothing more.
    async fn prune_other_versions(&self, slug: &str, keep: &str) {
        let Ok(found) = self.blobs.list(&document_prefix(slug)).await else {
            return;
        };
        let stale: Vec<String> = found
            .into_iter()
            .map(|o| o.key)
            .filter(|key| *key != document_key(slug, keep))
            .collect();
        if !stale.is_empty() {
            let _ = self.blobs.delete(&stale).await;
        }
    }

    /// Deletes every stored version of a document and its index entry,
    /// returning how many versions went. The index entry goes last: until it
    /// does the document is still listed, which is a better half-state than a
    /// listing pointing at nothing.
    pub async fn remove(&self, slug: &str) -> Result<usize, String> {
        let mut removed = 0;
        if let Ok(found) = self.blobs.list(&document_prefix(slug)).await {
            let keys: Vec<String> = found.into_iter().map(|o| o.key).collect();
            if !keys.is_empty() && self.blobs.delete(&keys).await.is_ok() {
                removed = keys.len();
            }
        }
        // The source is not a version, so it is not counted among them; it
        // goes with the document all the same.
        let _ = self
            .blobs
            .delete(&[
                source_key(slug),
                examples_key(slug),
                room_key(slug),
                room_lock_key(slug),
            ])
            .await;

        let mut state = self.state.lock().await;
        state.entries.remove(slug);
        self.save_locked(&mut state)
            .await
            .map_err(|err| err.to_string())?;
        Ok(removed)
    }

    /// Writes the index, and only over the version these entries were read
    /// from. On a single-writer deployment the mutex already guarantees that,
    /// and the check costs a comparison; on a bucket two processes can reach,
    /// it is what stops one of them overwriting the other's documents. A
    /// conflict is reported rather than retried, because what to do about it
    /// is the caller's decision.
    pub async fn save_locked(&self, state: &mut StoreState) -> Result<(), BlobError> {
        let raw =
            serde_json::to_vec(&state.entries).map_err(|err| BlobError::Other(err.to_string()))?;
        let at = self
            .blobs
            .swap(INDEX_KEY, raw, &state.index_version)
            .await?;
        state.index_version = at;
        Ok(())
    }
}

/// Makes a new document's link unguessable.
pub fn random_suffix(config: &Configuration) -> String {
    let alphabet = config.suffix_alphabet.as_bytes();
    crate::auth::random_bytes(config.suffix_length)
        .into_iter()
        .map(|b| alphabet[b as usize % alphabet.len()] as char)
        .collect()
}

/// The same title yields the same slug on every backend.
pub fn slugify(value: &str, config: &Configuration) -> String {
    let mut out = String::new();
    let mut previous_dash = false;
    for c in value.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
            previous_dash = false;
        } else if !previous_dash {
            out.push('-');
            previous_dash = true;
        }
    }
    let slug: String = out
        .trim_matches('-')
        .chars()
        .take(config.slug_max)
        .collect();
    slug.trim_matches('-').to_string()
}

/// The suffix a seeded example gets instead of a random one. The examples are
/// the documents whose links are written down -- in the README, in a talk, in
/// a bookmark -- and re-seeding used to mint a new random suffix and break
/// every one of those links. Deriving the suffix from the base makes it stable
/// across a reseed, while still looking like the random suffix every other
/// document carries. It is not a secret: an example is public on purpose.
pub fn example_suffix(base: &str, config: &Configuration) -> String {
    let sum = Sha256::digest(format!("komodoc example {base}").as_bytes());
    let alphabet = config.suffix_alphabet.as_bytes();
    sum.iter()
        .take(config.suffix_length)
        .map(|b| alphabet[*b as usize % alphabet.len()] as char)
        .collect()
}

/// The content hash a document is addressed by.
pub fn digest_of(html: &str) -> String {
    hex::encode(Sha256::digest(html.as_bytes()))
}
