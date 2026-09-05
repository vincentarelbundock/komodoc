# SPEC: the host program in JavaScript, and Rust reduced to the engine

Status: proposed. Nothing here is built. This supersedes the decision in
`SPEC-rust.md`: the command-line tool and the server move from Rust to
JavaScript, and Rust shrinks to the one crate that has no alternative. What
that spec got right still holds and is listed under "What does not change".
`SPEC-history.md` is written against the Rust server and is re-read against
this one at the end, because it is the next thing to build and this changes
where it is built.

## The problem

The port to Rust removed the Cloudflare Worker, and with it the second
implementation of the rules that `conformance.json` used to keep honest. That
was the right trade at the time: one server, one set of rules, one test
suite. It also left the sandbox on a VPS, with a process to keep alive, a
disk to back up, and a bandwidth bill that grows with readers.

The question that reopens it is whether the service can run on Cloudflare
alone -- static pages, a Worker, a Durable Object per room, an R2 bucket --
with no server of ours anywhere. It can; it did, before the port. The cost
was always the same one: a Worker is JavaScript, so a Rust server means the
rules exist twice.

There is a way to have the Cloudflare deployment and one implementation of
the rules, and it runs the other way from `SPEC-rust.md`. Write the host
program in JavaScript once, and run it in three places: as a Worker on
Cloudflare, as a single executable under Bun for anyone self-hosting, and as
the command line. Keep Rust for the part JavaScript cannot be.

## The decision

The host program -- server and command line, everything that is not the
engine and not the browser -- moves from Rust to JavaScript. The reasons, in
the order they weigh:

1. **The rules should exist once.** Ownership, quotas, retention, the
   reserved example slugs, the stale-save rule, the origin and cookie
   checks: today they live in `config.rs` and the handlers, and the shell is
   handed a copy of `config.rs` as a string at build time. In JavaScript the
   shell, the server and the command line import the same module. There is
   no injection and no second copy, on any platform.
2. **The engine already meets the host halfway.** `engine/` compiles to two
   WebAssembly modules with a plain-export ABI, no network, no package
   resolution, embedded fonts, and a clock supplied by the host. The loader
   in `web/src/lib/renderers.js` is forty lines of standard WebAssembly API
   and runs unchanged under Bun and in a Worker. A JavaScript command line
   renders a publish with the same module the browser previews with, so the
   parity `SPEC-rust.md` wanted from a native build is had from one build.
3. **Yjs is JavaScript.** The reference implementation is the one the
   browser already runs. `SPEC-history.md` needs the server to hold the
   document rather than relay it, and asks for `yrs` to do so; here it is
   `yjs`, the same package, with the same `Y.Text` and the same encoding.
   The server and the browser cannot disagree about a document because they
   run the same code on it.
4. **Cloudflare's shape fits the room.** A room is one document's sockets and
   one document's state, with one writer. That is what a Durable Object is:
   one instance per name, its own storage, an alarm, sockets that survive
   the instance being unloaded. The bucket lock, the in-memory eviction
   question, and the "second server serves read-only" rule all go away,
   because the platform guarantees the thing the lock was approximating.
5. **Cost is bounded by construction.** No egress from R2, no process to
   keep warm, no bandwidth bill for readers. Every publish still passes
   through the Worker, so `admit` still runs before a byte lands, and the
   size and quota ceilings stay enforcement rather than advice.
6. **The browser becomes a host too.** A core that imports from no runtime
   and speaks only the Fetch API runs in a page as readily as in a Worker.
   That is what makes "bring your own bucket" -- the mode `TODO.md` asks
   for, where the reader's browser holds the credential and talks to the
   bucket itself -- a configuration of the same store and the same S3
   adapter rather than a fourth implementation. `SPEC-blobstore.md` costed
   it as sixty lines of SigV4 in JavaScript beside the Go one; here there is
   no "beside".

What it costs, stated plainly: roughly 7,300 lines of Rust outside the
engine are rewritten, and the 4,300 lines of tests that made the last port
safe are ported first and play the same role again. The self-host binary
grows from about 40 MB to something near 100 MB, because a Bun executable
carries a runtime. Two runtimes means two places where WebSockets, crypto
and timers behave slightly differently, which is where the adapters below
earn their keep. And the Rust build stays, for the engine, so a contributor
still needs cargo and a wasm target; what goes is every crate that was not
typst or comrak.

What it does not buy: nothing here changes what a reader sees, what a
document is, or what the command line accepts. What it buys almost for
free, and is taken: the bring-your-own-bucket mode, under "Bring your own
bucket" below. If the port needs to change a
protocol, a layout, a rule or a flag, the port is wrong.

## What does not change

This is the contract the port is tested against. Every item is
language-neutral and must survive byte for byte.

- **The engine crate.** `engine/`, its two features, its ABI (`alloc`,
  `dealloc`, `compile`, `title_of`, `set_today`, `output_ptr`, `ok`), its
  pinned typst version, and the two modules it builds. It gains one thing,
  below, and loses nothing.
- **The browser.** `web/` and its build into `src/shell`: reader, editor,
  agent, anchoring, collaboration, panes. The one change is that
  `__CONFIG__` becomes an import of the rules module.
- **The wire protocols.** Every HTTP route, JSON body and status code; every
  room socket message (`hello`, comments, `y-open`, `y-state`, `y-update`,
  `y-snapshot`, `y-peers`, `y-awareness`, `published`, `editing`); every
  postMessage between shell and document frame; the `docs.` origin for
  documents and why it is a hostname and not a port.
- **The blob layout.** `index.json`, `documents/<slug>/<sha>.html`,
  `sources/<slug>`, `rooms/<slug>.json`, `examples/<slug>.json`,
  `session.key`, under the configured prefix. A JavaScript server pointed at
  a bucket the Rust server wrote must serve it. (`SPEC-history.md` changes
  this layout later; that is its change to make, on the new server.)
- **The rules.** Policies, ownership, quotas, retention, the reserved
  example slugs, the no-fork rule for stale saves, publisher-less documents
  staying publisher-less. The ported tests are what describe them.
- **The command line.** The same subcommands (`login`, `logout`, `publish`,
  `serve`, `list`, `comment`, `edit`, `export`, `seed`, `destroy`), the same
  flags and environment variables including `KOMODOC_S3_*`, and the release
  archive names (`komodoc_<os>_<arch>.tar.gz`), so `install.sh` does not
  change.
- **Sessions and tokens.** The HMAC signing scheme and the key at
  `session.key`, so an upgrade does not sign anyone out. The `__Host-`
  cookie rules and the CSRF and origin checks in `auth.rs` and `origins.rs`,
  ported line for line.

## Shape

```
Cargo.toml              workspace: engine only
engine/                 typst + comrak, to wasm; unchanged location
web/                    the browser, unchanged
src/shell/              built pages and wasm, unchanged
host/                   the JavaScript package
  core/                 rules, store, room, auth, documents, export, seed
  adapters/             blob: fs, s3, r2;  room host: process, durable-object
  workers/              the Worker entry, the Durable Object class, wrangler
  bun/                  the executable entry: serve + command line
  test/                 the ported suite, run against memory and workerd
Makefile                builds wasm, shell, executable; deploys to Cloudflare
```

One package, not three. `core/` never imports from a runtime: not `bun:*`,
not `cloudflare:*`, not `node:fs`. It speaks the Fetch API (`Request`,
`Response`, `crypto.subtle`, `WebSocket` on the browser side) which both
runtimes implement, and takes everything else from an adapter it is handed.
`workers/` and `bun/` are each a few hundred lines that construct the
adapters and hand them to the same `handle(request, env)`.

### The engine crate

Stays as it is, with one addition and one thing rejected.

**Files from the host.** `komodoc publish paper.typ` today reads sibling
files through a directory reader compiled natively; the browser gets no
files at all. A JavaScript command line has only the wasm, so the ABI gains
`add_file(path, bytes)` and `clear_files()`, and the world reads from that
map. This is the "file map" `SPEC-rust.md` planned and did not build; it is
required here rather than optional, and it is what makes `#include` and
`#bibliography` work in the browser as a side effect.

**typst.ts, rejected.** The prebuilt typst WebAssembly packages render pages
as vector graphics into the DOM. The reader anchors comments into text
nodes, which is why the engine uses typst's HTML export and not its layout;
no prebuilt module exposes that export with our page wrapping and our pinned
compiler. Building our own wasm is a `cargo build` with a target flag, and it
stays.

The native build of the engine has no remaining caller once the Rust command
line goes. The crate keeps compiling natively for its own tests, and that is
all.

### The rules, once

`config.rs` becomes `host/core/rules.js`: every limit, the extensions, the
source formats, the motivations, the slug pattern, the cap on tags. The
server imports it. The shell imports it, replacing the `__CONFIG__` string
the build injects today. The command line imports it, so `publish` can
refuse a 5 MB file before uploading it. One module, no build step between
it and any of its readers.

### Storage

The six-method blob interface from `SPEC-blobstore.md` -- read, write with
conditional version, delete, list, and the key helpers -- with three
implementations:

| adapter | where | conditional write |
| --- | --- | --- |
| `fs` | Bun, `--data` directory | rename over a version file, as `blob.rs` does |
| `s3` | Bun, `--s3-*` flags | SigV4 by hand in WebCrypto, `If-Match`; the startup probe from `s3.rs` |
| `r2` | Workers, the bucket binding | `onlyIf: { etagMatches }` on `put` |

The `s3` adapter is the port of `s3.rs`, 580 lines, kept hand-written for
the reason `SPEC-rust.md` gave: the SDK is larger than the program. The `r2`
adapter is the smallest of the three; the old Worker's `blobs(env)` was it.

### The index has one writer

Today the index is `index.json` in the bucket, written under a mutex in one
process with a conditional put behind it. On Cloudflare there is no one
process: every Worker invocation is its own, and a room object taking a
checkpoint would race a Worker admitting a publish. Conditional puts with
retry would work and is what the old Worker did. The cleaner answer is a
single Durable Object named `index`, holding the index in its storage and
answering `admit`, `put`, `remove` and `list` in order. That is exactly the
role the mutex in `store.rs` plays, and it is what "under the same lock that
commits it" means on this platform.

Under Bun the same store class sits behind an in-process lock, as now.
`core/store.js` is written once against an interface that is either.

### Rooms

`room.rs` becomes `core/room.js`: the comment list, the peer set, the
`apply` rules, the rate limit, the Yjs relay. It is a plain class with three
things it asks of its host: storage for its state, a way to be woken later,
and a way to send on a socket. Two hosts:

- **Durable Object.** One instance per slug, `idFromName(slug)`. Sockets
  through the hibernation API, so an idle room costs nothing and a reader
  who leaves a tab open all day holds no memory. State in the object's own
  SQLite storage, written on a short debounce and on every socket close,
  because hibernation may unload the instance between messages and a room
  must come back with nothing lost. The quiet timer is an alarm.
- **Process.** A `Map` of rooms in the Bun server, timers for alarms, the
  blob store for state under `rooms/<slug>.json` as now. One process, so
  the single-writer guarantee holds for free and the bucket lock is not
  needed.

The `y-*` protocol is unchanged for the port. Holding a `Y.Doc` in the room
rather than relaying updates it cannot read is `SPEC-history.md` step 1 and
is not done here, but the room is written so that it is a change to one
class and neither host.

### Auth

`auth.rs` ported: the OAuth web flow, the device flow for the command line,
the HMAC cookie, the visitor cookie, the policies. The client secret is a
Worker secret binding or an environment variable, read through the adapter.
`session.key` is read from the blob store as today, so an existing
deployment's users stay signed in; a Worker may also be given it as a
secret, which saves a bucket read per request.

### The command line

The same executable as the server, as now: `komodoc serve` and `komodoc
publish` in one file with subcommands. `publish` loads the wasm for the
document's format from inside the executable, feeds sibling files through
`add_file`, renders, and posts what the Rust command line posted. `export`,
`list`, `comment`, `edit`, `seed`, `destroy`, `login`, `logout` are HTTP
clients and port directly. `seed` is the one that renders locally, for the
example annotations' anchors, and uses the markdown module.

### The executable

`bun build --compile` produces one static executable per platform with the
shell, the two wasm modules and the code inside it. Linux `x86_64` and
`aarch64`, macOS both, Windows `x86_64`: the five targets `install.sh`
already expects, built on one machine because Bun cross-compiles. No
runtime to install, no `node_modules` on the user's disk. This keeps the
README's first promise: one static binary, on your laptop or on a small
server.

### Cloudflare

The Worker is the same `handle` behind a `fetch` export, plus the two
Durable Object classes, plus a `scheduled` export for retention. Static
assets -- the pages and `markdown.wasm` -- are served by the platform in
front of the Worker. The `docs.` origin is a second route to the same
Worker, which reads `Host` as the server does today.

Two limits shape this, and both should be verified against current
documentation before step 8:

- **`typst.wasm` is 31 MB**, above the per-file limit for static assets as I
  understand it (25 MiB) and far above the Worker script limit. It is served
  from R2 under a digest key with a year-long cache, as the README already
  describes, either through a public bucket hostname or streamed by the
  Worker. Fonts on demand, the engine follow-up `SPEC-rust.md` noted, would
  bring it under the asset limit and is the better fix; it is not part of
  this port.
- **Durable Object socket messages are capped**, at 1 MiB as I understand
  it. A `y-state` for a document whose source is a 4 MB HTML file does not
  fit. The room sends state above a threshold in chunks, and the shell
  reassembles; or the shell fetches it over HTTP. Decided in step 6, once
  measured.

Retention is a cron trigger that sweeps the index, as the old Worker did,
with an R2 lifecycle rule as a backstop for objects the sweep missed.
Secrets are bindings. `komodoc deploy`, which uploaded the old Worker
through Cloudflare's API, is not rebuilt; `wrangler deploy` from the
Makefile does the job, and a self-hoster who wants Cloudflare has a
toolchain already.

### Bring your own bucket

The mode `SPEC-blobstore.md` designed and did not build, and the one line
in `TODO.md`: our hosted instance serves the pages, signs people in and
runs the rooms; the documents live in a bucket the publisher owns, and the
bytes and the credential never pass through us. What made it a separate
shape before was that the store logic lived in Go and the browser would
have needed its own. Here it is the same `core/store.js` and the same `s3`
adapter, constructed in the page instead of in a process. The list of what
is new is short, and none of it duplicates anything:

- **Where the credential lives.** Endpoint, bucket, prefix, key id and
  secret, entered once in the reader's settings. Held in a closure for the
  session; kept across reloads only if the publisher sets a passphrase, as
  AES-GCM under a PBKDF2 key in IndexedDB. Never in plain `localStorage`,
  never sent to us. The credential is the identity for writes: a bucket
  that accepts the signature is the authorisation, and no GitHub sign-in
  is needed to publish. Comments still carry the GitHub or visitor
  identity, as now, because they go through the room.
- **Readers have no credential.** They must not need one. Documents and
  sources under the prefix are public-read, which is the coherent choice
  for a publishing tool whose links were always the capability; the
  publisher's index stays owner-only, under a second prefix the public
  policy does not cover. The startup probe's copy-pasteable policies from
  `s3.rs` are printed by the settings page instead, and the bucket must
  expose `ETag` through CORS or `swap` is gone.
- **How a reader finds the bucket.** A link is `/docs/<slug>` and stays
  so. On first publish the owner's browser registers the document with the
  index object: slug, the bucket's public origin, the prefix. That is one
  small record we hold per document, and the only thing we store about it.
  The reader resolves it, then loads the document through the same
  under-a-kilobyte envelope page `--s3-direct-reads` uses today, which
  fetches from the bucket and becomes the document, so the CSP and the
  agent are unchanged.
- **Comments live with us.** The room is a Durable Object whether the
  document is in our bucket or theirs, and its durable copy is in the
  object's storage. `SPEC-blobstore.md` noted that nothing persists to a
  browser-held bucket unless a key-holder is attached; keeping the small
  part -- comments, kilobytes -- and letting the bucket hold the large
  part -- documents, megabytes -- is the honest split. A later option can
  have the owner's browser mirror `rooms/<slug>.json` into their bucket
  when it is connected; it is not needed to ship.
- **Quotas become advisory.** `admit` runs in the page from the same rules
  module and warns; nothing enforces, because the ceiling on their bucket
  is their decision and their bill.
- **The command line is the headless key-holder.** `komodoc publish` with
  `KOMODOC_S3_*` set writes to the bucket through the same adapter and
  registers the document the same way. This answers the agent-track
  question in `SPEC-blobstore.md`: an agent that must persist without a
  browser is a command-line process with its own scoped credential, and
  the browser mode is not held to support it.
- **A CSP** on our pages whose `connect-src` is `self`, our socket, and the
  bucket endpoint the publisher entered, so a compromised script has
  nowhere else to send the key.

The honest caveat from `SPEC-blobstore.md` stands and goes in the docs: we
serve the JavaScript that holds the key, so "we never see it" is a claim
about our code, not a property of the architecture. A publisher who will not
take that on trust runs the executable, which is the same code.

What this costs the hosted deployment: one record per document, one Durable
Object per room with comments in it, and no document bytes. What it costs
the design: nothing in `core/` changes. The browser is a host that supplies
an `s3` adapter and no room host.

### Tests

The suite in `komodoc/src/tests/` -- hardening, auth, ownership, quota,
serve, edit, blob, s3, export, retention, seed, visitor, assets -- is
ported to `bun test` before any of the code it tests, and runs twice: against
`core/` with in-memory adapters, and against the Worker under `wrangler
dev`, which runs the real Durable Object and R2 code locally. The second run
is what makes two runtimes one codebase rather than two: any divergence
between Bun and workerd shows up as the same test failing in one column.
The headless-browser convergence tests for the editor run against both
servers too.

### Building and releasing

`make build` runs the engine's two wasm builds, `vite build` for the shell,
and `bun build --compile` for each target. `make test` runs `cargo test -p
engine`, `bun test`, the `wrangler dev` column, and the wasm size checks.
`make deploy` runs `wrangler deploy`. Release archives keep their names.
The `komodoc` crate, `cargo-dist` and the release matrix of Rust runners are
deleted; the engine builds on one runner.

## SPEC-history, re-read

The history spec is written against the Rust server and asks for `yrs` in
it. Against this one, every rule holds and the implementation is shorter:

- **The session is the document.** The room holds a `Y.Doc` from `yjs`,
  applies relayed updates, answers `y-open` from it, and applies a restore
  or a command-line publish as a diff into `Y.Text`. These are the standard
  patterns of the library the browser already uses.
- **The quiet timer and the last-editor rule** are a Durable Object alarm
  and a `webSocketClose` event; under Bun, a timer and a close handler.
- **`sessions/<slug>`** is the Durable Object's own storage rather than a
  bucket object, because it is hot and has one writer; checkpoints and the
  manifest go to R2, because they are cold and anyone who may read the
  document may read them.
- **Eviction**, an open question there, is answered by hibernation: the
  platform unloads an idle room and reloads it from storage on the next
  message.
- **The word-level diff** is asked for twice there, in Rust for the command
  line and in JavaScript for the browser. Here it is one module, used by the
  command line, the reader and the room's restore.
- **Server-side rendering, never stored**, the open question that would
  serve a reader without WebAssembly, is possible under Bun for both formats
  and in a Worker for markdown only, since the typst module exceeds the
  script limit. The spec leaves it out of the first version, and nothing
  else in it depends on it.

The size cap on socket messages, above, is the one constraint the history
spec must be told about: a checkpoint's bytes and a `y-state` for a large
document travel over HTTP, not the socket.

## Cost, since it is the priority

The sandbox's bill is bounded today by `--quota`, the 4 MB cap, and hourly
expiry. All three survive unchanged: `admit` runs in the Worker before any
`put`. What changes is the shape of the rest:

- **R2**: no egress, a cent and a half per gigabyte-month, and a publish is
  a handful of writes at a fraction of a cent per thousand. Reads of a
  document by many readers cost nothing beyond the request.
- **Workers**: the free tier covers a hundred thousand requests a day. The
  paid plan is needed for Durable Objects and costs five dollars a month,
  less than a VPS.
- **Durable Objects**: charged by request and by duration while awake.
  Hibernated sockets are free. The one recurring cost is storage writes for
  live state, which a two-second debounce keeps to one per typing session
  per two seconds.

Expiry deletes a document's objects, its room, and later its history, so
the bound on the sandbox is the same bound as today.

## Steps

Each step leaves `main` shippable, with the Rust server still the one
deployed until step 9. Nothing is merged with a red test.

1. **Files from the host.** `add_file` and `clear_files` in the engine ABI;
   the directory reader in the native build replaced by the same map.
   Sibling imports work in the browser at the end of this step, on the Rust
   server. Worth doing regardless of the rest.
2. **Tests first.** `host/test/`: the harness from `tests/harness.rs`, the
   fake bucket, and every test file above ported to `bun test` and failing.
   This is the longest step and the one that makes the rest safe.
3. **Rules and storage.** `rules.js`; the blob interface with `fs`, `s3` and
   `r2`; the store with the in-process index. The blob and s3 tests go
   green. A JavaScript server reads a bucket the Rust server wrote.
4. **Auth, policies, origins, http.** Sessions verify against a
   `session.key` the Rust server wrote. The auth, hardening and visitor
   tests go green.
5. **Documents and comments.** Routes, rooms without collaboration, seeding,
   examples, retention, export. The serve, ownership, quota, edit, export,
   seed and assets tests go green. The shell works against the Bun server
   for reading and commenting.
6. **Rooms, both hosts.** The room class, the process host, the Durable
   Object host with hibernation, the `y-*` protocol, the message-size
   decision. The `wrangler dev` column goes green. The headless-browser tests
   run against both.
7. **The command line and the executable.** Every subcommand; `publish`
   through the wasm; `bun build --compile` on five targets; `install.sh`
   verified against a snapshot release.
8. **Cloudflare.** Static assets, `typst.wasm` from R2, the index object,
   the cron trigger, secrets, `make deploy`. The sandbox runs on it beside
   the VPS for a week.
9. **Delete.** The `komodoc` crate, `cargo-dist`, the Rust release matrix,
   the `__CONFIG__` injection. The workspace is `engine/`. README updated.
   The VPS is switched off.
10. **Bring your own bucket.** The settings page and the credential's
    keeping, the public-read and CORS policies it prints, the document
    record in the index object, the reader resolving it, the CSP. Tests:
    the store runs in a headless browser against the fake bucket; a reader
    with no credential reads; a publisher with one publishes; the key is
    absent from every storage the page can reach.

Then `SPEC-history.md`, on the room from step 6. Step 10 depends on nothing
after 8 and can be taken before 9 if it is wanted sooner.

## Risks

- **Security-relevant drift**, again. Origin checks, cookie attributes,
  ownership, the reserved-slug gate, timing-safe comparison of signatures.
  Mitigated by porting the hardening tests before the code, as last time,
  and by nothing else, which is why step 2 is not optional.
- **Hibernation losing state.** A room that keeps anything only in memory
  loses it when the platform unloads the instance. Rule: nothing a room
  would miss lives only in memory; every mutation is written before the
  handler returns or on a debounce short enough that the loss is a second
  of typing. Tested by killing the instance under `wrangler dev`.
- **Two runtimes.** Bun and workerd differ in WebSocket close semantics,
  timer behaviour, and which Web APIs are complete. Bounded by the two test
  columns and by `core/` importing from neither.
- **The two size limits** above. Both have a way out; both must be measured
  in step 6 and step 8 rather than assumed.
- **Bun as a dependency.** Its compile-to-executable is younger than
  `cargo-dist`, and Windows is its least exercised target. Verified on all
  five targets in step 7 before the Rust release path is removed.
- **The key in the page.** A browser-resident bucket credential is only as
  safe as the token's scope and our script's integrity. Bounded by the CSP,
  by printing a one-bucket one-prefix policy rather than accepting an
  account-wide key silently, by the passphrase for anything persisted, and
  by saying in the docs what is trusted. Not bounded by anything else.
- **Executable size** surprising a self-hoster. Documented; a hundred
  megabytes is the price of no runtime to install, and the `typst.wasm`
  inside it is most of it.

## Non-goals

- Changing any protocol, layout, rule or flag.
- Holding a `Y.Doc` on the server. That is `SPEC-history.md`.
- Fonts on demand. Engine follow-up; it is what would let `typst.wasm` be a
  static asset.
- Mirroring comments into a publisher's own bucket. Later option, above.
- TypeScript. `web/` is JavaScript and so is this; the message protocols get
  JSDoc types where the bugs are.
- A router or server framework. `handle(request, env)` and a path match is
  what `server.rs` is and what this is.
- Rebuilding `komodoc deploy`. `wrangler` does it.

## Decisions taken here, so they need not be reopened

- Our own wasm from `engine/`, not typst.ts. The HTML export with text nodes
  is the reason the project works.
- One package with adapters, not one package per runtime. The tests run the
  same code in both columns or the point is lost.
- The index in a Durable Object, not conditional puts with retry. Single
  writer is the property the mutex had; keep it.
- Hibernation from the start, not added later. A room written without it
  keeps state in memory that it must not.
- `session.key` stays in the bucket. Nobody is signed out by the upgrade.
- Hand-written SigV4 kept, in WebCrypto. Same reason as before.
- Bun, not Node, for the executable and the tests; and Bun's own server, not
  a Node compatibility layer, so there is one WebSocket implementation on
  that side.
- In bring-your-own-bucket, comments stay with us and documents go to the
  bucket. Persisting to a bucket only a browser can sign for is not a
  promise worth making.
- A document's bucket is registered with the index object, not carried in
  the link. Links stay short and stable, and the record is a few bytes.
- The Rust host code is deleted at the end of step 9, not kept for
  reference. Git has it.
