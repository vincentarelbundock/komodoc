# SPEC: komodoc in Rust

Status: proposed, not started. Nothing in this document is built. The current
Go implementation on `live-markdown-editor` is complete and tested, and lands
first; the port begins on a fresh branch after it.

## The decision

The host program -- the command-line tool and the server, everything that is
not forced to be JavaScript -- moves from Go to Rust. The reasons, in the
order they weigh:

1. **Typst is Rust, and komodoc is becoming a typst tool.** Compiling typst,
   resolving packages, finding fonts, citing bibliographies, autocompleting
   in an editor: every one of these is a crate. A Rust host imports them. A Go
   host runs one of them (the compiler) through a WebAssembly boundary and
   re-implements the others (packages, fonts) in Go, as a second copy of code
   that already exists.
2. **Yjs outside a browser is Rust.** The agent track needs a program that is
   not a browser to read and write the shared document, and the server
   benefits from doing so too. `yrs` is the maintained port. Go has nothing.
3. **One language for the non-browser half.** Today the repository is Go, Rust
   and JavaScript. Afterwards it is Rust and JavaScript, and the JavaScript is
   the part that cannot be anything else.

What it costs, stated plainly: a two-second build becomes ten to twenty seconds
incremental; a one-dependency binary becomes a few-hundred-crate binary; the
release pipeline changes tools; and roughly twelve thousand lines, half of
them tests, are rewritten. The tests are what make the rewrite safe, which is
why they go first.

What it does not buy: the server and the Cloudflare Worker still implement
the same rules twice. That duplication is Go-or-Rust versus JavaScript, not
Go versus Rust, and no choice here removes it. The conformance fixtures
remain the thing that keeps them honest.

## What does not change

This list is the contract the port is tested against. Every item is
language-neutral and must survive byte for byte.

- **The browser.** `src/shell/*` -- reader, editor, agent frame, anchoring,
  collaboration, pane layout -- is untouched by the port. The web-side changes
  in "The browser" below are a separate track that would happen under Go too.
- **The Worker.** `src/worker/worker.js` and `room.js` are untouched. The
  deploy path (upload the script through Cloudflare's API, create the bucket)
  is reimplemented in Rust but uploads the same files.
- **The wire protocols.** Every HTTP route, JSON body and status code; every
  room socket message (`hello`, comments, `y-open`, `y-state`, `y-update`,
  `y-snapshot`, `y-peers`, `published`, `editing`); every postMessage between
  shell and document frame.
- **The blob layout.** `index.json`, `documents/<slug>/<sha>.html`,
  `sources/<slug>`, `rooms/<slug>.json`, `rooms/<slug>.lock`,
  `examples/<slug>.json`, `session.key`, under the configured prefix. A Rust
  server pointed at a bucket a Go server wrote must serve it, and vice versa.
- **The rules.** Policies, ownership, quotas, retention, the reserved example
  slugs, the no-fork rule for stale saves, publisher-less documents staying
  publisher-less. These are what `src/conformance.json` and the hardening,
  ownership, quota and auth tests describe.
- **The command line.** The same subcommands, flags and environment variables,
  including the storage flags and `KOMODOC_S3_*`. The release archive names
  (`komodoc_<os>_<arch>.tar.gz`) stay, so `install.sh` does not change.
- **Session cookies and tokens.** The signing scheme and key stored at
  `session.key`, so a running deployment's users are not logged out by an
  upgrade.

## Shape

A cargo workspace at the repository root:

```
Cargo.toml              workspace
engine/                 the typst engine crate (today src/typst)
komodoc/                the binary: cli + server
src/shell/              browser, unchanged location
src/worker/             Worker, unchanged location
src/conformance.json    fixtures, unchanged
```

Two crates, deliberately. `engine` is the part that is compiled twice -- to
`wasm32-unknown-unknown` for the browser and natively into the binary -- and
keeping it a separate crate is what guarantees both builds are the same
compiler with the same world. `komodoc` depends on it and on nothing that
cannot cross-compile.

### The engine crate

What `src/typst/src/lib.rs` is today, grown into what the browser and the CLI
both need:

- **A world with files.** The `World` trait has six methods, and only two of
  them read anything: `source` and `file`. The engine gains a `Files` map from
  virtual path to bytes, filled by the host before each compile. That single
  change is what makes `#bibliography("refs.bib")`, custom `.csl` styles,
  `#include`, `#image` and user fonts work; the citation engine is already
  compiled in and needs nothing else.
- **Packages through the host.** The engine does not do networking. When a
  `FileId` names a package the map lacks, the compile returns "needs package
  `@preview/name:version`"; the host fetches and unpacks it (JavaScript in the
  browser, `typst-kit` natively), adds the files, and compiles again. The
  loop is the host's; the engine stays pure.
- **`today` from the host.** A timestamp passed in, so `datetime.today()`
  works and a compile is still a pure function of its inputs.
- **Same ABI, more exports.** The raw C ABI stays (`alloc`, `dealloc`,
  `compile`, `output_ptr`, `ok`) and gains `add_file(path, bytes)`,
  `clear_files()`, and later `complete(offset)` and `tokens()` from
  `typst-ide` and `typst-syntax` for the editor. No bindgen, for the reasons
  the current file states.
- **Fonts on demand, later.** Twelve of the module's fourteen compressed
  megabytes are fonts. A later step ships the font book without the faces and
  fetches a face the first time a document uses it, as typst's own web app
  does. Not part of the port; noted because it is the one egress lever that
  matters and it belongs in this crate.

Native, the engine is used by `komodoc publish paper.typ` with a world built
from `typst-kit`: the embedded faces plus the system's fonts, and the package
cache in the standard location the `typst` binary already uses. The `typst`
binary dependency and `warnIfTypstDiffers` go away; the compiler is inside
komodoc and is the one the browser runs.

### The komodoc crate

A file-per-concern layout mirroring the Go one, so the port is mechanical and
the tests map one to one.

| Go | lines | Rust module | notes |
| --- | --- | --- | --- |
| `serve.go` | 1344 | `server/{routes,documents,rooms_http,socket}.rs` | split; it was too big in Go too |
| `room.go` | 742 | `room.rs` | plus `yrs` (see below) |
| `s3.go` | 496 | `blob/s3.rs` | hand-written SigV4 ports directly; no SDK |
| `store.go` | 446 | `store.rs` | lock becomes `tokio::sync::Mutex` |
| `auth.go`, `login.go` | 623 | `auth.rs`, `login.rs` | HMAC sessions, GitHub device flow |
| `blob.go`, `storage.go` | 516 | `blob/{mod,fs}.rs`, `storage.rs` | |
| `seed.go`, `seed_examples.go` | 705 | `seed.rs` | |
| `publish.go`, `export.go`, `edit.go` | 577 | `cli/{publish,export,edit}.rs` | |
| `deploy.go`, `destroy.go`, `cloudflare.go` | 561 | `cli/deploy.rs`, `cloudflare.rs` | |
| `main.go`, `config.go` | 542 | `main.rs`, `config.rs` | |
| `assets.go`, `http.go`, `origins.go` | 465 | `assets.rs`, `http.rs`, `origins.rs` | `include_dir!` for shell and worker |
| `websocket.go` | 222 | -- | `axum`'s WebSocket support |
| `markdown/`, `markdown_shim.go`, `wasm/` | 181 | `markdown.rs` | `comrak`; see "Markdown" |
| `typst.go` | 137 | `typst.rs` | native engine + `typst-kit` |
| `retention.go`, `version.go` | 55 | `retention.rs`, build-time version | |

Dependencies, each with the reason it is there and not something smaller:

| crate | for | why this one |
| --- | --- | --- |
| `tokio`, `axum` | server, WebSockets | the standard pair; WebSockets built in, no hand-rolled framing |
| `reqwest` with `rustls` | GitHub, Cloudflare, S3, packages | no OpenSSL, so every target cross-compiles |
| `serde`, `serde_json` | every JSON body | |
| `sha2`, `hmac`, `base64`, `hex` | digests, SigV4, sessions | what `crypto/*` was |
| `include_dir` | embedding shell and worker | what `go:embed` was |
| `comrak` | markdown | CommonMark + GFM, small in wasm |
| `typst`, `typst-html`, `typst-kit`, `typst-assets` | the engine, native world | pinned together, as now |
| `yrs` | the shared document | the Yjs port |
| `clap` | flags | hand-rolling `flag` is not worth it; `clap` with derive is one file |
| `time` | retention, dates | |

Nothing for logging beyond `eprintln!`, nothing for tracing, no ORM, no
templating: the Go code has none of these and the Rust code does not need
them either.

### Async and locks: the one thing that needs care

Go let a handler hold a lock while calling S3. Rust will not let an ordinary
lock cross an `await`, and this is the only place the port is not mechanical.

Rules:

- `store` and `roomSet` use `tokio::sync::Mutex`. The critical section in
  `store.put` -- admit, check `base_sha`, `swap` the index -- holds it across
  the S3 call, exactly as the Go mutex did, because that is the correctness
  property: two saves cannot interleave between the check and the write.
- A room's peer list is a `std::sync::Mutex<HashMap<PeerId, Sender>>`, never
  held across an `await`: fan-out clones the senders under the lock, releases
  it, then writes. Sends are `tokio::sync::mpsc` so a slow socket cannot block
  the room.
- Nothing else locks.

### The server as a participant in the shared document

Today the Go server relays Yjs updates it cannot read. With `yrs` a room holds
a `Doc`:

- `y-open` returns the state vector and the encoded state, not a replayed
  log. Late joiners receive one compact update.
- Incoming `y-update` is applied to the `Doc` and rebroadcast. The log and
  `editLogMax` snapshot dance go away; the Worker keeps its own until it
  adopts Yjs in the Durable Object, which is a follow-up and not this spec.
- When the last editor leaves, the room writes the text of the `Doc` to
  `sources/<slug>` and the encoded state to `rooms/<slug>.y`, then drops the
  `Doc`. The next `y-open` reloads it. A session survives everyone leaving,
  which is the SPEC-blobstore step 7 that Go could not do.
- The wire messages are unchanged, so the browser does not know which server
  it is talking to. This is what makes it possible to port the server without
  touching `collab.js`.

Rendering at save time stays the browser's job in the editor. The server
renders only in `publish` (native engine) and, later, when an agent saves
through the API without HTML -- which the native engine makes possible and the
Go server could not offer.

### Markdown

`goldmark` is Go. The Rust renderer is `comrak`, and its output is not byte
for byte the same. Consequences, and how each is handled:

- **Existing documents are unaffected.** Rendered HTML is immutable and
  addressed by digest; nothing re-renders until the next save.
- **The next save re-renders differently.** Comments re-anchor by quote, which
  is what they do on every edit anyway. Heading ids, emphasis and lists render
  the same; tables and footnotes may differ in whitespace. The example
  documents are re-seeded and checked by eye once.
- **The browser module shrinks.** The markdown wasm goes from 4.9 MB to a few
  hundred kilobytes, because it no longer carries the Go runtime. The
  `wasm_exec.js` shim is deleted. `TestRendererIsSmallEnough` gets a much
  tighter bound.
- **One renderer, still.** The same `comrak` configuration compiled into the
  binary and into the browser module, with the byte-identical test kept.

### The browser

Separate from the port and not blocked on it, but decided here because it is
the other half of "infrastructure":

- **Bun** for installing packages, bundling, and running the fixture tests.
  One binary; replaces Node in `make test`. The source stays standard ES
  modules so the bundler is replaceable.
- **CodeMirror 6** replaces the `<textarea>`, with `y-codemirror.next` for the
  Yjs binding. That gives shared cursors and names (the awareness track),
  syntax highlighting from `typst-syntax` tokens, and a place to attach
  `typst-ide` completions.
- **TypeScript incrementally.** New files and the editor in TypeScript;
  existing files converted only when edited. The message protocols get types
  first, because that is where the bugs are.
- **Built by the Makefile**, not committed. A toolchain is already required
  for the wasm; bundled output stays out of git.
- **No framework.** Three panes and a list of cards do not need one.
- **The Worker stays plain JavaScript**, unbundled, with no dependencies, as
  now.

### Building and releasing

- `make build` runs `cargo build --release` for the host and `cargo build
  --target wasm32-unknown-unknown --release -p engine` for the browser, plus
  `bun build` for the shell. `make test` runs `cargo test`, the fixture tests
  under Bun, and the wasm size and parity checks.
- Release by `cargo-dist`: Linux `x86_64` and `aarch64` on `musl` for static
  binaries, macOS both architectures, Windows `x86_64`. This is three runners
  where goreleaser used one; the archive names are configured to match what
  `install.sh` expects.
- The binary grows: roughly 12 MB today to roughly 40 MB with the engine and
  embedded fonts, the same size as the `typst` binary it replaces.
- Incremental builds link a large binary; `mold` or `lld` is configured in
  `.cargo/config.toml` to keep the loop under ten seconds.

## Steps

Each step leaves `main` shippable. Nothing is merged with a red fixture test.

1. **Land Go.** Merge `live-markdown-editor`. Add `make typst` to the release
   hooks. Deploy the sandbox. This is the baseline the port is measured
   against.
2. **The engine crate, under Go.** Move `src/typst` to `engine/`, add the file
   map, host-driven packages and `today`, and the new exports. Wire
   `typst.js` to feed files and fetch packages. Bibliographies work in the
   browser at the end of this step, on the Go server. This step is identical
   under both plans and is the one the roadmap wants most.
3. **The browser toolchain.** Bun, CodeMirror, the Yjs binding, TypeScript
   for the editor. Awareness ships here. Also identical under both plans.
4. **Tests first.** Create the `komodoc` crate with nothing but a test
   harness: an in-process server, `post`/`get` helpers matching the Go ones,
   the fake bucket, and every test in `hardening_test.go`, `auth_test.go`,
   `ownership_test.go`, `quota_test.go`, `serve_test.go`, `edit_test.go`,
   `blob_test.go`, `s3_serve_test.go`, `export_test.go`, `retention_test.go`,
   `seed_test.go`, `visitor_test.go` ported and failing. The fixture runner
   from `conformance_test.go` runs against the Rust server. This is the
   longest step and the one that makes the rest safe.
5. **Storage and store.** `blob/{fs,s3}.rs`, `store.rs`, `storage.rs`. The
   blob and S3 tests go green. A Rust binary can read and write a bucket a Go
   server wrote.
6. **Auth, policies, origins, http.** Sessions verify against a `session.key`
   the Go server wrote. The auth and hardening tests go green.
7. **Documents and comments.** Routes, rooms without collaboration, seeding,
   examples, retention, export. The serve, ownership, quota, edit and
   conformance tests go green. The shell works against the Rust server for
   reading and commenting.
8. **Collaboration with `yrs`.** The socket, `y-*` messages, the room as a
   `Doc`, flush on last leave, reload on open. The headless-browser
   convergence tests from this branch run against it.
9. **The command line.** `publish` with the native engine and `typst-kit`,
   `deploy`, `destroy`, `login`, `export`, `list`, `comment`, `edit`, `seed`.
   The `typst` binary dependency is removed.
10. **Markdown to `comrak`**, the small browser module, re-seeded examples.
11. **Release.** `cargo-dist`, `install.sh` verified against a snapshot
    release on all five targets, the Go code deleted, `go.mod` deleted,
    README updated.

Steps 2 and 3 are worth doing regardless of whether 4 through 11 ever happen.
Steps 5 through 8 can each be checked by pointing the existing shell and the
existing Worker fixtures at the new server; there is no moment where the port
must be trusted on faith.

## Risks

- **Security-relevant drift.** Origin checks, cookie attributes, the
  ownership rules, the reserved-slug gate. Mitigated by porting the hardening
  tests before the code they test, and by the fixtures. Not mitigated by
  anything else, which is why step 4 is not optional.
- **The lock across S3.** Described above. A mistake here is a lost document
  under contention, and `TestASecondWriterCannotClobberTheIndex` is the test
  that catches it.
- **WebSocket behaviour.** The Go implementation is hand-rolled and has been
  tested against browsers for months; `axum`'s is more correct but different
  in edge cases (fragmentation, close codes). The headless-browser tests
  cover the paths that matter.
- **`comrak` rendering differences** surprising a user on their next save.
  Bounded: comments re-anchor, and the diff is visible in the preview before
  saving.
- **Build time discouraging small fixes.** `mold`, a `dev` profile with
  `opt-level = 1` for dependencies, and keeping the engine a separate crate
  so a server change never recompiles typst.
- **Cross-compiling with `typst-kit`.** It reads system fonts through
  `fontdb`, which is pure Rust; no C dependencies enter the tree. Verified in
  step 9 on all five targets before anything else depends on it.

## Non-goals

- Changing any protocol, layout, rule or flag. If the port needs to, the port
  is wrong.
- Yjs in the Worker's Durable Object. Worth doing; separate spec.
- Fonts on demand. Engine follow-up; noted above.
- LaTeX. Still no.
- A framework, a dev server, or a Cloudflare emulator on the web side.

## Decisions taken here, so they need not be reopened

- `axum` over `actix` or `hyper` directly: the WebSocket support and the
  tower ecosystem, and it is what the async ecosystem has settled on.
- `comrak` over a goldmark-compatible renderer that does not exist. The
  output difference is accepted.
- `clap` over hand-rolled flags: the subcommand tree is thirteen deep and
  growing.
- Hand-written SigV4 kept, not replaced by an AWS SDK: it is 500 tested
  lines, and the SDK is larger than the rest of the program.
- Two crates, not one and not five.
- The Go code is deleted at the end of step 11, not kept "for reference".
  Git has it.
