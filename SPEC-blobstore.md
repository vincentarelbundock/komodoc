# SPEC: a blob store seam, and bring-your-own S3

Status: steps 1-5 implemented; 6-9 not. See "What was built" at the end.

## The problem

komodoc stores durable state in two places today, and each backend spells it
differently:

| what | `serve` | Worker |
| --- | --- | --- |
| index of documents | `<dir>/index.json` | `env.DOCS` object `index.json` |
| rendered versions | `<dir>/documents/<slug>/<sha>.html` | `documents/<slug>/<sha>.html` |
| editable source | `<dir>/documents/<slug>/source.txt` | `sources/<slug>` (`sourceKey`) |
| example annotations | seeded into the room | `examples/<slug>.json` |
| comments | `<dir>/comments/<slug>.json` | Durable Object `Room` |

The two implementations are hand-synced, and the comments in `store.go` and
`room.go` say so explicitly ("R2's counterpart", "the Durable Object's
counterpart"). A third storage target -- someone else's S3 bucket -- makes that
divergence worse unless the access itself is behind an interface first.

The goal is a deployment where the operator supplies S3 credentials, a cheap VPS
runs `komodoc serve`, and the VPS holds no durable state of its own. The bytes,
the bill, and the ownership of the data are the operator's.

## What ports and what does not

**Blobs and the index port cleanly.** Every access in both backends reduces to
six operations, and the whole surface is already narrow: `store.read`,
`store.readSource`, `store.put`, `store.remove`, `store.pruneOtherVersions`,
`loadIndex`, `saveLocked`, and their `env.DOCS.{get,put,delete,list}`
counterparts.

**Rooms do not.** A room is mutable, concurrent, latency-sensitive state with
open WebSockets attached. S3 is not a coordination primitive. The room question
is settled separately, below, and it is the harder half.

## The interface

```go
// blobStore is the bytes komodoc keeps, addressed by key, wherever they live:
// a directory, an R2 bucket, or an S3 bucket somebody else pays for. Keys are
// the Worker's layout -- "index.json", "documents/<slug>/<sha>.html",
// "sources/<slug>" -- so the two backends finally agree on names as well as on
// rules.
type blobStore interface {
    get(ctx context.Context, key string) ([]byte, error)   // errNotFound if absent
    put(ctx context.Context, key string, body []byte, contentType string) error
    delete(ctx context.Context, keys ...string) error      // absent keys are not an error
    list(ctx context.Context, prefix string) ([]blobInfo, error)

    // swap is the compare-and-swap the index needs: write body only if the
    // object's current version is `expect`, where the zero version means "only
    // if it does not exist". Returns errConflict if it moved.
    swap(ctx context.Context, key string, body []byte, expect version) (version, error)
    getVersioned(ctx context.Context, key string) ([]byte, version, error)
}

type version string // an ETag, or "" for absent
```

Six methods, and `swap` is the only interesting one. Three implementations:

- `fsStore` -- what `serve` does today, with `version` synthesized from a
  content digest and `swap` guarded by the existing `store.mu`. Single process,
  single writer; the CAS is bookkeeping, not contention control.
- `r2Store` -- a thin shim in `worker.js`, not Go, wrapping `env.DOCS` with
  `onlyIf: {etagMatches}` exactly as `updateIndex` already does.
- `s3Store` -- SigV4 against any S3-compatible endpoint. No SDK; SigV4 is ~60
  lines of WebCrypto in the Worker and `crypto/hmac` in Go, and pulling
  `aws-sdk-go-v2` into a binary that currently has a tiny dependency tree is a
  bad trade.

`store` keeps everything it has -- the entry cache, `admit`, the staleness
check, `ownedBy` -- and loses only `filepath`. Its mutex still guards
admission and the index mutation as one unit; what changes is that
`saveLocked` becomes `swap` and can now fail with `errConflict`.

## Compare-and-swap is the load-bearing assumption

`index.json` correctness rides on conditional writes. R2 has them; AWS S3 gained
conditional `PutObject` (`If-Match` / `If-None-Match`) in November 2024;
MinIO, Backblaze B2, Wasabi and Ceph vary by version.

A lost index update is a lost document, so this must not degrade silently:

1. At startup, `s3Store` probes the endpoint. Write a scratch key
   `.komodoc-probe`, read its ETag, attempt an `If-Match` put with a wrong
   ETag, and require a 412. Then attempt an `If-None-Match: *` put over an
   existing key and require a 412. Delete the scratch key.
2. If both hold, run normally.
3. If either fails, refuse to start unless `--single-writer` is passed. That
   flag is an assertion by the operator that exactly one process writes this
   bucket -- which is true of the VPS deployment by construction -- and makes
   the in-process mutex the authority, with the index written unconditionally.
   It must be a deliberate choice, printed at startup, not a fallback.

The same probe reports CORS: issue a preflight for the bucket and, when it
fails, print the exact policy JSON to paste. Presigned reads (below) are useless
without it.

## Bytes should not transit the VPS

If the VPS proxies every document read, the storage bill is replaced by a
bandwidth bill and the design has bought nothing.

- **Reads.** `GET /documents/<slug>` answers `302` to a presigned GET, TTL 120s.
  Ownership and example-policy checks happen before the redirect is minted, as
  they do before the read today.
- **Writes.** Publishes are already size-capped by `config.MaxHTML`, so they can
  keep going through the VPS; a presigned PUT is a later optimization and
  complicates `admit`, which must run *before* bytes land.
- **What the VPS still serves.** The shell, the reader, the room WebSocket,
  auth, and the index. That is genuinely cheap.

A presigned URL is a bearer token for its lifetime, which is why the TTL is
short. It does not weaken what exists: an unlisted document's slug was already
the capability.

## Rooms

Four options, in the order I would take them:

1. **Rooms in the operator's bucket, VPS-authoritative.** `room.path` becomes
   the key `rooms/<slug>.json`; `load` reads it once on first `get`, `save`
   writes it debounced (say 2s, and immediately on the last socket closing).
   The in-memory copy stays the source of truth while anyone is connected, so
   the existing mutex is still correct and comment latency is unchanged. The
   VPS becomes genuinely stateless: losing it loses at most the last couple of
   seconds of comments. **This is the recommendation.** It gives up serving one
   document from two VPSs, which this deployment shape does not do anyway.
2. Rooms on VPS local disk -- what `room.go` does now. Simplest, but then "we
   store nothing" is false, and comments are the part an operator would most
   hate to lose.
3. `swap` per mutation on the room object. Correct for multiple nodes, but a
   round-trip per comment, and it needs the conditional writes that option 1
   does not.
4. A separate coordination service. Out of scope; it reintroduces the thing
   BYO-storage exists to remove.

Option 1 needs one new guard: a `rooms/<slug>.lock` object holding a process id
and a timestamp, taken on first load and refreshed. A second VPS pointed at the
same bucket must refuse to serve rather than silently interleave writes.

## What changes meaning

- **Quotas become advisory.** `storageLimit` exists to bound *our* bill. On the
  operator's bucket, the ceiling is their decision: keep enforcing
  `config.Storage`, but the BYO defaults should be far looser than the sandbox's,
  and `--quota` should be settable per deployment rather than compiled in.
- **Retention.** `KOMODOC_EXPIRE_SECONDS` sweeps from the Worker's scheduled
  handler; on the VPS it is a ticker goroutine. Where the provider supports
  lifecycle rules, prefer those and say so in the docs.
- **Credentials.** A long-lived secret in the VPS's config is fine for
  self-hosting and is the only mode this spec covers. A hosted "bring your
  bucket to our instance" service is a credential-custody problem and needs
  scoped temporary credentials (STS `AssumeRole`, R2 temporary tokens); it is
  explicitly **out of scope** here.
- **`komodoc destroy`.** Must not delete an operator's bucket. It deletes the
  keys komodoc wrote, under its own prefix, and never the container.

## Key layout

One prefix so komodoc can share a bucket, and so `destroy` has a bounded blast
radius. Default `komodoc/`, settable with `--s3-prefix`:

```
<prefix>/index.json
<prefix>/documents/<slug>/<sha>.html
<prefix>/sources/<slug>
<prefix>/examples/<slug>.json
<prefix>/rooms/<slug>.json
<prefix>/rooms/<slug>.lock
```

Note this moves `serve` from `documents/<slug>/source.txt` to `sources/<slug>`,
adopting the Worker's layout. A one-shot migration on startup: if
`source.txt` exists and `sources/<slug>` does not, copy and remove.

## Configuration

```
komodoc serve --dir ./data                      # unchanged, fsStore
komodoc serve --s3-endpoint https://… \
              --s3-bucket komodoc \
              --s3-region auto \
              --s3-prefix komodoc/ \
              --s3-access-key … --s3-secret-key …   # or KOMODOC_S3_* in the env
```

`--dir` and `--s3-bucket` are mutually exclusive. Credentials come from the
environment by preference; accepting them as flags means they land in the
process table and in shell history, so the flags should warn.

## Bring your own bucket, without us holding the key

The "Credentials" note above rules out hosted BYO-storage because custody of a
long-lived secret is a liability we do not want. There is a way to have the
deployment without the custody: the credential never leaves the reader's
browser, and the shell talks to the bucket directly.

SigV4 is HMAC-SHA256 four times over a canonical request plus a SHA-256 of the
body. WebCrypto does all of it -- the same ~60 lines budgeted for the Go
implementation, in JavaScript. So `blobStore` gains a fourth implementation that
is not Go at all: `browserS3Store` in the shell, the same six methods, issuing
`PUT`, `GET`, `DELETE` and `ListObjectsV2` from the page. In that mode komodoc
serves the shell, auth, and the room socket, and nothing else; the document
bytes never touch us, and neither does the secret.

Three things the bucket must be configured for:

- **CORS**, allowing our origin with `GET, PUT, DELETE, HEAD`, and -- this is
  the one people miss -- `ExposeHeaders: ["ETag"]`. Without the exposed ETag the
  browser cannot read the version it just wrote, and `swap` is gone.
- **A scoped credential.** An R2 API token bound to one bucket, or an IAM user
  whose policy is limited to `arn:aws:s3:::<bucket>/komodoc/*`. A
  browser-resident key with account-wide S3 access is a bad object to bring into
  existence; the blast radius here is entirely a function of how the token was
  minted, so the startup probe should print a copy-pasteable policy alongside
  the CORS JSON it already prints.
- **A CSP** on our own pages with `connect-src` limited to `self` and the S3
  endpoint, so a compromised script has nowhere to send what it steals.

In the page, the key lives in a closure and is never in plaintext
`localStorage`. If it must persist across reloads, encrypt it with a
passphrase-derived key (PBKDF2 or Argon2 to AES-GCM) in IndexedDB, and treat the
passphrase prompt as the cost of the mode.

**The honest caveat**: we serve the JavaScript that holds the key, so "we never
see it" is a claim about our code, not a property of the architecture. A user
who does not want to take that on trust needs a pinned bundle -- SRI, published
hashes, or self-hosting the shell. The docs should say this plainly rather than
implying the guarantee is structural.

### What does not survive the trip

1. **Readers have no credentials.** Documents must therefore be public-read
   under the prefix, or reached by presigned GETs that only the key-holder can
   mint. Public-read is the coherent choice for a publishing tool and costs
   nothing we were not already spending: an unlisted document's slug was always
   the capability. Long-TTL presigned links are the alternative and they expire
   badly.

2. **Comments are written by people who hold no key.** This is the real
   blocker, and it is the room problem in a sharper form: annotation is exactly
   the write that cannot come from the author's browser. Either rooms stay
   authoritative on our side -- so "we store nothing" becomes "we store nothing
   durable except comments" -- or the author's browser flushes the room to their
   bucket as a periodic backup while we remain the writer. The second is cheap
   and honest; either way, option 1 under "Rooms" does not apply to this
   deployment.

3. **`admit` stops being enforcement.** If bytes never transit us we cannot
   check size or count before they land, and a client-side check is advice. It
   is their bucket and their bill, so this is mostly fine -- but drop the check
   rather than ship one that lies.

4. **Rendering, mostly already solved.** `komodoc edit` renders in the browser
   from the WASM build of `src/markdown`, and a save stores what the browser
   produced -- so in the editing path the source never reaches us either. What
   remains is publishing from the CLI, where the source is rendered locally and
   only the HTML travels. Neither path needs a server-side renderer, which is
   what makes this mode possible at all.

## Agents as peers

A separate want, but it lands on the same seams: an LLM agent that edits a
remote document and its annotations interactively, the way a person does.

The good news is that the interface mostly exists already and was not built for
this. The room socket carries two kinds of frame -- annotation mutations
(`comment`, `reply`, `resolve`, `delete`, in `room.message`) and Yjs updates
(`y-update`, with `replace` for a whole-session state). An agent that speaks
that socket is indistinguishable from a browser, and because the source is a
CRDT it gets the hard part free: an agent rewriting a paragraph while a person
types in the next one converges, with no lock, no last-writer-wins, and no
separate edit API to keep in step with the collaborative one. Adding a
REST-shaped `PATCH /documents/<slug>` beside it would be a second way to write
the same text, with different concurrency rules -- exactly the hand-synced
divergence this document exists to remove.

So the work is not a new interface. It is four things the existing one does not
yet do.

**A credential that is not a cookie.** `apply` takes `author` and `isOwner` from
the caller's identity, never from the message, which is right and should stay
that way -- so an agent needs an identity of its own. A bearer token, minted by
the owner, scoped to one document, revocable, presented on the WebSocket
upgrade. The author key derives from the token, so an agent's comments are
attributable to the agent rather than to the person who issued it, and the
reader can label them. Rate limiting already keys on the caller
(`config.RatePerHour`); agent tokens should carry their own budget rather than
share a human's.

**A stated protocol.** `message` is an internal struct today, and every field is
optional with the type deciding which ones matter -- which is the right shape
for an API, but it is not one until it is written down and versioned. Publishing
it is a commitment: the reader and the agent then have to be changed together.

**A save that does not need a browser.** The Yjs session is relay state; the
durable copy is what a save stores. With a human editing, a browser eventually
saves. A headless agent has no such moment, so either it sends an explicit
`save`, or the room flushes the CRDT to `sources/<slug>` when the last peer
leaves -- which is the same debounce the rooms design already calls for, applied
to the source instead of the comments. The second is better: it makes
"disconnect" mean "persisted" for every client, not just for agents.

**Rendering, which is the awkward one.** `komodoc edit` deliberately moved
rendering into the browser: the WASM build of `src/markdown` renders the
preview, a save stores what the browser produced, and the deployment renders
nothing at all. An agent has no browser, so an agent's save has no HTML. Three
ways out, and only one keeps the current property:

1. **The agent's client is the CLI.** `komodoc` already links `src/markdown`;
   let it join a room, apply edits, and render its own save with the same
   package the browser's WASM is built from. Parity holds by construction and
   the deployment still renders nothing. This is the recommendation, and it
   means the agent-facing artifact is a komodoc subcommand (and a small library
   behind it), not an HTTP surface an agent has to reimplement.
2. The server regains a render path used only for agent saves -- reintroducing
   the thing `edit.go` says it removed, and giving two renderers to keep in
   step.
3. Agent saves carry HTML the agent rendered by other means, which is
   unverifiable and lets the source and the document drift apart.

### What this costs the browser-side S3 mode

The two features pull against each other. If the bucket credential lives only in
the reader's browser, then nothing persists unless a browser peer is attached --
a headless agent can edit the live CRDT all night and lose it when it
disconnects. The options are that the agent holds a scoped bucket credential of
its own (defensible for a CLI running on the operator's machine, and no worse
than the credential the VPS holds in the server-side mode), or that headless
agents are simply not supported when the key is browser-resident and the mode is
documented as requiring a human's tab to be open. The first is the honest one;
the second is what happens by accident if nobody decides.

Reads are easier: an agent needs a snapshot -- current source, current
annotations -- before it can do anything useful, and that is a plain `GET` that
does not touch the socket or the CRDT.

## Plan

1. Extract `blobStore` with `fsStore` only. No behaviour change; the existing
   suite (`store` coverage in `serve_test.go`, `ownership_test.go`,
   `quota_test.go`, `hardening_test.go`) is the proof.
2. Same seam in `worker.js`: an object wrapping `env.DOCS`, so every call site
   goes through one place.
3. `s3Store` in Go, plus SigV4, plus the startup probe.
4. Presigned read redirects.
5. Rooms to the bucket, with the lock object.
6. `s3Store` in the Worker, if it is wanted -- it is the least valuable step,
   since a Cloudflare deployment already has R2.

Steps 1 and 2 are worth doing on their own merits: they collapse two hand-synced
storage implementations into two implementations of one stated interface.

The agent work is a separate track and does not wait on any of it -- steps 7 to
9 could be done first:

7. Room-side flush of the CRDT to `sources/<slug>` when the last peer leaves.
   Useful on its own: it makes disconnect mean persisted for browsers too.
8. Scoped bearer tokens for the room socket, with their own author key and rate
   budget, and a snapshot `GET` for source and annotations.
9. A komodoc subcommand that joins a room as a peer, applies edits to the Yjs
   source, posts and resolves annotations, and renders its own saves with
   `src/markdown`.

## Open questions

- Does `admit` stay authoritative when the bucket is not ours? An operator can
  fill their own bucket by other means, and the index would not know. Reconcile
  on startup with a `list`, or accept the drift?
- Should `--single-writer` be the *default* for `serve`, given that it is
  always a single process, with conditional writes as the opt-in?
- Do example annotations belong in the bucket at all, or should they stay
  seeded from the repo at deploy time?
- Is the browser-side mode a separate deployment shape, or the same one with the
  credential moved? If both are supported, `blobStore` has four implementations
  and the room story differs between them.
- Does making `room.message` a public protocol constrain the reader too much?
  The alternative is an agent-only translation layer, which is a second way to
  say the same things.
- Should an agent be able to publish a *new* document, or only edit one it has
  been given a token for? Creation is where quota and ownership are decided.


## What was built

Steps 1 to 5 of the plan, and the open questions answered by making a call
rather than leaving the code ambiguous. Steps 6 to 9 -- an `s3Store` in the
Worker, the browser-side credential, and the agent track -- are not here.

**The seam.** `blobStore` in `src/blob.go`, with the six methods this
document specified, and `fsStore` behind it. `store` keeps the entry cache,
`admit`, the staleness check and `ownedBy`, and lost `filepath` entirely;
`saveLocked` is now a `swap` and can fail with `errConflict`. The existing
suite is the proof it changed nothing: it passes unmodified apart from
constructing a store from a directory rather than being handed one.

**The Worker's half.** `blobs(env)` in `worker.js` wraps `env.DOCS` with
the same six operations, and every access goes through it. The key helpers
(`indexKey`, `documentKey`, `documentPrefix`, `sourceKey`,
`examplesKey`) are stated once on each side, in the same layout.

**S3.** `src/s3.go`: SigV4 by hand, ListObjectsV2, conditional writes via
`If-Match`/`If-None-Match`, presigned GETs, and the startup probe. The
probe writes a scratch key, requires a wrong-version write and a create-only
write over an existing object to be refused, requires the right-version write
to be accepted, and deletes the scratch key. A bucket that fails it refuses to
start without `--single-writer`, and the refusal prints what to do.

**Rooms in the bucket, VPS-authoritative** -- option 1, as recommended. The
in-memory copy stays the source of truth while anyone is connected, so the
existing mutex is still correct and comment latency is unchanged; `save`
writes to `rooms/<slug>.json`. The lock object is there: a second server
takes `rooms/<slug>.lock`, finds it held, and serves that room read-only
rather than interleaving writes. A lock outlives its holder by five minutes,
so a killed server does not need an object deleted by hand.

**Direct reads**, behind `--s3-direct-reads`. Not a bare 302: the response
carries the CSP that confines a document and the agent that makes it annotable,
and a redirect would lose both. Instead the response is a page under a kilobyte
that fetches the document from the presigned URL and becomes it --
`document.write`, so a document's own scripts still run -- and then loads the
agent. The bytes go from the bucket to the browser; what passes through the
server is the envelope.

### The open questions, answered

- **Does `admit` stay authoritative?** Yes, and it stays enforcement rather
  than becoming advice: bytes still transit the server on the way in, so the
  check still means something. Drift from a bucket filled by other means is
  accepted rather than reconciled -- a `list` on every start is a cost paid
  forever for a case that is somebody else editing their own bucket by hand.
- **Should `--single-writer` be the default?** No. It is always true of a
  single `serve`, which is exactly why defaulting to it would make the
  dangerous configuration the quiet one. Conditional writes are the default and
  the assertion is deliberate, printed at startup.
- **Do example annotations belong in the bucket?** They are already there, as
  `examples/<slug>.json`, written by the seed. Nothing changed.
- **Is the browser-side mode a separate shape?** Not answered by code. It is
  not built, and the four-implementation question it raises is still open.
- **Does publishing `room.message` constrain the reader?** Not answered; the
  agent track is untouched.
- **Should an agent create documents?** Not answered, for the same reason.

### What this does not do

- No `s3Store` in the Worker (step 6), which the plan itself calls the least
  valuable: a Cloudflare deployment already has R2.
- No browser-resident credential. The CORS and IAM policies the probe prints
  are what that mode would need, and printing them is useful now regardless.
- No agent tokens, snapshot endpoint, or peer subcommand (steps 7-9).
- No CORS preflight probe. The policy is printed on request rather than
  detected, because a preflight from the server tests the server's origin,
  which is not the origin a browser would use.
