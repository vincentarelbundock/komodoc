# SPEC: `komodoc sync`, the file on disk as a peer in the session

Status: proposed. Nothing here is built.

## The problem

A document published from markdown or typst can be edited in the page it is
read in, and several people can edit it at once: the source is a Yjs document,
the server relays updates, and a save publishes a revision to the same link.
That editor is the only door into a live session. The author who wrote the
paper in vim, Positron or Emacs, and who renders it from a Makefile, has to
choose between their own tools and the session: publish from the command line
and the session is bypassed, or open the browser and leave the tools behind.

`komodoc sync` makes the file on disk a peer in the session. Run it on the
file a document was published from, and edits made in any local editor flow
into the session as a coauthor types in the browser, edits made in the browser
land in the file, and saving the file publishes. The browser stays the tool
for simultaneous work and for whoever has no toolchain; the file stays the
tool for the author who has one. Neither is an import of the other.

This is also the first program that is not a browser to read and write the
shared document, which `SPEC-rust.md` lists as the reason the host is Rust.
Whatever joins a session on behalf of an author later -- a language model
acting on comments, a batch job -- joins it the way this command does.

A note on names. "Agent" already means the script the server injects into
every document it serves (`web/src/agent`). This spec never uses the word
for the program described here; that program is the sync client.

## What it looks like

```sh
komodoc sync c9k paper.typ
```

The ID is the one `list` prints, the same as `comment`, `edit` and `export`
take. The file is the source the document was published from, or will be.
The command runs until interrupted, and says what it is doing:

```
syncing paper.typ with https://komodoc.arelbundock.com/docs/typst-what-a-confidence-interval-does-not-say-5vvxv8ebpd
joined the session (2 people editing)
paper.typ changed: published 4f2a91c
session changed: wrote paper.typ
paper.typ changed: published 8b03d77
```

Flags:

| flag | meaning |
| --- | --- |
| `--server` | the deployment, as every other command takes it |
| `--no-publish` | keep the file and the session in step, but never publish; a save is a save in the browser |
| `--interval 250ms` | how long the file has to stay quiet before it is read, and the session before it is written |

Writing the file publishes, unless told not to. In the browser a save is a
deliberate keystroke; on disk, writing the file is that keystroke. What is
published is exactly the text of the session at that moment -- not the file --
so a browser in the session sees "saved" rather than a warning, by the rule
`published()` in `Reader.svelte` already applies.

Only the owner of a document may edit its source, in the browser and here.
The command checks ownership before it starts and refuses with a plain
message, because the server drops a non-owner's `y-*` messages silently and a
sync client that ran anyway would sit there doing nothing.

## What does not change

The server is a relay and stays one. It holds the updates of a live session,
hands them to whoever joins, and never merges. `yrs` does not go into the
server in this spec; the sync client is where it goes, and the server needs
no new message and no new route. The browser editor is untouched. The
durable copy of a document is still the source a save stores.

The protocol the sync client speaks is the browser's, as it stands today:

| direction | message | meaning |
| --- | --- | --- |
| in | `hello {comments}` | on connect; the comments, which the client ignores |
| out | `y-open` | I am editing |
| in | `y-state {updates, seed, count}` | replay these, or seed the session from the stored source if `seed` |
| out, in | `y-update {update, replace?}` | one Yjs update, base64; `replace` means the whole state |
| out, in | `y-awareness {update}` | who is here, base64, relayed and never stored |
| in | `y-snapshot` | the log is long; send the whole state with `replace: true` |
| in | `y-peers {count}` | how many sockets the room has |
| in | `published {sha, title}` | a new version exists |

Identity is the `Authorization: Bearer` header the command line already
sends, on `GET /ws/<slug>`. The stored source and its SHA come from
`GET /api/documents/<slug>/source`, and publishing is `POST /api/documents`
with `base_sha`, which the store refuses with 409 if the document moved. All
of this exists. The updates are Yjs's binary v1 encoding, which `yrs` reads
and writes byte for byte, so a `yrs` document and a `Y.Doc` in a browser are
peers without either knowing which the other is.

## Design

### The peer

The sync client is a `yrs::Doc` with one `Text` named `source`, the name
`collab.js` uses, and a `yrs::sync::Awareness`, whose update encoding is
`y-protocols`'s. It connects with `tokio-tungstenite` over rustls -- no
OpenSSL, the same rule `reqwest` is under -- and drives everything from one
task, so an update from the socket and a change from the disk are never
applied to the document at the same time. The mutex the room uses for the
same reason on the server is a single task here.

On `y-state`:

- `seed: true` -- nobody is editing. The client is the one to start the
  session, and starts it from the stored source, not from the file: exactly
  one peer seeds, and it seeds from what is published, the same as a browser.
  Then the file is reconciled against the session as a local change (below),
  so a file that has moved on since the last publish becomes the first edit
  of the session rather than a second history of the same words.
- `seed: false` -- replay the updates, then reconcile the file the same way.

On `y-snapshot`, send `encode_state_as_update_v1` with `replace: true`,
which is what the browser does. On `y-awareness`, apply it and print who
joined or left. The client's own awareness entry is
`{user: {name: "<login> (sync)", color}}`, so a caret label in the browser
says where the other edits are coming from, even though the client has no
caret to show.

If the socket drops, reconnect with backoff, send `y-open` again, and treat
what comes back as above. Reconnecting is the ordinary case for a process
that runs all day on a laptop that sleeps.

### The mirror

Session to disk. Every update from the socket marks the document dirty. When
it has been quiet for the interval, the text of `source` is written to a
temporary file beside the target and renamed over it, so an editor never
reads half a write and a crash never leaves half a file. The digest of what
was written is remembered, and the watcher ignores the event that write
causes. Permissions are copied from the file being replaced.

Disk to session. The file is watched with `notify`. When it has been quiet
for the interval, it is read, normalised (CRLF to LF, invalid UTF-8 refused
with a message), and compared by digest with what was last written. If it is
the same, nothing happened. If it differs, the change is reconciled into the
document -- and reconciled is not replaced. Replacing the whole text would
delete and reinsert every character, which destroys the other editors'
concurrent insertions, every caret position, and the anchors of every
comment. The document gets the smallest set of inserts and deletes that turn
its text into the file's, applied in one transaction, against the text as it
stood at the start of that transaction. Because one task does everything,
no remote update lands between reading the document and writing to it.

Then, unless `--no-publish`, the session's text is rendered and published.

### The merge, which is the only hard part

A text editor is a snapshot client. It read the file at some moment, holds
the text in a buffer, and writes the buffer back whole when the author saves.
If a coauthor edited in the browser meanwhile, the buffer does not know, the
saved file lacks the coauthor's words, and a diff of the document against
the file would remove them. That is the case the design has to get right,
and the rest is plumbing.

The fix is a three-way merge. The client keeps the **base**: the text the
file and the session last agreed on, which is what was last written to disk
or last read from it without conflict. On a file change:

1. `local` = what the file says now; `remote` = what the document says now.
2. If `remote == base`, nothing happened in the session: diff `base` to
   `local`, apply. This is the common case and it is exact.
3. Otherwise merge `base`, `local` and `remote`. Edits to different regions
   go through, and the merged text becomes the target: diff `remote` to
   `merged`, apply. The coauthor's edits are already in `remote`, so they
   survive; the author's edits are in the diff, so they arrive.
4. Where both sides changed the same region, the **session wins**, the merged
   text is written back to the file, and a line is printed naming the region.
   The session has other people in it and the file has one; and the file's
   author is looking at a terminal that just told them, while the browser
   editors would find out only by noticing words vanish.

Afterwards `base` is the document's text, whichever branch ran.

The merge is by word, not by line. The editable examples were just unwrapped
into one line per paragraph, and a paragraph is the natural unit of prose
anyway; a merge by line would call any two edits to the same paragraph a
conflict, which is most of the conflicts a pair of coauthors would ever have.
`similar` diffs by word and by character; the merge itself is a small
function over two diffs, of the kind `diff3` has done for forty years, and
it is where the tests go.

The other direction has no merge to do. When the session changes and the
file is written, the editor either reloads it or does not: VS Code and
Positron reload a clean buffer without asking, vim does with `autoread` on
its next focus or command, Emacs polls under `auto-revert-mode`. A buffer
that is dirty when the file changes gets whatever that editor does about a
file changed on disk, and that is the editor's prompt to give, not ours. What
the client guarantees is that the file on disk is never behind the session
by more than the interval.

### Publishing

The client has the engine, natively. Markdown renders with `render_markdown_document`; typst
renders with `render_typst_document`, which reads includes, images and
bibliographies from the directory the file is in. That is more than the
browser can do today, where `#import` and `#bibliography` do not work
because the engine's file map is a directory reader, and it is the reason
publishing from the sync client rather than from the browser is worth having
even for someone who does all their typing in the browser.

The POST carries the session's text as `source`, the rendered page as
`html`, the format, the title the document already has, and `base_sha`. On
201, `base_sha` becomes the new SHA. On 409, the document moved under the
client; see the next section. A `published` broadcast for the client's own
SHA is the receipt and is ignored. Every browser in the session compares the
new source with its own text, finds them equal, and says "saved".

Publishing is debounced by the same interval as writing, and a publish that
is already in flight is not overlapped: the next one waits and carries
whatever the text is when it starts. A document over `max_html` is refused
here with the same message `publish` gives, before anything is sent.

### A publish from outside

Someone runs `komodoc publish` on the same slug, or saves from a browser that
was reading rather than editing. The room broadcasts `published` with a SHA
the client did not produce. The browser's rule is: fetch the source; if it
equals the session's text, fine; otherwise warn and refuse to save until the
page is reloaded. The client follows the same rule: fetch the source, adopt
the SHA as `base_sha` if the text matches, otherwise print that a newer
version was published from outside the session and stop publishing. The
file and the session keep syncing; only publishing stops, until the author
restarts the command, which re-seeds from what is now published and
reconciles the file against it. Merging the outside version into the
session is possible -- it is the same three-way merge -- and would let
everyone carry on; it is left out of the first version because it changes
what the browser sees without the browser having been told, and the
browser's own rule is to stop.

## Edge cases, and what is decided about each

- **The file does not exist.** Written from the stored source on start, and
  said so. This is how an author pulls a document down to edit locally.
- **The file exists and was never published.** Refused: `sync` takes a
  document that exists, `publish` makes one. The message says which to run.
- **The editor's temporary files.** Vim's swap and backup files, Emacs's
  `#paper.typ#`, an editor's `paper.typ~`: the watcher is on the one path,
  and events for any other name are ignored.
- **Editors that write by rename.** Vim, by default, writes a new file and
  renames it over the old one, so the inode changes. `notify` reports that
  as a remove and a create, or a rename, depending on the platform. The
  watcher is on the parent directory, filtered to the name, so it survives.
- **A trailing newline.** Editors add one; the browser does not. The document
  is what it is: a newline the editor adds is an edit like any other, it
  goes into the session, and the browser shows it. Nothing normalises it
  away, because a rule that stripped it would fight the editor forever.
- **Format on save.** A formatter that rewrites the file touches every line,
  which is a diff against everyone and a re-anchor of every comment. Not
  prevented; documented as the thing to turn off for a synced file.
- **Two sync clients on one file.** Two people cannot have the same file,
  but one person can run the command twice. The second is refused by a lock
  file beside the target, with the pid of the first in it.
- **Two sync clients on one document, different machines.** Fine. Each is a
  peer; the session is the meeting point, as it is for two browsers.
- **The document is deleted while syncing.** The socket closes with the
  reason the room gives; the client prints it and exits, and leaves the file
  alone.
- **Binary assets, other files.** Out of scope. The session is one `Text`,
  and the file map that would make a project of several files is not built
  (`SPEC-rust.md`, "still not built"). Images beside a typst file are read
  at render time and published inside the HTML; they are not synced, and
  a browser in the session cannot see them until the file map exists.

## What it is not

It is not live collaboration from vim. The local side of the session moves
when the file is written, and a file is written when the author saves -- or
every second or so with an editor's auto-save, which is the setting to
recommend. Two people typing in the same paragraph at the same time is the
browser editor's job, and the command's documentation says so in its first
paragraph.

It is not an editor plugin. A VS Code extension that made the editor buffer a
real Yjs peer, with the coauthors' carets drawn as decorations, would remove
the snapshot problem entirely, and would cover Positron with the same code.
It is a different and larger project; this command is what it would be
measured against, and what it would replace only for people who ask.

It is not a general file synchroniser. One file, one document, one session.

## Steps

1. **The peer.** `yrs`, `tokio-tungstenite`, the message types above, seed
   and replay, snapshot on request, awareness in and out, reconnect. A test
   runs `serve` in-process, joins two `yrs` peers through it, and checks
   that an insert on one is the text of the other, that the second to join
   replays, and that the log rolls over to a snapshot at `EDIT_LOG_MAX`.
   Nothing touches disk yet.
2. **The mirror, one direction each.** Session to disk with atomic writes
   and echo suppression; disk to session as a minimal diff. Tests drive the
   watcher with real writes into a temporary directory and assert on the
   document; and drive the document and assert on the file.
3. **The merge.** The three-way function, by word, with its table of cases:
   disjoint edits, adjacent edits, the same word, an insert at the seam of a
   deletion, an edit at either end of the text, an empty base. Then wired
   into the disk-to-session path with `base` bookkeeping, and a test in
   which a browser-shaped peer edits while the file holds a stale copy.
4. **Publishing.** Render natively, POST with `base_sha`, handle 201, 409 and
   the outside-publish broadcast. A test publishes through the sync client
   and checks that a browser-shaped peer would call it "saved": the stored
   source equals the session's text.
5. **The command.** `clap` subcommand, ownership check, the lock file, the
   messages above, `--no-publish`, `--interval`. A section in the README
   under "CLI", after "Edit", which opens with what it is not.

Steps 1 and 2 are a day each; step 3 is a day or two, most of it tests; 4
and 5 are a day together. Step 3 is where the risk is, and it is the step
with no dependency on the network, so it can be built and tested first if
that is where the doubt is.

## Open questions

- **Should the session outlive the client?** Today a session ends when the
  last socket closes, and a sync client that runs all day keeps it alive all
  day, which is a change in what a browser sees: a document with a long
  history of updates rather than a fresh seed. The `y-snapshot` rollover
  keeps the log bounded, so nothing breaks; but a session that lives for
  weeks is new, and a limit may be wanted.
- **Cursor positions.** An editor that speaks LSP knows where its cursor is.
  A future sync client could take that over a local socket and put it in
  awareness, so the browser draws a caret for the file. Not now.
- **Merging an outside publish** into the session rather than stopping, as
  discussed above. Worth revisiting once the browser has an opinion.
