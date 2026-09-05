// What this browser remembers on its own: which documents are starred, when
// each was last opened, how wide the panes are. None of it belongs to a
// document -- it is one reader's own -- so none of it leaves the browser.
//
// Storage can be switched off, and a page that throws when it is would be a
// page that does not load at all. Every read has a fallback and every write
// is allowed to fail.

export function read(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    return raw === null ? fallback : JSON.parse(raw);
  } catch {
    return fallback;
  }
}

export function write(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    /* it still applies to this page */
  }
}

export const VIEWED = "komodoc-viewed";
export const FAVORITES = "komodoc-favorites";
export const AUTHOR = "komodoc-author";
export const LINKED = "komodoc-linked";

/// Notes that this document was opened just now, which is what the landing
/// page sorts by.
export function markViewed(slug) {
  const seen = read(VIEWED, {});
  seen[slug] = new Date().toISOString();
  write(VIEWED, seen);
}
