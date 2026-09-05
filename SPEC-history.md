# SPEC: history, as a document that is never lost

Status: proposed. Nothing here is built. This replaces an earlier draft of
the same spec, which kept history at the grain of a publish. There is no
publish in this one, and that is the point of it. It depends on the session
model in `SPEC-sync.md` and changes two of that spec's decisions, listed
under "What changes elsewhere".

## The problem

A document has one version. `put` writes the new HTML, names it in the index,
and prunes every other object under `documents/<slug>/`; the source lives at
one unversioned key, `sources/<slug>`, overwritten on every save. The only
trace a revision leaves is `updated_at`, and the only memory of what the
text used to say is inside the comments: each one quotes the passage it was
made on, which is how it re-anchors when the passage moves, and why it gets
the badge "Needs re-anchoring" when the passage is gone.

That badge is the whole of what a reviewer gets today about a change. They
come back to a link a week later, and the document is different; which
sentences changed, whether their comment was acted on, and what the passage
they objected to became are all questions the document cannot answer. The
author is in the same position from the other side: at the end of a
revision cycle a journal wants a response to reviewers, and it is written
by hand from memory, one comment at a time.

The editor has the mirror-image problem. What is typed lives in a Yjs
session the server relays and forgets: `end_editing` clears the log when the
last socket closes, so a draft exists only in open tabs, and a tab closed by
mistake, a laptop that dies, or a browser that restarts loses it. The save
button, the "unsaved changes" badge, the `beforeunload` prompt, Ctrl-S, the
`base_sha` check and the 409 that says "reload before saving over it" are all
consequences of one fact: the durable copy of the document and the thing
being edited are two different objects, and the reader is asked to carry one
to the other by hand.

Overleaf sells history as a premium feature: a timeline, a diff between two
points, labelled versions, restore. typst.app has none, and delegates it to
git (`SPEC-typst-app-research.md`, §5, calls this the clearest gap). Both
keep history of the source, for the people editing it. Neither connects it
to comments, because in both the comments live in the editor, not on the
text a reader was shown. That connection is the thing to build, and it is
easier to build once there is only one object.

## The decision: one document, always current

Four rules. Everything below follows from them.

**The session is the document.** The Yjs document the room holds is the
document, not a draft of it. It is persisted by the server and outlives
every socket. It is seeded once, from the source the document was created
with, and never seeded again: the open question at the end of
`SPEC-sync.md`, whether the session should outlive the client, is answered
yes, for the browser as much as for the sync client. Whoever opens the
document, in whatever tool, joins what is there.

**Readers see the current text.** A reader's browser joins the session
read-only, receives updates as they happen, and renders the text with the
engine the editor already loads, after a pause long enough that a word is
never shown half-typed. Reading and editing are the same page with the
source pane folded away. There is no moment at which the author decides
that readers may now see what was typed; the answer to "when do readers see
it" is "now", and the answer to "what if I am not ready" is a pinned
checkpoint, below.

**Nothing derived is stored.** The server keeps the source and nothing
rendered from it. Every browser that shows the document renders it, from
the live text or from a checkpoint, with the same module the editor
previews with, so what is shown is by construction what the source says.
This removes `documents/<slug>/<sha>.html`, `prune_other_versions`, the
`html` field of a publish, `max_html` as a ceiling on anything but the
source, and the whole class of "the HTML and the source disagree". A
document published as HTML from Quarto or a notebook has HTML for its
source, format `html`, and is stored and shown as such; it is not derived
from anything the store can see.

**There is no save.** Nothing a reader does makes the document durable,
because it always is. History is a sequence of **checkpoints** the server
takes on its own, at moments that mean something, and the only deliberate
act left is naming one. Restore is an edit. Nothing is ever rewritten or
removed, except by `destroy`.

## What a checkpoint is

A checkpoint is the source of the document at one moment, named by the
sha256 of its bytes. The server takes one:

- **After quiet.** When the session has had no update for `--checkpoint
  5m`, and the text differs from the last checkpoint.
- **When the last editor leaves.** The last owner socket closing, if the
  text differs from the last checkpoint. This is the one that replaces
  `end_editing`'s forgetting.
- **When a comment is made.** If the text differs from the last checkpoint,
  a checkpoint is taken before the comment is stored, so that every comment
  sits on a checkpoint by construction: what the reviewer saw is on record
  the moment they say something about it.
- **On a write from outside.** `komodoc publish` on an existing document,
  a file written under `komodoc sync`, and a restore each ask for one,
  because each is a deliberate act by the author, and a deliberate act is
  worth a mark in the timeline. Owner-only, as a `y-checkpoint` message on
  the socket or implied by the route.
- **When somebody names the moment.** "Name this point" in the reader or
  `komodoc label` takes a checkpoint if the current text is not already one,
  then labels it.

A checkpoint whose SHA is already in the manifest is not written again and
adds no entry: quiet after quiet costs nothing. Two checkpoints by the same
author within a short span are still two checkpoints; the timeline in the
reader shows them folded, and folding is a matter of display, never of
storage.

Between checkpoints the Yjs update log is the only record. It is kept
persistently too, as the live state of the session, so a server restart
loses nothing; but it is not history in the sense of this spec, and it is
rolled up into a snapshot on the same `y-snapshot` schedule as today. The
grain of what is promised to be never lost is the checkpoint, and the quiet
interval says how fine that grain is.

## Storage

Three kinds of object. `sources/<slug>` goes away in favour of the first;
nothing may live under `sources/<slug>/` on the directory store, and there
is no longer a reason to want to.

| key | contents | who may read |
| --- | --- | --- |
| `sessions/<slug>` | the Yjs state of the live document, as one update, replaced on every snapshot and checkpoint | the server |
| `history/<slug>/index.json` | the manifest: the list of checkpoints, oldest first | anyone who may read the document |
| `history/<slug>/<sha>` | one checkpoint: the source bytes | anyone who may read the document |

A manifest entry:

```json
{
  "sha": "4f2a91c…",
  "parent": "8b03d77…",
  "at": "2026-09-05T14:02:11Z",
  "by": "vincentarelbundock",
  "why": "quiet",
  "source_format": "typst",
  "size": 61240,
  "label": "sent to the journal",
  "commit": "a1b2c3d…",
  "dirty": false
}
```

`parent` is the checkpoint before it. The chain is linear because there is
one document and every write goes into it; a restore has the current
checkpoint as `parent` and an old one's bytes, and says so in `why`. `why`
is one of `quiet`, `left`, `comment`, `cli`, `sync`, `restore`, `label`.
`by` is the owner for `quiet` and `left`, since only the owner edits; for
`comment` it is the commenter, which is the one case where somebody other
than the owner causes a checkpoint. `commit` and `dirty` are git provenance,
below. `label` is empty until somebody sets one.

The index entry for a document keeps `sha`, which now names the latest
checkpoint rather than an HTML object, and gains nothing else; the reader
asks the manifest for the rest.

Order of writes when a checkpoint is taken, so that a crash leaves nothing
worse than an untidy history: the checkpoint object, then the session
state, then the index entry, then the manifest. A manifest missing its
newest entry is repaired by the next checkpoint, which names the missing
one as `parent` and finds the object present.

`remove` deletes `sessions/<slug>` and `history/<slug>/` along with
everything else, which is what `destroy` has promised in the README since
before there was a history to delete.

### What it costs, and how it is bounded

A checkpoint is the source: tens of kilobytes for a paper, before the store
compresses anything. Two hours of writing at one checkpoint per five quiet
minutes is at most twenty-four of them, and in practice far fewer, because
quiet after quiet is not written. A document with a hundred checkpoints is a
few megabytes, on a store that charges a cent and a half per gigabyte-month.
No HTML is stored at all, which on a typst document with figures is the
saving that pays for all of this.

History is counted where the quotas look: the index entry's `size` is the
session state plus every checkpoint object, and `admit` enforces the
per-owner and total ceilings against it unchanged. Above the ceiling a
checkpoint is still taken -- refusing it would lose work, which is the one
thing this spec exists to prevent -- and the reader is told the document is
over its quota, so that the fix, deleting a document or a checkpoint, is
made by a person rather than by the timer. The sandbox is bounded as it is
today, by expiry: a document that expires takes its session and its history
with it.

`--history N` caps checkpoints per document for an operator who wants one:
when a checkpoint would exceed it, the oldest unlabelled one is dropped, and
the oldest labelled one only when every checkpoint is labelled. The default
is no cap. `--history 0` keeps no history beyond the session state, which is
today's behaviour with the losing removed.

### Git provenance

When `publish` or `sync` runs inside a git repository, the checkpoint it
causes records the commit the working tree was at and whether it was dirty.
This is one process spawn, and it is the pointer into the history the
author's files already have, which for a document written in Quarto is the
only source history there is. `--no-git` leaves it out. The server records
what it is sent and checks only that a commit is forty hex characters.

## Reading without stored HTML

The reader today loads `/raw/<slug>/<sha>.html` into a frame on the
documents origin, and the agent injected into that page reports its text
back for anchoring. The frame stays, and so does its origin: it is what
confines a document that turns out to be hostile, and the agent is the only
thing on either side that touches the DOM. What changes is where the HTML
comes from.

`/raw/<slug>/` on the documents origin serves an empty shell with the agent
in it. The reader joins the session, renders the text with the engine, and
sends the page to the frame with the `preview` message the editor already
uses on every keystroke. A reader renders after a second of quiet; the
editor after sixty milliseconds, as now. A checkpoint is shown the same way,
from bytes fetched at `history/<slug>/<sha>`. A document whose format is
`html` is sent as it is.

Two consequences, one of them a change of policy.

**The source becomes readable by anyone who may read the document.** It
has to be: the browser cannot render what it is not given. Today the source
is owner-only and readers see only HTML, which for markdown is a distinction
without a difference, and for typst hides source comments, unused
definitions and whatever else the author left in the file. This spec accepts
that. A source that must stay private is a document that must be published
as HTML, and the format `html` is how that is said.

**Every reader downloads an engine.** The markdown module is small; the
typst module is thirty megabytes, cached for a year under a digest URL, so
it is paid once per browser and not per document. That is the price of
storing nothing rendered, and it is the right trade for this project; but a
server that has the engine natively could render on request, keep the
result in memory, and store nothing, for a client that cannot run the
module. That is compatible with the third rule and left out of the first
version. See the open questions.

## Comments know their checkpoint

Two fields on `Comment`, both set by the server in `apply`, never by the
client:

- `revision`: the SHA of the checkpoint the comment was made on, which is
  the checkpoint `apply` takes or finds current, by the rule above.
- `resolved_in`: the SHA current when it was resolved, alongside
  `resolved_at`.

A comment from before the fields existed has neither, and is treated as made
on the oldest checkpoint the manifest knows. Both fields travel in the
JSON-LD export as extra properties, which the Web Annotation model permits
and the export already relies on for `resolved`.

With these, a comment's passage can be looked up in any checkpoint's text
by the match that anchors it in the reader: the browser renders the
checkpoint, takes the visible text the way the agent does, and searches it.
Found at its own checkpoint, followed forward until the checkpoint where it
stops being found, and quoted from the current text if it is still there.
That lookup is the primitive under everything in the next section, and it
runs entirely in the browser, from the manifest and the checkpoints it
fetches on demand.

## What is built on it

In the order they are worth having.

### The timeline

`GET /api/documents/<slug>/history` returns the manifest. `komodoc history
c9k` prints it:

```
sha      at                    by                  why      label
8b03d77  2026-09-03 09:12:40   vincentarelbundock  cli
4f2a91c  2026-09-05 14:02:11   vincentarelbundock  sync     sent to the journal
c07e1aa  2026-09-05 16:40:03   annegrandchamp      comment
d1e0f42  2026-09-05 17:02:19   vincentarelbundock  left     *
```

In the reader, a `history` button in the toolbar, in reading and editing
alike, opens a panel listing checkpoints by day, newest first, with the
author and the reason, labelled ones standing out, and runs of unlabelled
checkpoints by one person folded to their first and last. Selecting one
shows it in the document pane, read-only, with a bar saying which
checkpoint is showing and offering "Back to now", "Restore", "Name this
point" and "Copy link". A label is a `PATCH` to the manifest entry,
owner-only, from the panel or from `komodoc label c9k 4f2a91c "sent to the
journal"`.

This panel is what replaces the save button, the "saved" badge and the
`beforeunload` prompt. The only status the toolbar keeps is connectivity:
"offline, changes kept in this browser" while the socket is down, with
y-indexeddb holding the local state until it is back.

### The passage, then and now

Every comment card gains one line when the passage has changed since the
comment was made: what it said then, what it says now, or that it is gone,
with the checkpoint where it went. This replaces the badge as the answer to
"Needs re-anchoring": the comment is still shown as needing a home, but the
reader can see what happened to the passage instead of guessing. A document
with no changed passages costs one request for the manifest and nothing
more.

### What changed since

A reader picks a checkpoint -- by default the one their last comment was
made on, or the last one they opened, which the browser remembers locally --
and the reader lists what changed between it and now, as passages rather
than as a source diff. Each entry is a hunk of the word-level diff of the two
visible texts, with a few words of context either side. A changed or
inserted passage anchors into the current document by the same quotation
mechanism a comment uses, so clicking it reveals the place; a deleted
passage cannot anchor and is shown in the list with the words that survived
on either side of it.

This is the same diff `SPEC-sync.md` needs for its three-way merge, by word
because paragraphs are single lines. The list is a component beside the
comments, not a rendering of insertions and deletions inside the document:
painting a diff into rendered HTML is a research problem and painting it
into a typst document is not possible, whereas anchoring a quotation is a
thing the reader does already.

### The response to reviewers

```sh
komodoc export c9k --format response --since 4f2a91c
```

For every comment made at or after the given checkpoint -- all of them
without `--since` -- a section with the passage as the reviewer saw it, the
thread, and the passage as it now stands or the note that it was removed,
and for a resolved comment the checkpoint it was resolved in. Grouped by
reviewer, because that is how a response is organised, and in the markdown
the `export` command already writes, so it goes into a Quarto document as it
is:

```markdown
## Reviewer: annegrandchamp

### 1. commenting, resolved in d1e0f42

> The confidence interval does not say that the parameter is inside it with 95% probability.

**Then:** "…with 95% probability, the true value lies in the interval…"

**Now:** "…95% of intervals built this way, over repeated samples, cover the true value…"

Fixed as suggested; see also the new footnote on coverage.
```

The last line is the author's reply from the thread, which is where the
response gets written from now on: replying to the comment in the reader is
writing the response document. That is the feature to show first, because
nobody else can build it -- their comments do not live on the text a reader
was shown -- and because it is cheap: one export format over the lookup
above.

### Diff and restore

In the editor, a merge view between any two checkpoints' sources, which
CodeMirror provides as a package with per-hunk accept and reject, so an
author can pull a sentence back from an old draft without leaving the page.
`komodoc diff c9k 8b03d77 4f2a91c` prints the same as a unified diff of the
sources, for a terminal or a pipe.

Restore, from the panel or `komodoc restore c9k 8b03d77`, writes that
checkpoint's source into the live session as an edit: the server diffs it
against the current text and applies the difference to the Yjs document, so
a coauthor typing at that moment keeps their words and sees the rest
change under them. It takes a checkpoint with `why: restore`. History is
never rewritten; the checkpoint restored from is still there, and so is the
one restored over.

### Pinning, later

The one thing lost by making readers see the current text is working on a
draft while readers keep seeing the old one. The answer is not a return to
publishing but a per-document setting, owner-only: "readers see this
checkpoint", after which the reader page shows that checkpoint to everyone
but the owner, with a line saying a newer text exists, until the owner
unpins or pins a later one. The history panel is where it is set. It is
left out of the first version because it is a policy on top of the timeline
and needs the timeline first; and because it may turn out that nobody asks
for it.

## What changes elsewhere

**In the reader.** The save button, `dirty`, `savedSource`, `baseSHA`,
`shownSHA`, `published()`, the 409 handling, `beforeUnload`, and the state
badge except for connectivity all go. Ctrl-S is kept as a key that does
nothing but say, once, that everything is kept. `startEditing` no longer
seeds or fetches source: it unfolds the source pane over the Yjs document
the page already holds for reading. `paintPreview` runs for readers too, on
a longer timer. The toolbar while editing is layout, comments, history; while
reading, comments, history.

**In the server.** `yrs` goes into the server, which `SPEC-sync.md` said it
would not. The reason it stayed out was that the server had nothing to do
with the text but relay it; now it takes checkpoints when no browser may be
present, applies a restore or a command-line publish as a diff into the live
document, and persists the state, all of which need the document rather
than its updates. The room keeps a `yrs::Doc` per open document, applies
every relayed update to it, and answers `y-open` from it. `y-open` is
answered for any socket that may read the document, with incoming `y-*`
still dropped from anyone but the owner. `end_editing` takes a checkpoint
instead of forgetting. `POST /api/documents` on an existing slug carries
`source` and `source_format` and no `html` or `base_sha`; it becomes an edit
into the session, followed by a checkpoint, and cannot conflict. `PutError::Stale`
and `prune_other_versions` go. `seed_examples` renders the seeded markdown in
memory to anchor the example comments, as `visible_text` does today from
stored HTML, and stores the source.

**In `SPEC-sync.md`.** "The server is a relay and stays one" and "`yrs` does
not go into the server" are superseded by the paragraph above. "A publish
from outside" is obsolete: there is no publish to be outside of, and a
`komodoc publish` by someone else while the sync client runs is an edit the
client receives like any other. `--no-publish` goes, since writing the file
is syncing and nothing more; the file's save asks for a checkpoint, which is
the only trace the keystroke leaves. The sync client's `seed: true` branch
goes too: the server seeds once, at creation, and the client always joins.

**In the README.** Destroy already promises to delete history. The
description of saving, of "reload before saving", and of the two-tab
conflict comes out, and a paragraph about the timeline goes in.

## What it is not

It is not keystroke history. The Yjs log between checkpoints exists and is
persisted, and with garbage collection off it would give any past state and
per-character attribution, which is Overleaf's premium feature. It is rolled
up on the snapshot schedule instead, because it grows with every keystroke
and deletion, and because it attributes to nobody in particular while a
document has one owner. If the editor grows coauthors with identities of
their own, keeping the log is a flag, and it would sit beside this spec, not
replace it.

It is not a store of rendered pages, ever. If an operator wants the rendered
form of an HTML document kept when its source is regenerated -- a notebook
whose figures changed -- that is a new checkpoint of the `html` source,
which is what the format means.

It is not a git remote. Provenance is a commit hash recorded on a
checkpoint, so the author can find it; it is not a way to push or pull.

## Steps

1. **The document on the server.** `yrs` in the room; `sessions/<slug>`
   written on snapshot; `y-open` answered from the document for any reader;
   a server restart that comes back with the text intact. Tests: two
   browsers, one restart, no words lost; a reader receives updates and
   cannot send them.
2. **Checkpoints.** The quiet timer, the last-editor rule, the comment
   rule, the `y-checkpoint` message; the manifest and the objects in the
   order above; `size` including history; `--history`, `--checkpoint`;
   `remove` deleting the prefix. Tests: quiet after quiet writes nothing, a
   comment lands on a checkpoint whose text contains its quotation, a crash
   between the index and the manifest is repaired by the next checkpoint.
3. **Reading from source.** The shell at `/raw/<slug>/`, the reader
   rendering the live text and painting the frame, format `html` sent as
   is; the HTML object, `prune_other_versions` and `PutError::Stale`
   removed; `POST` on an existing slug as an edit into the session.
4. **The reader loses its save.** Everything in "What changes elsewhere,
   in the reader". The connectivity indicator and y-indexeddb.
5. **The timeline.** `GET .../history`, the label `PATCH`, `komodoc
   history` and `komodoc label`, the panel, viewing a checkpoint.
6. **Comments know their checkpoint.** The two fields, set in `apply`, in
   both exports. The passage-then-and-now line on the card.
7. **The response export.** `--format response` and `--since`.
8. **What changed since.** The word-level diff in Rust for the command
   line, and in the browser for the list beside the comments; anchoring
   hunks by quotation.
9. **Diff and restore.** The merge view in the editor, `komodoc diff`,
   `komodoc restore`, restore as a server-side diff into the document.
10. **Provenance.** The git fields from `publish` and `sync`.

Steps 1 to 3 are the foundation and the ones that change what exists; each
is a couple of days, and 3 is the one that removes the most code. Step 4 is
an afternoon once 3 is in. 5 to 7 are a day each and 7 is the one to
demonstrate. 8 and 9 are the browser work and take longer. 10 can go
anywhere after 2.

## Open questions

- **The source is public to readers.** Accepted above, and the honest
  price of storing nothing rendered. Who a reader is becomes a question
  `SPEC-sharing.md` answers: "anyone who may read the document" in the
  storage table means the reader role there, and a `private` document has
  named readers only. Whether a document may ask for its source to be hidden
  from readers who may see the rendered page, at the cost of the server
  rendering for them, is the same question as the next one.
- **Rendering on the server, never stored.** The server has the engine
  natively. A `/raw/<slug>/<sha>` that renders on request, holds the result
  in memory for a while and writes nothing would serve a client without
  WebAssembly, a reader who does not want thirty megabytes for a typst
  document, a search engine, and a document whose source is not for
  readers. It is compatible with every rule here and is not in the first
  version.
- **The quiet interval.** Five minutes is a guess in both directions: long
  enough that a paragraph is a checkpoint and not each sentence of it,
  short enough that a session's worth of work is many points rather than
  one. The last-editor rule bounds the loss in any case.
- **How long a session may stay hot.** A `yrs::Doc` per open document in
  memory is fine for a server with a few dozen open at once and not for
  one with ten thousand. Evicting a document nobody has open, after a
  checkpoint, and reloading it from `sessions/<slug>` on the next `y-open`,
  is the obvious answer, and whether the first version needs it depends on
  the deployment.
- **Documents that need files beside them.** A typst source that `#import`s
  a file or reads a `#bibliography` renders on the author's machine and
  nowhere else, because the engine's file map in the browser has no
  directory. Stored HTML used to hide this from readers; now it does not. The
  answer is the project, several files travelling with the source, which
  `SPEC-rust.md` lists as not built and `SPEC-sync.md` is the natural place
  to feed. Until then such a document is published as HTML.
- **Checkpoints on a document that never quiets.** A sync client that
  writes the file every few seconds all day, or a bot, would make the quiet
  rule fire rarely and the `sync` rule fire constantly. A minimum spacing
  for `why: sync`, perhaps the quiet interval itself, may be wanted.
