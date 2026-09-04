# Komodoc

Publish an HTML or Markdown document, share its unlisted link, and collect
comments and highlights in real time.

- Highlight passages, suggest edits, and comment on figures
- Multiple people can annotate simultaneously, with live updates
- Publish and manage documents from the web or CLI
- Trivial to deploy: host locally with the bundled server, or on Cloudflare
  Workers and R2
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

[Komodoc sandbox](https://komodoc.vincentarelbundock.workers.dev)

If you do not want to log in but want to try annotating some documents, you can try one of these live examples:

- [HTML: A Short Style Guide for Quantitative Writing](https://komodoc.vincentarelbundock.workers.dev/docs/html-a-short-style-guide-for-quantitative-writing)
- [Markdown: What a Regression Table Is Hiding](https://komodoc.vincentarelbundock.workers.dev/docs/markdown-what-a-regression-table-is-hiding)
- [Quarto: What the Bootstrap Actually Resamples](https://komodoc.vincentarelbundock.workers.dev/docs/quarto-what-the-bootstrap-actually-resamples)
- [Calepin: Newton's Method Is Not Always Your Friend](https://komodoc.vincentarelbundock.workers.dev/docs/calepin-newton-s-method-is-not-always-your-friend)
- [Jupyter: Simpson's Paradox Is Not a Paradox](https://komodoc.vincentarelbundock.workers.dev/docs/jupyter-simpson-s-paradox-is-not-a-paradox)
- [Marimo: How Far Does a Drunk Walk?](https://komodoc.vincentarelbundock.workers.dev/docs/marimo-how-far-does-a-drunk-walk)
- [Publication and management console](https://komodoc.vincentarelbundock.workers.dev) (requires Github Login)

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
komodoc <COMMAND> --server https://komodo.vincentarelbundock.workers.dev
```

When making repeated calls to the same server, it is convenient to specify the address using an [Environment Variable](#environment-variables). This allows us to omit the `--server` flag:

```sh
export KOMODOC_SERVER="https://komodoc.vincentarelbundock.workers.dev"

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

List the documents visible to your account. Each row shows the shortest prefix
that uniquely identifies the document, its date, and its title:

```sh
komodoc list
```

```
i  2026-09-03  Marimo: How Far Does a Drunk Walk?
b  2026-09-03  Jupyter: Simpson's Paradox Is Not a Paradox
w  2026-09-03  Calepin: Newton's Method Is Not Always Your Friend
r  2026-09-03  Quarto: What the Bootstrap Actually Resamples
5  2026-09-03  Markdown: What a Regression Table Is Hiding
y  2026-09-03  HTML: A Short Style Guide for Quantitative Writing
```

### Comment

Open a listed document in your browser for commenting. The ID can be the short
prefix printed by `list` (or the full slug):

```sh
komodoc comment 5
```

### Export

Export annotations as readable Markdown:

```sh
komodoc export DOCUMENT-SLUG --format markdown --out comments.md
```

Without `--format markdown`, Komodoc exports W3C Web Annotation JSON-LD.

### Destroy

Delete one document, including its history and comments. The command asks for
confirmation unless `--yes` is supplied:

```sh
komodoc destroy --document DOCUMENT-SLUG
```

To remove an entire Cloudflare deployment instead, use
`komodoc destroy --service --label my-docs`.

## Deploy

Komodoc can run locally for a quick trial, as a self-managed server on your own host, or on Cloudflare Workers and R2.

### Local

Run a public local instance without Cloudflare or GitHub setup:

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

### Cloudflare

Komodoc can also run on Cloudflare Workers and R2. Before deploying:

Cloudflare R2 includes 10 GB of storage per month for free and does not charge
egress fees. You may be charged if your storage exceeds 10 GB; see
[R2 pricing](https://developers.cloudflare.com/r2/pricing/).

1. In the Cloudflare dashboard, enable R2 and choose a `workers.dev` subdomain.
2. Create a [Cloudflare API token](https://dash.cloudflare.com/profile/api-tokens)
   with `Workers Scripts: Edit`, `Workers R2 Storage: Edit`, and
   `Account Settings: Read` permissions.
3. Create a [GitHub OAuth app](#github-oauth) for the deployment's address.

Deploy it:

```sh
export CLOUDFLARE_API_TOKEN="..."
export KOMODOC_GITHUB_CLIENT_ID="..."
export KOMODOC_GITHUB_CLIENT_SECRET="..."

komodoc deploy --label my-docs --publishers YOUR-GITHUB-LOGIN
```

The label is the first part of the URL, so this server is
`https://my-docs.YOUR-SUBDOMAIN.workers.dev`. Cloudflare runs the cleanup
schedule; the `komodoc` program does not need to remain running. A label
itself ending in `-docs` is refused: `<label>-docs` is where the deployment
hosts documents, and a label already ending that way would collide with it.

### Retention

Delete documents automatically after their most recent publication:

```sh
komodoc serve --expire-after 24h
```

For a fixed lifetime from the first upload, use `--expire-from created`. Use
`--expire-after never` to disable expiry. The same options apply to Cloudflare
deployments:

```sh
komodoc deploy --label my-docs --expire-after 24h
```

### Storage

`komodoc serve` keeps documents, comments and the session key in the directory
named by `--data` or `KOMODOC_DATA`, `komodoc-data` in the working directory by
default; back it up if the instance holds real work. A Cloudflare deployment
stores the same things in an R2 bucket named after its label, so
`--label my-docs` uses the `my-docs` bucket.

Five flags bound what a deployment will store, on `komodoc deploy` and
`komodoc serve` alike:

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

Two flags say who may do what, on both `komodoc deploy` and `komodoc serve`.
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

`--publishers` has no default: both commands insist you say who may publish.
`komodoc deploy` additionally refuses `anyone`, which would let the internet
fill your bucket; `komodoc serve` allows it, which is what makes the local
trial above work without any GitHub setup. `--commenters` defaults to `anyone`,
so readers can annotate a document straight from its link; use `any` to
attribute every comment to a GitHub account, or a list to keep a draft among
named reviewers.

### GitHub OAuth

Komodoc signs people in with GitHub, so any server that asks anyone to sign in
needs an OAuth app of its own. A server where both `--publishers` and
`--commenters` are `anyone` never asks, and runs without one.

Create the app at [github.com/settings/developers](https://github.com/settings/developers)
(New OAuth App) with **Device Flow enabled**, which is what `komodoc login`
uses at the terminal. Point its two URLs at the server's own address: for a
Cloudflare deployment labelled `my-docs`,

```text
Homepage URL:               https://my-docs.YOUR-SUBDOMAIN.workers.dev
Authorization callback URL: https://my-docs.YOUR-SUBDOMAIN.workers.dev/auth/callback
```

and for a self-managed server, the public HTTPS address it sits behind, with
the same `/auth/callback` path.

Pass the credentials to `deploy` or `serve` through the environment, rather
than as flags: an argument is visible in `ps` to every process on the machine,
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
| `CLOUDFLARE_API_TOKEN` | Cloudflare credentials for deploying or removing a service |
| `CLOUDFLARE_ACCOUNT_ID` | Cloudflare account to use when the token can access more than one |
| `KOMODOC_SERVER` | Default server for `login`, `publish`, `list`, `export`, and document deletion |
| `KOMODOC_TOKEN` | GitHub token to use instead of `komodoc login` |
| `KOMODOC_LABEL` | Default deployment label |
| `KOMODOC_DATA` | Directory `serve` and `seed` use for documents and comments (default `komodoc-data`) |
| `KOMODOC_GITHUB_CLIENT_ID` | GitHub OAuth app client ID |
| `KOMODOC_GITHUB_CLIENT_SECRET` | GitHub OAuth app client secret |
| `KOMODOC_PUBLISHERS` | GitHub accounts allowed to publish |
| `KOMODOC_COMMENTERS` | Who may comment: `anyone`, `any`, or a list of GitHub accounts |
| `KOMODOC_EXPIRE_AFTER` | Automatically delete documents after a duration such as `24h` or `30d` |
| `KOMODOC_EXPIRE_FROM` | Start retention at `updated` (default) or `created` |
| `KOMODOC_VERSION` | Version selected by the installer |
| `KOMODOC_BIN_DIR` | Installation directory selected by the installer |
