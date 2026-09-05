# Komodoc

Publish an HTML or Markdown document, share its unlisted link, and collect
comments and highlights in real time.

- Highlight passages, suggest edits, and comment on figures
- Multiple people can annotate simultaneously, with live updates
- Publish and manage documents from the web or CLI
- Trivial to deploy: one static binary, on your laptop or on a small server
- Free public sandbox for small, short-lived notebooks
- Allow anonymous comments or require GitHub authentication
- Export annotations as Markdown or W3C JSON-LD

<div class="screenshot-pair">
<figure>
<img src="docs/sandbox.png" alt="Komodoc sandbox landing page with the upload area and document list">
<figcaption>The free sandbox landing page.</figcaption>
</figure>
<figure>
<img src="docs/commenting.png" alt="A document open in Komodoc with highlighted passages and the comments sidebar">
<figcaption>The annotation window, with highlights and threaded comments.</figcaption>
</figure>
</div>

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/vincentarelbundock/komodoc/main/install.sh | sh
```

The installer supports Linux and macOS. Windows binaries are available on the
[releases page](https://github.com/vincentarelbundock/komodoc/releases).

## Web interface: Try it now!

The Komodoc sandbox is a free website where anyone can upload small (<4MB) short-lived (<24hrs) HTML or Markdown files. To upload a document, you will need to log with your Github username:

[Komodoc sandbox](https://komodoc.arelbundock.com)

If you do not want to log in but want to try annotating some documents, you can try one of these live examples:

- [HTML: A Short Style Guide for Quantitative Writing](https://komodoc.arelbundock.com/docs/html-a-short-style-guide-for-quantitative-writing-72wgqautjz)
- [Markdown: What a Regression Table Is Hiding](https://komodoc.arelbundock.com/docs/markdown-what-a-regression-table-is-hiding-c9kqgt7acs)
- [Typst: What a Confidence Interval Does Not Say](https://komodoc.arelbundock.com/docs/typst-what-a-confidence-interval-does-not-say-5vvxv8ebpd)
- [Quarto: What the Bootstrap Actually Resamples](https://komodoc.arelbundock.com/docs/quarto-what-the-bootstrap-actually-resamples-j9iu5cqy7b)
- [Calepin: Newton's Method Is Not Always Your Friend](https://komodoc.arelbundock.com/docs/calepin-newton-s-method-is-not-always-your-friend-2sdp6b6aga)
- [Jupyter: Simpson's Paradox Is Not a Paradox](https://komodoc.arelbundock.com/docs/jupyter-simpson-s-paradox-is-not-a-paradox-b5serei7j7)
- [Marimo: How Far Does a Drunk Walk?](https://komodoc.arelbundock.com/docs/marimo-how-far-does-a-drunk-walk-2c9n8gwd6i)
- [Publication and management console](https://komodoc.arelbundock.com) (requires Github Login)

A published document lives at `/docs/<title>-<suffix>`, where the suffix is
random so the link cannot be guessed from the title. The seeded examples above
are the exception: their suffix is derived from the title rather than drawn at
random, so re-seeding the sandbox leaves these links pointing at the same
documents. That is safe only because an example is public on purpose; every
other document keeps an unguessable address. A link that resolves to nothing
gets a 404 page saying so.

<aside class="callout warning">
<strong>Warning:</strong> Do not publish confidential information on the Komodoc sandbox. Normally, documents are only visible to the person who uploaded them, or to people with the randomly generated and unlisted link. But if you are gathering comments on documents about national security, you should probably <a href="#self-managed-server">host your own instance</a> or find another solution.
</aside>

<br>

The standard web-based workflow is:

1. Open a Komodoc server in a browser, 
2. Sign in with GitHub (if the manager requires it), 
3. Upload an `.html` or `.md` file,
4. Send the (unlisted) link to your readers. 

anyone with the link can read the document. The Komodoc console only lists only the documents you own.

Click on the thumbnails near to top of this page for screenshots of the Komodoc management console and annotation page.


## CLI

Every command executed from the CLI must point to a specific Komodoc server. Typically, users will specify their server with a flag. For example, to make a request against the Komodoc sandbox, a live instance maintained by the developers, use:

```sh
komodoc <COMMAND> --server https://komodoc.arelbundock.com
```

When making repeated calls to the same server, it is convenient to specify the address using an [Environment Variable](#environment-variables). This allows us to omit the `--server` flag:

```sh
export KOMODOC_SERVER="https://komodoc.arelbundock.com"

komodoc <COMMAND>
```

In the examples below, we use the environment variables and omit the flag.

### Authenticate

Sign in once with GitHub using the device flow:

```sh
komodoc login
```

### Publish

Publish an HTML or Markdown document:

```sh
komodoc publish paper.html --title "My Paper"
```

HTML files must be self-contained, with images, styles, and fonts embedded. For
Quarto, render with:

```sh
quarto render paper.qmd --to html -M embed-resources:true
```

### List

List the documents visible to your account. Each row shows a short ID (at
least three characters, and the same width for every document) along with its
date and title. The ID is a prefix of the document's suffix, so a document
you publish again gets a new one; the seeded examples below keep theirs,
because their suffix is derived rather than random:

```sh
komodoc list
```

```
2c9  2026-09-04  Marimo: How Far Does a Drunk Walk?
b5s  2026-09-04  Jupyter: Simpson's Paradox Is Not a Paradox
2sd  2026-09-04  Calepin: Newton's Method Is Not Always Your Friend
j9i  2026-09-04  Quarto: What the Bootstrap Actually Resamples
c9k  2026-09-04  Markdown: What a Regression Table Is Hiding
5vv  2026-09-04  Typst: What a Confidence Interval Does Not Say
72w  2026-09-04  HTML: A Short Style Guide for Quantitative Writing
```

### Comment

Open a listed document in your browser for commenting. The ID is the one `list`
prints (a full slug also works):

```sh
komodoc comment c9k
```

### Edit

A document published from markdown or typst keeps that source, so it can be
edited in the page it is read in: the source on one side, the document as it will be
published on the other, and the comments beside both. Either of the two panes
next to the source folds away. `edit` takes the same ID `comment` does, and
just opens that page:

```sh
komodoc edit c9k
```

The editor is offered to whoever may replace the document, and the document
opens ready to work on. Saving publishes a revision to the same link, so the
comments survive it — as you type, they re-anchor against the edited text, and
one whose passage is gone is marked as needing re-anchoring rather than
quietly dropped.

Several people can edit at once. The source is a CRDT (Yjs), so two people
typing in the same sentence converge without either waiting for the other, and
the toolbar says how many are in the session. The server relays the updates and
remembers them for as long as somebody is editing; it never merges them, and
needs no Yjs of its own. Nothing about a session is persisted, because the
durable copy of a document is the source a save stores — that is what a session
is seeded from when nobody is editing yet.

What is shared is the source. The preview is not: each browser renders what it
now has, so a session costs the deployment no CPU and no bandwidth beyond
relaying a few dozen bytes per keystroke.

A save publishes what the session holds, so everyone in it moves to the new
version without being told they have collided. A version published from outside
the session — from the command line, or a browser that was not in it — is
refused rather than allowed to discard the work: the editor says so, and the
save is rejected by the server, which checks against the index under the same
lock that commits it.

Rendering happens in the browser, by the same compiler that published the
document, built for WebAssembly. So the preview is the document that a save
would store, byte for byte, and an edit made in the browser renders exactly as
one made from the command line. The deployment renders nothing: it stores what
the browser produced, the way it has always stored what the CLI produced —
which is also why a live preview costs it no CPU and no bandwidth.

Two formats, and they are not available in the same places:

| | Published with | Renderer | Over the wire |
|---|---|---|---|
| **Markdown** | `komodoc publish paper.md` | comrak | ~130 KB compressed |
| **Typst** | `komodoc publish paper.typ` | typst | ~13 MB compressed |

Both renderers are the same crate the binary itself renders with, compiled to
WebAssembly. Nothing else has to be installed: publishing a `.typ` file needs
no `typst` binary on your PATH, because the compiler is inside Komodoc, and it
is the same one the editor runs — so a document cannot render one way when it
is published and another way when it is edited.

The typst module is thirty megabytes, typst itself and the fonts it sets
documents in, so it is optional at build time and fetched only by someone who
actually opens a typst document to edit. Both modules are fetched once and
cached for a year. A build without the typst module hosts and annotates typst
documents as usual and says so, rather than offering an editor that could not
save.

Typst's HTML export is still marked experimental upstream, so complex
documents may not survive it intact. Simple ones, including maths, come out as
real HTML with MathML, which is what lets comments anchor into them at all.

The typst renderer is built by `make typst`, which needs a Rust toolchain and
is deliberately not part of `make build`. Without it Komodoc builds and runs
exactly as before, and simply does not offer typst editing.

A document published as HTML has no markdown to reopen, and one published
before its source was kept has none stored; publish it again from its markdown
to make it editable. The list on the landing page marks which is which.

### Storage

By default a server keeps everything in a directory:

```sh
komodoc serve --data ./komodoc-data
```

It can keep it in any S3-compatible bucket instead — R2, AWS, MinIO, Backblaze
— so a small server holds no durable state of its own and the bytes, the bill
and the ownership of the data are yours:

```sh
export KOMODOC_S3_ACCESS_KEY=...
export KOMODOC_S3_SECRET_KEY=...

komodoc serve --s3-endpoint https://<account>.r2.cloudflarestorage.com \
              --s3-bucket komodoc --s3-region auto
```

Credentials come from the environment by preference: a flag is visible to every
process on the machine and lands in your shell history, and komodoc says so if
you pass one.

Everything komodoc writes lives under one prefix (`komodoc/` by default,
`--s3-prefix` to change it), so a bucket can be shared and deleting a
document has a bounded blast radius. It never deletes the bucket, and never
touches a key outside its own prefix.

At startup the bucket is probed. The index is kept correct by conditional
writes, so a bucket that does not support them is refused rather than run on
quietly — pass `--single-writer` to assert that only this process writes
these keys, which is true of a single server, and it will use its own lock
instead. It is printed at startup either way.

Comments live in the bucket too (`rooms/<slug>.json`), written as they are
made. A second server pointed at the same bucket finds the room locked and
serves it read-only rather than interleaving its writes.

With `--s3-direct-reads`, a document's bytes are fetched by the reader's
browser straight from the bucket rather than passing through the server. That
needs a CORS rule on the bucket; the deployment prints the policy to paste.

### Export

Export annotations as readable Markdown. `export` takes the same ID `comment`
does, a short ID from `list`:

```sh
komodoc export c9k --format markdown --out comments.md
```

Without `--format markdown`, Komodoc exports W3C Web Annotation JSON-LD.

### Destroy

Delete one document, including its history and comments. It takes the same ID
`comment` and `export` do -- the short one from `list`, or a full slug -- and
asks you to type the full slug to confirm unless `--yes` is supplied:

```sh
komodoc destroy --document c9k
```

It deletes the document, its history and its comments. Nothing else on the
server is touched.

## Deploy

Komodoc is one static binary with everything compiled into it: the reader, the
renderers, and the server. Run it on your laptop for a quick trial, or on a
small host for something durable.

### Local

Run a public local instance with no GitHub setup at all:

```sh
komodoc serve --port 8081 --publishers YOUR-GITHUB-LOGIN
```

Open <http://localhost:8081>. Everything is stored in `komodoc-data` (see [Storage](#storage)).

### Self-managed server

Run the bundled server on your own host, with `--data` set to a persistent
directory:

```sh
komodoc serve --port 8080 --data /var/lib/komodoc --publishers YOUR-GITHUB-LOGIN
```

To let people sign in, set up a [GitHub app](#github-oauth) for this server's address.

Run the server behind a reverse proxy that terminates HTTPS, and have the proxy send the `X-Forwarded-Proto: https` header. That header is how the server knows its own address is an HTTPS one: without it the session cookie is not marked `Secure`, and uploads and comments are refused because the browser's idea of where the page came from does not match the server's. Plain HTTP is fine on `localhost` and nowhere else.

### Retention

Delete documents automatically after their most recent publication:

```sh
komodoc serve --expire-after 24h
```

For a fixed lifetime from the first upload, use `--expire-from created`. Use
`--expire-after never` to disable expiry. Expired documents are removed by an
hourly pass, and once at startup.

### Storage

`komodoc serve` keeps documents, comments and the session key in the directory
named by `--data` or `KOMODOC_DATA`, `komodoc-data` in the working directory by
default; back it up if the instance holds real work. Point it at a bucket
instead and the server holds nothing of its own — see
[Bring your own bucket](#bring-your-own-bucket).

Five flags bound what a deployment will store:

| Flag | Caps | Default |
| --- | --- | --- |
| `--max-size` | one document | 4 MB |
| `--quota` | everything one publisher holds | 100 MB |
| `--storage` | the whole deployment | 5120 MB |
| `--max-documents` | documents one publisher may hold | 50 |
| `--uploads-per-hour` | uploads one publisher may make in an hour | 30 |

```sh
komodoc serve --max-size 8 --quota 500 --storage 10240
```

Under `--publishers anyone` (see [Rights](#rights)), a browser's quota is tied
to a cookie rather than an account, so clearing cookies gets a new one;
`--storage` is the bound that actually holds under that policy.

### Rights

Two flags say who may do what.
`--publishers` says who may upload documents, and `--commenters` who may
annotate them. Both accept a comma-separated list of GitHub logins:

```sh
komodoc serve --publishers alice,bob --commenters anyone
```

Besides a list, each flag takes two keywords:

| Value | Meaning |
| --- | --- |
| `alice,bob` | only these GitHub accounts |
| `any` | any signed-in GitHub account |
| `anyone` | no sign-in at all |

`--publishers` has no default: the server insists you say who may publish.
`anyone` is allowed, which is what makes the local trial above work without any
GitHub setup; on a host the internet can reach, name the accounts instead.
`--commenters` defaults to `anyone`,
so readers can annotate a document straight from its link; use `any` to
attribute every comment to a GitHub account, or a list to keep a draft among
named reviewers.

### GitHub OAuth

Komodoc signs people in with GitHub, so any server that asks anyone to sign in
needs an OAuth app of its own. A server where both `--publishers` and
`--commenters` are `anyone` never asks, and runs without one.

Create the app at [github.com/settings/developers](https://github.com/settings/developers)
(New OAuth App) with **Device Flow enabled**, which is what `komodoc login`
uses at the terminal. Point its two URLs at the server's own address — the
public HTTPS address it sits behind, with the same `/auth/callback` path:

```text
Homepage URL:               https://docs.example.org
Authorization callback URL: https://docs.example.org/auth/callback
```

Pass the credentials to `serve` through the environment, rather than as flags: an argument is visible in `ps` to every process on the machine,
an environment variable is not.

```sh
export KOMODOC_GITHUB_CLIENT_ID="..."
export KOMODOC_GITHUB_CLIENT_SECRET="..."
```

## Environment variables

[^github-data]: Komodoc requests no GitHub scopes through OAuth. It uses the
GitHub API only to obtain your public login name; it does not collect your email,
repositories, or other profile data.

Flags take precedence over their corresponding environment variables.

| Variable | Purpose |
| --- | --- |
| `KOMODOC_SERVER` | Default server for `login`, `publish`, `list`, `export`, and document deletion |
| `KOMODOC_TOKEN` | GitHub token to use instead of `komodoc login` |
| `KOMODOC_DATA` | Directory `serve` and `seed` use for documents and comments (default `komodoc-data`) |
| `KOMODOC_GITHUB_CLIENT_ID` | GitHub OAuth app client ID |
| `KOMODOC_GITHUB_CLIENT_SECRET` | GitHub OAuth app client secret |
| `KOMODOC_PUBLISHERS` | GitHub accounts allowed to publish |
| `KOMODOC_COMMENTERS` | Who may comment: `anyone`, `any`, or a list of GitHub accounts |
| `KOMODOC_EXPIRE_AFTER` | Automatically delete documents after a duration such as `24h` or `30d` |
| `KOMODOC_EXPIRE_FROM` | Start retention at `updated` (default) or `created` |
| `KOMODOC_VERSION` | Version selected by the installer |
| `KOMODOC_BIN_DIR` | Installation directory selected by the installer |

## Building from source

Komodoc is a Cargo workspace of two crates. `engine` renders markdown and typst
and is built twice — natively into the binary, and to WebAssembly for the
browser — so the editor's preview and the command line's output come from the
same code. `komodoc` is the binary: the server and the command line.

```sh
make wasm     # the markdown renderer for the browser (fast)
make typst    # the typst renderer, ~30 MB (slow, and optional)
make build    # dist/komodoc, with the shell and whichever renderers are built
make test     # rustfmt, clippy and the test suite
```

`make build` needs the markdown module, which `make wasm` builds; both need the
`wasm32-unknown-unknown` target (`rustup target add wasm32-unknown-unknown`).
`make typst` is deliberately separate: it takes a few minutes and adds thirty
megabytes to the binary, and a build without it works exactly as described
above, minus typst editing.
