//! The service: the routes the shell and the command line talk to, the socket
//! a room's readers hang on, and the document origin that serves the bytes.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::extract::connect_info::ConnectInfo;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequest, FromRequestParts, Multipart};
use axum::http::{header, HeaderMap, HeaderValue, Method, Request, Response, StatusCode};
use axum::response::IntoResponse;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use http_body_util::Limited;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

use crate::assets::{renderers, ShellFile};
use crate::auth::{
    cookie_name, now_unix, random_token, read_session, read_visitor, sign_session, sign_visitor,
    GithubApp, Identity, Policy, TokenCache, SESSION_COOKIE, SESSION_MAX_AGE, STATE_COOKIE,
    VISITOR_COOKIE,
};
use crate::blob::document_key;
use crate::config::Configuration;
use crate::origins::{
    cross_site_refusal, cross_site_refused, header as header_of, ws_origin_refused, Arrival,
};
use crate::render::{is_markdown, render_markdown_document, title_from_markdown};
use crate::room::{Message as RoomMessage, Outgoing, Room, RoomSet};
use crate::store::{digest_of, random_suffix, slugify, IndexEntry, Publication, PutError, Store};
use crate::util::clean;

/// How much room a multipart upload gets beyond the document itself for part
/// headers, the title, and the slug.
const MULTIPART_SLACK: usize = 1 << 20;

pub struct Server {
    pub store: Store,
    pub rooms: RoomSet,
    /// Whether a document's bytes are fetched by the reader's browser straight
    /// from the bucket. Only possible when the bucket can presign, and only
    /// useful when it allows this origin to read it.
    pub direct_reads: bool,
    pub shell: HashMap<String, ShellFile>,
    pub app: GithubApp,
    pub key: Vec<u8>,
    pub tokens: TokenCache,
    pub config: Arc<Configuration>,
    pub publishers: Policy,
    pub commenters: Policy,
    sockets: AtomicU64,
}

/// What an authorized write is attributed to: the owner key a document's
/// publisher field is compared against (a lowercased GitHub login, a visitor:
/// key, or "" for neither), and the GitHub numeric account id when the caller
/// is signed in.
#[derive(Clone, Debug, Default)]
pub struct Caller {
    pub key: String,
    pub id: String,
}

/// Keeps a browser's key from ever colliding with a GitHub login, which
/// cannot contain a colon.
pub const VISITOR_PREFIX: &str = "visitor:";

type Reply = Response<Body>;

impl Server {
    #[allow(clippy::too_many_arguments)] // a server is made of exactly these
    pub fn new(
        store: Store,
        rooms: RoomSet,
        shell: HashMap<String, ShellFile>,
        app: GithubApp,
        key: Vec<u8>,
        config: Arc<Configuration>,
        publishers: Policy,
        commenters: Policy,
    ) -> Server {
        Server {
            store,
            rooms,
            direct_reads: false,
            shell,
            app,
            key,
            tokens: TokenCache::new(),
            config,
            publishers,
            commenters,
            sockets: AtomicU64::new(1),
        }
    }

    pub fn router(self: Arc<Server>) -> Router {
        Router::new().fallback(handle).with_state(self)
    }

    pub fn renderers(&self) -> Vec<String> {
        renderers()
    }

    pub async fn delete_document(&self, slug: &str) -> Result<usize, String> {
        self.rooms.purge(slug).await;
        self.store.remove(slug).await
    }

    /// Removes every document older than `retention` seconds, measured from
    /// `from`. Returns how many went.
    pub async fn delete_expired(&self, now: i64, retention: i64, from: &str) -> usize {
        let mut removed = 0;
        let cutoff = now - retention;
        for entry in self.store.list().await {
            if let Some(stamp) = entry.expiry_time(from) {
                if stamp <= cutoff {
                    // The janitor runs unattended: a document whose index entry
                    // could not be rewritten is left for the next pass.
                    if let Err(err) = self.delete_document(&entry.slug).await {
                        eprintln!("could not expire {}: {err}", entry.slug);
                        continue;
                    }
                    removed += 1;
                }
            }
        }
        removed
    }

    /// Identifies the caller: a browser by its session cookie, the CLI by the
    /// GitHub token it sends as a bearer. Neither is required; the anonymous
    /// identity simply means nobody is signed in. A bearer is verified against
    /// GitHub's check-token endpoint (cached), and is never trusted at all
    /// when this deployment has no OAuth app configured to verify it against.
    pub async fn whoami(&self, headers: &HeaderMap, arrival: &Arrival) -> Identity {
        if let Some(bearer) = header_of(headers, "authorization")
            .and_then(|h| h.strip_prefix("Bearer ").map(str::to_string))
        {
            if !self.app.configured() {
                return Identity::anonymous();
            }
            let app = self.app.clone();
            return self
                .tokens
                .verify(
                    move |token| async move { app.check_token(&token).await },
                    &bearer,
                )
                .await;
        }
        match cookie(headers, &cookie_name(arrival.is_https(), SESSION_COOKIE)) {
            Some(value) => read_session(&self.key, &value),
            None => Identity::anonymous(),
        }
    }

    /// The key a caller's uploads belong to. A signed-in caller is their GitHub
    /// login. Where publishing needs no account there is still someone on the
    /// other end, so an anonymous caller is named by the visitor cookie the
    /// shell handed their browser: not an identity, but enough that one
    /// visitor's uploads are not another's to list, replace or delete. A
    /// caller with neither -- the CLI publishing to a deployment open to
    /// everyone -- owns nothing, and their uploads stay shared.
    pub fn owner(&self, headers: &HeaderMap, arrival: &Arrival, id: &Identity) -> String {
        if id.is_signed_in() {
            return id.login.to_lowercase();
        }
        if let Some(value) = cookie(headers, &cookie_name(arrival.is_https(), VISITOR_COOKIE)) {
            let token = read_visitor(&self.key, &value);
            if !token.is_empty() {
                return format!("{VISITOR_PREFIX}{token}");
            }
        }
        String::new()
    }

    /// The key a comment or reply is attributed to: the signed-in account, or
    /// a digest of the visitor cookie so that key can decide who may delete a
    /// comment without turning the raw cookie -- which also names the caller's
    /// uploads -- into something a comment payload carries around.
    pub fn comment_author(&self, headers: &HeaderMap, arrival: &Arrival, id: &Identity) -> String {
        if id.is_signed_in() {
            return format!("github:{}", id.login.to_lowercase());
        }
        if let Some(value) = cookie(headers, &cookie_name(arrival.is_https(), VISITOR_COOKIE)) {
            let token = read_visitor(&self.key, &value);
            if !token.is_empty() {
                return format!("visitor:{}", hex::encode(Sha256::digest(token.as_bytes())));
            }
        }
        String::new()
    }

    /// Answers the request itself when the caller may not publish, and
    /// otherwise returns the key and id that own whatever that caller uploads.
    // The error is a whole response, which is the point: a refusal says what
    // it refused and why, and boxing it would cost an allocation on every
    // authorized request to save one on the rare refusal.
    #[allow(clippy::result_large_err)]
    async fn publisher(&self, headers: &HeaderMap, arrival: &Arrival) -> Result<Caller, Reply> {
        let id = self.whoami(headers, arrival).await;
        if self.publishers.allows(&id.login) {
            return Ok(Caller {
                key: self.owner(headers, arrival, &id),
                id: id.id,
            });
        }
        if !id.is_signed_in() {
            return Err(write_json(
                401,
                &json!({"error": "sign in with GitHub to publish"}),
            ));
        }
        Err(write_json(
            403,
            &json!({"error": format!("@{} may not publish here; this deployment allows {}", id.login, self.publishers.describe())}),
        ))
    }

    /// Narrows a listing to what one caller should see: the reserved examples,
    /// the documents that predate ownership, and their own uploads.
    pub fn visible(&self, entries: Vec<IndexEntry>, who: &Caller) -> Vec<IndexEntry> {
        entries
            .into_iter()
            .filter(|entry| entry.example || entry.owned_by(&who.key, &who.id))
            .collect()
    }

    /// Enforces the comment policy, then hands the message to the room. When
    /// commenting needs a GitHub account, the name on the comment is the
    /// verified login rather than whatever the client typed.
    async fn apply_from(
        &self,
        room: &Room,
        mut incoming: RoomMessage,
        address: &str,
        id: &Identity,
        author: &str,
        is_owner: bool,
    ) -> (Value, bool) {
        if !self.commenters.allows(&id.login) {
            let reason = if id.is_signed_in() {
                format!(
                    "@{} may not comment here; this deployment allows {}",
                    id.login,
                    self.commenters.describe()
                )
            } else {
                "sign in with GitHub to comment".to_string()
            };
            return (
                json!({"type": "error", "message": reason, "temp_id": incoming.temp_id}),
                false,
            );
        }
        // A signed-in commenter is named by their account, whether or not
        // signing in was required. Only anonymous readers type a name.
        if id.is_signed_in() {
            incoming.creator = id.login.clone();
        }
        room.apply(incoming, address, author, is_owner).await
    }
}

/* -------------------------------------------------------------- routing */

/// A parsed request path, in the shapes the routes below look for.
fn segments(path: &str) -> Vec<&str> {
    path.trim_start_matches('/').split('/').collect()
}

fn is_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

async fn handle(
    axum::extract::State(server): axum::extract::State<Arc<Server>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request<Body>,
) -> Reply {
    let arrival = Arrival::from_headers(request.headers());
    let path = request.uri().path().to_string();
    let method = request.method().clone();
    let parts = segments(&path);

    // --- the document origin -------------------------------------------
    // Requests arriving on docs.<host> get documents and the in-frame agent,
    // and nothing else: no shell, no API, no session. That is the whole point
    // of the separate hostname.
    if arrival.is_docs_host() {
        if let ["raw", slug, file] = parts[..] {
            if let Some(digest) = file.strip_suffix(".html") {
                return server.serve_document(&arrival, slug, digest).await;
            }
        }
        if path == "/agent.js" {
            if let Some(asset) = server.shell.get("/agent.js") {
                let mut response = Response::new(Body::from(asset.body.clone()));
                set(&mut response, "content-type", asset.kind);
                privacy_headers(&mut response);
                set(&mut response, "cache-control", "public, max-age=300");
                return response;
            }
        }
        return plain(404, "not found");
    }

    // A document asked for on the reader's own host is sent to the other one,
    // so it is never served somewhere it could reach the session.
    if let ["raw", _slug, file] = parts[..] {
        if file.strip_suffix(".html").is_some_and(is_sha) {
            return redirect(&format!("{}{}", arrival.docs_origin(), path));
        }
    }

    // --- signing in ------------------------------------------------------
    if path.starts_with("/auth/")
        || path == "/api/me"
        || path == "/api/auth/config"
        || path == "/api/config"
    {
        if let Some(response) = server
            .handle_auth(
                &method,
                &path,
                request.headers(),
                request.uri().query(),
                &arrival,
            )
            .await
        {
            return response;
        }
    }

    // --- live comment channel --------------------------------------------
    if let ["ws", slug] = parts[..] {
        if !server.valid_slug(slug) {
            return plain(400, "bad slug");
        }
        return server
            .clone()
            .handle_socket(request, peer, &arrival, slug)
            .await;
    }

    // Stable, shareable URL: redirect to whichever version is current, on the
    // origin that serves documents.
    if let ["raw", slug] = parts[..] {
        return match server.store.get(slug).await {
            Some(entry) => redirect(&format!(
                "{}/raw/{}/{}.html",
                arrival.docs_origin(),
                entry.slug,
                entry.sha
            )),
            None => plain(404, "not found"),
        };
    }

    // --- api ---------------------------------------------------------------
    if path == "/api/documents" && method == Method::POST {
        return server.handle_upload(request, &arrival).await;
    }

    // Listing is the one thing a link-holder must not be able to do: knowing
    // one document must not reveal the others, so it takes a publisher.
    if path == "/api/list" && (method == Method::POST || method == Method::GET) {
        if cross_site_refused(request.headers(), &arrival) {
            return write_json(403, &cross_site_refusal());
        }
        let who = match server.publisher(request.headers(), &arrival).await {
            Ok(who) => who,
            Err(response) => return response,
        };
        let documents = server.visible(server.store.list().await, &who);
        return write_json(200, &json!({"documents": documents}));
    }

    if let ["api", "documents", slug, "delete"] = parts[..] {
        if method == Method::POST {
            return server
                .handle_delete(request.headers(), &arrival, slug)
                .await;
        }
    }

    // The editable source of a document, for whoever may replace it. Only the
    // publisher can act on it, so only the publisher is shown it.
    if let ["api", "documents", slug, "source"] = parts[..] {
        if method == Method::GET {
            return server
                .handle_source(request.headers(), &arrival, slug)
                .await;
        }
    }

    if let ["api", "documents", slug] = parts[..] {
        if method == Method::GET {
            let Some(entry) = server.store.get(slug).await else {
                return write_json(404, &json!({"error": "not found"}));
            };
            let id = server.whoami(request.headers(), &arrival).await;
            let (total, open) = server.rooms.get(slug).await.counts().await;
            let owned = entry.owned_by(&server.owner(request.headers(), &arrival, &id), &id.id);
            return write_json(
                200,
                &json!({
                    "slug": entry.slug, "title": entry.title, "sha": entry.sha,
                    "created_at": entry.created_at, "updated_at": entry.updated_at,
                    "comment_count": total, "open_count": open,
                    // What the document was written in, when it kept its source:
                    // the reader offers an editor for a document it can render
                    // again.
                    "source_format": entry.source_format,
                    // Which of those this deployment can render again, and so
                    // offer an editor for.
                    "renderers": server.renderers(),
                    // Whether this caller may replace the document, which is what
                    // an editor does on save. Here that is the same question as
                    // owning it.
                    "can_edit": owned,
                    // Where the reader should frame this document from, and the
                    // only origin it will accept messages from.
                    "docs_origin": arrival.docs_origin(),
                    // Whether this caller may delete anyone's comment here, per
                    // rule G.
                    "can_moderate": owned,
                }),
            );
        }
    }

    // REST fallbacks, used when the socket is unavailable.
    if let ["api", "documents", slug, "comments"] = parts[..] {
        return server.handle_comments(request, peer, &arrival, slug).await;
    }

    // --- the shell -------------------------------------------------------
    let mut page = path.clone();
    if !server.shell.contains_key(&page) {
        if let ["docs", slug] = parts[..] {
            if server.store.get(slug).await.is_none() {
                // Serving the reader here would answer a dead link with 200
                // and an empty page, which reads as the reader being broken.
                return server.not_found(request.headers());
            }
            page = "/reader.html".to_string();
        } else if path == "/" {
            page = "/index.html".to_string();
        } else if path == "/documentation" {
            page = "/documentation.html".to_string();
        }
    }
    if let Some(asset) = server.shell.get(&page) {
        let mut response = write_asset(asset);
        server.issue_visitor(request.headers(), &arrival, asset, &mut response);
        return response;
    }
    server.not_found(request.headers())
}

impl Server {
    fn valid_slug(&self, slug: &str) -> bool {
        slug_pattern(&self.config).is_match(slug)
    }

    /// Answers a browser asking for a page with the 404 page, and anything
    /// else -- a fetch, a script, an image -- with the plain line it can
    /// actually use. Both carry the 404 status; only the shape differs.
    fn not_found(&self, headers: &HeaderMap) -> Reply {
        let accepts_html = header_of(headers, "accept").is_some_and(|a| a.contains("text/html"));
        match self.shell.get("/404.html") {
            Some(asset) if accepts_html => {
                let mut response = Response::new(Body::from(asset.body.clone()));
                *response.status_mut() = StatusCode::NOT_FOUND;
                set(&mut response, "content-type", asset.kind);
                // A link that is dead now may resolve after the next publish,
                // so this answer is never the one a cache should keep.
                set(&mut response, "cache-control", "no-store");
                response
            }
            _ => plain(404, "not found"),
        }
    }

    async fn handle_socket(
        self: Arc<Server>,
        request: Request<Body>,
        peer: SocketAddr,
        arrival: &Arrival,
        slug: &str,
    ) -> Reply {
        // Browsers always send Origin on a WebSocket handshake and cannot be
        // made to attach a custom header to one, so this is rule A's WebSocket
        // variant: Origin alone, checked only when present.
        if ws_origin_refused(request.headers(), arrival) {
            return plain(403, "cross-site request refused");
        }
        // A room belongs to a document. Without this, any invented slug would
        // conjure one, and since the rate limiter counts per room, a new slug
        // per comment would also mean no rate limit at all.
        let Some(entry) = self.store.get(slug).await else {
            return plain(404, "not found");
        };
        let headers = request.headers().clone();
        let id = self.whoami(&headers, arrival).await;
        let author = self.comment_author(&headers, arrival, &id);
        let is_owner = entry.owned_by(&self.owner(&headers, arrival, &id), &id.id);
        let address = client_address(peer, &headers);

        let (mut parts, _body) = request.into_parts();
        let upgrade = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
            Ok(upgrade) => upgrade,
            Err(_) => return plain(400, "expected a websocket upgrade"),
        };
        let room = self.rooms.get(slug).await;
        let server = self.clone();
        upgrade
            .max_message_size(1 << 20)
            .on_upgrade(move |socket| async move {
                server
                    .run_socket(socket, room, address, id, author, is_owner)
                    .await;
            })
            .into_response()
    }

    async fn run_socket(
        &self,
        socket: WebSocket,
        room: Arc<Room>,
        address: String,
        id: Identity,
        author: String,
        is_owner: bool,
    ) {
        let (mut sink, mut stream) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Outgoing>();
        let socket_id = self.sockets.fetch_add(1, Ordering::Relaxed);
        room.attach(socket_id, address.clone(), tx.clone()).await;

        // One task writes, so a broadcast from another connection never
        // interleaves with a reply to this one.
        let writer = tokio::spawn(async move {
            while let Some(outgoing) = rx.recv().await {
                let result = match outgoing {
                    Outgoing::Text(text) => sink.send(WsMessage::Text(text.into())).await,
                    Outgoing::Close(reason) => {
                        let _ = sink
                            .send(WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                                code: 1000,
                                reason: reason.into(),
                            })))
                            .await;
                        break;
                    }
                };
                if result.is_err() {
                    break;
                }
            }
        });

        let hello =
            json!({"type": "hello", "comments": room.snapshot_for(&author, is_owner).await});
        let _ = tx.send(Outgoing::Text(hello.to_string()));

        while let Some(Ok(frame)) = stream.next().await {
            let raw = match frame {
                WsMessage::Text(text) => text.to_string(),
                WsMessage::Close(_) => break,
                _ => continue,
            };
            let Ok(incoming) = serde_json::from_str::<RoomMessage>(&raw) else {
                continue;
            };

            // Editing is relayed rather than applied: the updates are a CRDT's,
            // and this end neither reads nor merges them. Only somebody who may
            // replace the document may change its source, which is the same
            // rule the save itself is under.
            if incoming.kind.starts_with("y-") {
                if !is_owner {
                    continue;
                }
                match incoming.kind.as_str() {
                    "y-open" => {
                        let (updates, seed) = room.editing_state().await;
                        let payload = json!({
                            "type": "y-state", "updates": updates, "seed": seed,
                            "count": room.editors().await,
                        });
                        if tx.send(Outgoing::Text(payload.to_string())).is_err() {
                            break;
                        }
                        room.broadcast(&json!({"type": "y-peers", "count": room.editors().await}))
                            .await;
                    }
                    // Where everyone's caret is, and what they are called.
                    // Relayed and not remembered: it describes who is here
                    // now, so it is worth nothing to whoever arrives next, and
                    // a session that kept it would be keeping a list of ghosts.
                    "y-awareness" => {
                        if incoming.update.is_empty() {
                            continue;
                        }
                        room.broadcast_except(
                            Some(socket_id),
                            &json!({"type": "y-awareness", "update": incoming.update}),
                        )
                        .await;
                    }
                    "y-update" => {
                        if incoming.update.is_empty() {
                            continue;
                        }
                        let want_snapshot = room
                            .editing(incoming.update.clone(), incoming.replace)
                            .await;
                        // Straight on to everyone else. The sender already has it.
                        room.broadcast_except(
                            Some(socket_id),
                            &json!({"type": "y-update", "update": incoming.update}),
                        )
                        .await;
                        if want_snapshot
                            && tx
                                .send(Outgoing::Text(json!({"type": "y-snapshot"}).to_string()))
                                .is_err()
                        {
                            break;
                        }
                    }
                    _ => {}
                }
                continue;
            }

            let (result, ok) = self
                .apply_from(&room, incoming, &address, &id, &author, is_owner)
                .await;
            if !ok {
                if tx.send(Outgoing::Text(result.to_string())).is_err() {
                    break;
                }
                continue;
            }
            room.broadcast(&result).await;
        }

        room.detach(socket_id).await;
        // The last person editing has gone, so the session goes with them.
        room.end_editing().await;
        room.broadcast(&json!({"type": "y-peers", "count": room.editors().await}))
            .await;
        let _ = tx.send(Outgoing::Close(""));
        let _ = writer.await;
    }

    async fn handle_comments(
        &self,
        request: Request<Body>,
        peer: SocketAddr,
        arrival: &Arrival,
        slug: &str,
    ) -> Reply {
        if !self.valid_slug(slug) {
            return write_json(400, &json!({"error": "bad slug"}));
        }
        let Some(entry) = self.store.get(slug).await else {
            return write_json(404, &json!({"error": "not found"}));
        };
        let room = self.rooms.get(slug).await;
        let headers = request.headers().clone();
        let id = self.whoami(&headers, arrival).await;
        let author = self.comment_author(&headers, arrival, &id);
        let is_owner = entry.owned_by(&self.owner(&headers, arrival, &id), &id.id);

        match *request.method() {
            Method::GET => write_json(
                200,
                &json!({"comments": room.snapshot_for(&author, is_owner).await}),
            ),
            Method::POST => {
                if cross_site_refused(&headers, arrival) {
                    return write_json(403, &cross_site_refusal());
                }
                let Ok(body) = to_bytes(request.into_body(), 1 << 20).await else {
                    return write_json(400, &json!({"error": "bad request"}));
                };
                let Ok(incoming) = serde_json::from_slice::<RoomMessage>(&body) else {
                    return write_json(400, &json!({"error": "bad request"}));
                };
                let address = client_address(peer, &headers);
                let (result, ok) = self
                    .apply_from(&room, incoming, &address, &id, &author, is_owner)
                    .await;
                if ok {
                    room.broadcast(&result).await;
                    return write_json(200, &result);
                }
                write_json(400, &result)
            }
            _ => plain(405, "method not allowed"),
        }
    }

    async fn handle_upload(&self, request: Request<Body>, arrival: &Arrival) -> Reply {
        if cross_site_refused(request.headers(), arrival) {
            return write_json(403, &cross_site_refusal());
        }
        // Checked before the body is read, so an unauthorised upload costs
        // nothing.
        let who = match self.publisher(request.headers(), arrival).await {
            Ok(who) => who,
            Err(response) => return response,
        };
        let parsed = match self.read_upload(request).await {
            Ok(parsed) => parsed,
            Err(response) => return response,
        };

        let mut base = slugify(&parsed.slug, &self.config);
        if base.is_empty() {
            base = slugify(&parsed.title, &self.config);
        }
        if base.is_empty() {
            return write_json(400, &json!({"error": "could not derive a slug"}));
        }
        // An exact slug that already exists is a replacement of that document,
        // and keeps its URL and its comments. Anything else is a new document,
        // and gets a random suffix so the link cannot be guessed from the
        // title. Someone else's document is not yours to replace, and guessing
        // its slug should not even tell you it is there: a title that collides
        // with another publisher's document simply becomes a new document of
        // your own.
        let existing = self.store.get(&base).await;
        let mine = existing
            .as_ref()
            .is_some_and(|e| e.owned_by(&who.key, &who.id));
        let key = if mine {
            base.clone()
        } else {
            format!("{base}-{}", random_suffix(&self.config))
        };
        // An edit is about one document. Publishing a file that collides with
        // someone else's title becomes a new document of your own, which is
        // right for publishing and wrong here: a save that quietly forked
        // would leave the editor showing a document nobody else can see, and
        // the one it was opened from untouched. Refused instead.
        if !parsed.base_sha.is_empty() {
            if existing.is_none() {
                return write_json(
                    409,
                    &json!({"error": "the document you were editing is gone"}),
                );
            }
            if !mine {
                return write_json(
                    409,
                    &json!({"error": "this document is no longer yours to replace"}),
                );
            }
        }

        let digest = digest_of(&parsed.html);
        let entry = match self
            .store
            .put(Publication {
                slug: key.clone(),
                title: parsed.title,
                digest,
                html: parsed.html,
                source: parsed.source,
                source_format: parsed.source_format,
                owner: who.key,
                owner_id: who.id,
                base_sha: parsed.base_sha,
            })
            .await
        {
            Ok(entry) => entry,
            Err(PutError::Quota { status, message }) => {
                return write_json(status, &json!({"error": message}))
            }
            // Someone published while this caller was editing. Nothing is
            // written: the editor is told, and decides what to do about it.
            Err(PutError::Stale) => {
                return write_json(409, &json!({"error": PutError::stale_message()}))
            }
            Err(PutError::Storage(_)) => {
                return write_json(500, &json!({"error": "could not store the document"}))
            }
        };
        // Comments survive the replacement; they re-anchor in the reader.
        // Everyone with the document open is told a new version exists, over
        // the same socket their comments arrive on.
        self.rooms
            .get(&key)
            .await
            .broadcast(&json!({"type": "published", "sha": entry.sha, "title": entry.title}))
            .await;
        write_json(
            201,
            &json!({
                "slug": entry.slug, "title": entry.title, "sha": entry.sha,
                "created_at": entry.created_at, "updated_at": entry.updated_at,
                "url": format!("/docs/{}", entry.slug),
            }),
        )
    }

    /// Parses a publish request's body, in either format it may arrive as, and
    /// applies the checks common to both: a title and some HTML are present,
    /// and the HTML is not over the size ceiling. It answers the request
    /// itself on any problem, so `handle_upload` only has to decide where to
    /// store what comes back.
    #[allow(clippy::result_large_err)] // as publisher: the error is a response
    async fn read_upload(&self, request: Request<Body>) -> Result<Upload, Reply> {
        let max_html = self.config.max_html;
        let content_type = header_of(request.headers(), "content-type").unwrap_or_default();
        let (mut title, mut slug, mut html) = (String::new(), String::new(), String::new());
        let (mut source, mut source_format, mut base_sha) =
            (String::new(), String::new(), String::new());

        if content_type.contains("multipart/form-data") {
            // The whole request is bounded, not just the document: without
            // this an oversized body would be read in full before the HTML
            // limit below is even consulted. The slack covers the part headers
            // and the other fields.
            let ceiling = max_html + MULTIPART_SLACK;
            let declared = header_of(request.headers(), "content-length")
                .and_then(|v| v.parse::<usize>().ok());
            let (parts, body) = request.into_parts();
            let limited = Request::from_parts(parts, Body::new(Limited::new(body, ceiling)));
            let mut multipart = match Multipart::from_request(limited, &()).await {
                Ok(multipart) => multipart,
                Err(_) => return Err(write_json(400, &json!({"error": "bad upload"}))),
            };
            let mut filename = String::new();
            loop {
                let field = match multipart.next_field().await {
                    Ok(Some(field)) => field,
                    Ok(None) => break,
                    Err(_) => {
                        if declared.is_some_and(|n| n > ceiling) {
                            return Err(write_json(
                                413,
                                &json!({"error": "that upload is too large"}),
                            ));
                        }
                        return Err(write_json(400, &json!({"error": "bad upload"})));
                    }
                };
                let name = field.name().unwrap_or_default().to_string();
                match name.as_str() {
                    "title" => title = field.text().await.unwrap_or_default(),
                    "slug" => slug = field.text().await.unwrap_or_default(),
                    "file" => {
                        filename = field.file_name().unwrap_or_default().to_string();
                        let bytes = match field.bytes().await {
                            Ok(bytes) => bytes,
                            Err(_) => {
                                if declared.is_some_and(|n| n > ceiling) {
                                    return Err(write_json(
                                        413,
                                        &json!({"error": "that upload is too large"}),
                                    ));
                                }
                                return Err(write_json(400, &json!({"error": "bad upload"})));
                            }
                        };
                        html = String::from_utf8_lossy(&bytes[..bytes.len().min(max_html + 1)])
                            .to_string();
                    }
                    _ => {}
                }
            }
            // Markdown dropped on the page is rendered here, so what gets
            // stored is HTML like everything else.
            if !filename.is_empty() && is_markdown(&filename) {
                if title.trim().is_empty() {
                    title = title_from_markdown(&html);
                }
                let rendered = render_markdown_document(&html, title.trim());
                source = html;
                source_format = "markdown".to_string();
                html = rendered;
            }
        } else {
            // JSON escaping can inflate the document, so the body is allowed to
            // be larger than the document limit; the real check is on the
            // decoded html below. Refusing early keeps a huge body from being
            // read at all, and says why rather than failing to parse.
            let ceiling = max_html * 2 + 1024;
            if let Some(length) =
                header_of(request.headers(), "content-length").and_then(|v| v.parse::<usize>().ok())
            {
                if length > ceiling {
                    return Err(write_json(413, &json!({"error": "document too large"})));
                }
            }
            #[derive(Deserialize, Default)]
            struct Body_ {
                #[serde(default)]
                title: String,
                #[serde(default)]
                slug: String,
                #[serde(default)]
                html: String,
                #[serde(default)]
                source: String,
                #[serde(default)]
                source_format: String,
                #[serde(default)]
                base_sha: String,
            }
            let Ok(bytes) = to_bytes(request.into_body(), ceiling).await else {
                return Err(write_json(413, &json!({"error": "document too large"})));
            };
            let Ok(body) = serde_json::from_slice::<Body_>(&bytes) else {
                return Err(write_json(400, &json!({"error": "bad request"})));
            };
            title = body.title;
            slug = body.slug;
            html = body.html;
            source = body.source;
            source_format = body.source_format;
            base_sha = body.base_sha;
        }

        // A source is kept only in a format something can render again.
        // Anything else is dropped rather than refused: the document itself
        // is fine, it simply cannot be reopened in the editor.
        if !self.config.storable_source(&source_format) {
            source.clear();
            source_format.clear();
        }
        if source.is_empty() {
            source_format.clear();
        }
        if source.len() > max_html {
            return Err(write_json(413, &json!({"error": "source too large"})));
        }

        let title = title.trim().to_string();
        if title.is_empty() || html.trim().is_empty() {
            return Err(write_json(
                400,
                &json!({"error": "title and html are required"}),
            ));
        }
        // Stripped of control characters the same way every other free-text
        // field is, then capped: refused rather than truncated, and before
        // anything is written, so a caller sees the limit rather than a
        // silently shortened title.
        let title = clean(&title, title.chars().count());
        if title.chars().count() > self.config.max_title {
            return Err(write_json(400, &json!({"error": "title too long"})));
        }
        if html.len() > max_html {
            return Err(write_json(413, &json!({"error": "document too large"})));
        }
        Ok(Upload {
            title,
            slug,
            html,
            source,
            source_format,
            base_sha,
        })
    }

    /// Hands back the markup a document was rendered from, so an editor can
    /// reopen it. Gated exactly as replacing it is: a document whose publisher
    /// is someone else answers as a missing one does, so a guessed slug
    /// reveals nothing about who published what.
    async fn handle_source(&self, headers: &HeaderMap, arrival: &Arrival, slug: &str) -> Reply {
        if !self.valid_slug(slug) {
            return write_json(400, &json!({"error": "bad slug"}));
        }
        let id = self.whoami(headers, arrival).await;
        let entry = match self.store.get(slug).await {
            Some(entry) if entry.owned_by(&self.owner(headers, arrival, &id), &id.id) => entry,
            _ => return write_json(404, &json!({"error": "not found"})),
        };
        if entry.source_format.is_empty() {
            return write_json(
                404,
                &json!({"error": "this document has no stored source; publish it again from its markdown to edit it"}),
            );
        }
        let Ok(source) = self.store.read_source(slug).await else {
            return write_json(
                404,
                &json!({"error": "this document's source is no longer stored"}),
            );
        };
        write_json(
            200,
            &json!({
                "slug": entry.slug, "title": entry.title, "sha": entry.sha,
                "format": entry.source_format, "source": String::from_utf8_lossy(&source),
            }),
        )
    }

    async fn handle_delete(&self, headers: &HeaderMap, arrival: &Arrival, slug: &str) -> Reply {
        if cross_site_refused(headers, arrival) {
            return write_json(403, &cross_site_refusal());
        }
        if !self.valid_slug(slug) {
            return write_json(400, &json!({"error": "bad slug"}));
        }
        let who = match self.publisher(headers, arrival).await {
            Ok(who) => who,
            Err(response) => return response,
        };
        // Another publisher's document answers exactly as a missing one does,
        // so a guessed slug reveals nothing.
        let entry = match self.store.get(slug).await {
            Some(entry) if entry.owned_by(&who.key, &who.id) => entry,
            _ => return write_json(404, &json!({"error": "not found"})),
        };
        match self.delete_document(slug).await {
            Ok(removed) => write_json(
                200,
                &json!({"deleted": slug, "title": entry.title, "versions_removed": removed}),
            ),
            Err(_) => write_json(500, &json!({"error": "could not remove the document"})),
        }
    }

    async fn serve_document(&self, arrival: &Arrival, slug: &str, digest: &str) -> Reply {
        if !self.valid_slug(slug) || !is_sha(digest) {
            return plain(404, "not found");
        }
        // Where the bytes come from is a separate question from what this
        // response says about them. If they live in a bucket the reader's
        // browser can reach, they go straight there and never pass through
        // this process. A bare redirect will not do, though: the headers below
        // are what confine a document, and the agent injected into it is what
        // makes it annotable. So this response is still made, and it fetches
        // the document itself.
        let direct = if self.direct_reads {
            self.store
                .blobs
                .presigned_get(&document_key(slug, digest), 120)
        } else {
            None
        };
        let raw = match &direct {
            Some(_) => Vec::new(),
            None => match self.store.read(slug, digest).await {
                Ok(raw) => raw,
                Err(_) => return plain(404, "not found"),
            },
        };
        let reader = arrival.reader_origin();
        // The document runs on its own origin, with nothing of the reader's to
        // reach for, so it may run its own scripts: charts, maps, whatever it
        // shipped with. What it may not do is escape the frame or be framed by
        // anyone but the reader.
        let body = match direct {
            Some(url) => direct_document(&url, &reader),
            None => with_agent(&raw, &reader),
        };
        let mut response = Response::new(Body::from(body));
        set(&mut response, "content-type", "text/html; charset=utf-8");
        set(
            &mut response,
            "content-security-policy",
            &format!(
                "default-src 'self' data: blob: https:; \
                 script-src 'self' 'unsafe-inline' 'unsafe-eval' data: blob: https:; \
                 style-src 'self' 'unsafe-inline' data: https:; \
                 frame-ancestors {reader}; form-action 'none'; base-uri 'none'"
            ),
        );
        set(&mut response, "x-content-type-options", "nosniff");
        privacy_headers(&mut response);
        // Content-addressed path, so the bytes behind a URL never change.
        set(
            &mut response,
            "cache-control",
            "public, max-age=31536000, immutable",
        );
        response
    }

    /// The sign-in routes: the redirect to GitHub, the callback it returns to,
    /// signing out, and the two endpoints the page and the CLI ask about the
    /// current state.
    async fn handle_auth(
        &self,
        method: &Method,
        path: &str,
        headers: &HeaderMap,
        query: Option<&str>,
        arrival: &Arrival,
    ) -> Option<Reply> {
        let https = arrival.is_https();
        let query: HashMap<String, String> = query
            .map(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .into_owned()
                    .collect()
            })
            .unwrap_or_default();
        match path {
            "/auth/login" => {
                // Nothing to sign in to: this deployment is open to everyone
                // and was started without a GitHub OAuth app.
                if !self.app.configured() {
                    return Some(plain(
                        404,
                        "this deployment has no sign-in: everyone may read, comment and publish",
                    ));
                }
                let state = random_token();
                // The next URL is arbitrary caller-supplied text, so it is
                // URL-encoded before it rides beside the state token in one
                // cookie value.
                let next = query.get("next").cloned().unwrap_or_default();
                let value = format!("{state}|{}", url_escape(&next));
                let mut response =
                    redirect(&self.app.authorize_url(&arrival.callback_url(), &state));
                add_cookie(
                    &mut response,
                    &set_cookie(&cookie_name(https, STATE_COOKIE), &value, 600, https),
                );
                Some(response)
            }
            "/auth/callback" => {
                let Some(value) = cookie(headers, &cookie_name(https, STATE_COOKIE)) else {
                    return Some(plain(400, "sign-in expired; try again"));
                };
                let (state, encoded_next) = value.split_once('|').unwrap_or((&value, ""));
                // The state ties this callback to the redirect that started
                // it, so a link someone else crafted cannot sign you in as
                // them.
                if state.is_empty() || query.get("state").map(String::as_str) != Some(state) {
                    return Some(plain(400, "sign-in state did not match; try again"));
                }
                let next = url_unescape(encoded_next);
                let code = query.get("code").cloned().unwrap_or_default();
                let token = match self.app.exchange(&code, &arrival.callback_url()).await {
                    Ok(token) => token,
                    Err(err) => {
                        return Some(plain(400, &format!("github refused the sign-in: {err}")))
                    }
                };
                let who = match crate::auth::login_for(&token).await {
                    Ok(who) => who,
                    Err(_) => return Some(plain(502, "github would not say who you are")),
                };
                let mut response = redirect(&local_path(&next));
                let session = sign_session(
                    &self.key,
                    &who,
                    now_unix() + SESSION_MAX_AGE.as_secs() as i64,
                );
                add_cookie(
                    &mut response,
                    &set_cookie(
                        &cookie_name(https, SESSION_COOKIE),
                        &session,
                        SESSION_MAX_AGE.as_secs() as i64,
                        https,
                    ),
                );
                add_cookie(
                    &mut response,
                    &clear_cookie(&cookie_name(https, STATE_COOKIE), https),
                );
                Some(response)
            }
            "/auth/logout" => {
                // A GET here would be a plain link or a browser prefetch either
                // could trigger from a hostile page, and cookies alone do not
                // stop that on a same-site document host; POST plus rule A's
                // checks below do.
                if *method != Method::POST {
                    let mut response = plain(405, "method not allowed");
                    set(&mut response, "allow", "POST");
                    return Some(response);
                }
                if cross_site_refused(headers, arrival) {
                    return Some(write_json(403, &cross_site_refusal()));
                }
                let mut response = write_json(200, &json!({"logged_out": true}));
                add_cookie(
                    &mut response,
                    &clear_cookie(&cookie_name(https, SESSION_COOKIE), https),
                );
                Some(response)
            }
            "/api/me" => {
                let id = self.whoami(headers, arrival).await;
                Some(write_json(
                    200,
                    &json!({
                        "login": id.login,
                        "can_publish": self.publishers.allows(&id.login),
                        "can_comment": self.commenters.allows(&id.login),
                        "comments_need_login": !self.commenters.public,
                        // A wholly public deployment has no OAuth app, so there
                        // is nothing to sign in to and the page hides the button.
                        "can_sign_in": self.app.configured(),
                        "publishers": self.publishers.describe(),
                        "commenters": self.commenters.describe(),
                    }),
                ))
            }
            // The client id is public by design; the CLI asks for it so `login`
            // needs no configuration of its own.
            "/api/auth/config" => Some(write_json(200, &json!({"client_id": self.app.client_id}))),
            // What this deployment will accept, so the upload page can refuse a
            // 30 MB mistake before it is sent rather than after.
            "/api/config" => Some(write_json(200, &json!(*self.config))),
            _ => None,
        }
    }

    /// Names a browser the first time it is served a page, so an upload it
    /// makes without signing in belongs to it and to nobody else. Only pages
    /// carry it: an image or a font is not where a session starts.
    fn issue_visitor(
        &self,
        headers: &HeaderMap,
        arrival: &Arrival,
        asset: &ShellFile,
        response: &mut Reply,
    ) {
        if !asset.kind.starts_with("text/html") {
            return;
        }
        let https = arrival.is_https();
        // An unsigned cookie -- from before this server signed them, or forged
        // -- verifies as absent, so it is simply replaced with a signed one.
        if let Some(value) = cookie(headers, &cookie_name(https, VISITOR_COOKIE)) {
            if !read_visitor(&self.key, &value).is_empty() {
                return;
            }
        }
        let value = sign_visitor(&self.key, &random_token());
        add_cookie(
            response,
            &set_cookie(
                &cookie_name(https, VISITOR_COOKIE),
                &value,
                365 * 24 * 3600,
                https,
            ),
        );
        // This response carries a freshly minted visitor cookie, and a shared
        // cache handing that same identity to the next browser would defeat
        // the point of having one.
        set(response, "cache-control", "private, no-store");
    }
}

struct Upload {
    title: String,
    slug: String,
    html: String,
    source: String,
    source_format: String,
    base_sha: String,
}

pub fn slug_pattern(config: &Configuration) -> regex::Regex {
    regex::Regex::new(&config.slug_pattern).expect("the slug pattern is a valid expression")
}

/// The page that fetches a document from the bucket and becomes it.
/// document.write rather than innerHTML, because a document may carry scripts
/// of its own -- a chart, a map -- and innerHTML would leave them inert. The
/// agent is added afterwards, so it is there whichever way the bytes arrived.
fn direct_document(url: &str, reader: &str) -> Vec<u8> {
    let quoted_url = serde_json::to_string(url).unwrap_or_default();
    let quoted_agent = serde_json::to_string(&docs_origin_agent(reader)).unwrap_or_default();
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body>\n<script>\n(async () => {{\n  \
         const response = await fetch({quoted_url}, {{ mode: \"cors\" }});\n  if (!response.ok) {{\n    \
         document.body.textContent = \"This document could not be fetched from its bucket.\";\n    return;\n  }}\n  \
         const html = await response.text();\n  document.open();\n  document.write(html);\n  \
         const agent = document.createElement(\"script\");\n  agent.src = {quoted_agent};\n  \
         document.body.appendChild(agent);\n  document.close();\n}})();\n</script>\n</body></html>"
    )
    .into_bytes()
}

/// The agent's own URL, carrying the origin it may talk to.
fn docs_origin_agent(reader: &str) -> String {
    format!("/agent.js?reader={}", url_escape(reader))
}

/// Appends the in-frame half of the reader to a document. The stored bytes are
/// never modified; the script is added on the way out, and told which origin
/// to talk back to. Before </body> if there is one, so the document has parsed
/// by the time the agent runs; appended otherwise.
pub fn with_agent(document: &[u8], reader: &str) -> Vec<u8> {
    let tag = format!(
        "<script src=\"/agent.js?reader={}\"></script>",
        url_escape(reader)
    )
    .into_bytes();
    let lower = document.to_ascii_lowercase();
    let mut out = Vec::with_capacity(document.len() + tag.len());
    match rfind(&lower, b"</body>") {
        Some(at) => {
            out.extend_from_slice(&document[..at]);
            out.extend_from_slice(&tag);
            out.extend_from_slice(&document[at..]);
        }
        None => {
            out.extend_from_slice(document);
            out.extend_from_slice(&tag);
        }
    }
    out
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// Where a sign-in may return to: somewhere on this site, and nowhere else. A
/// value like "//elsewhere.example" starts with a slash but is read by
/// browsers as an absolute URL, which would make the callback an open
/// redirect, so the path is parsed and required to carry no scheme or host.
pub fn local_path(next: &str) -> String {
    if next.is_empty() || !next.starts_with('/') || next.starts_with("//") {
        return "/".to_string();
    }
    let Ok(parsed) = url::Url::parse("http://komodoc.invalid").and_then(|base| base.join(next))
    else {
        return "/".to_string();
    };
    if parsed.host_str() != Some("komodoc.invalid") || next.contains('\\') {
        return "/".to_string();
    }
    let mut target = parsed.path().to_string();
    if let Some(query) = parsed.query() {
        target.push('?');
        target.push_str(query);
    }
    if let Some(fragment) = parsed.fragment() {
        target.push('#');
        target.push_str(fragment);
    }
    target
}

/// What the rate limiter counts against. Behind a reverse proxy the peer is
/// the proxy, so the first X-Forwarded-For entry is the client -- but only a
/// peer that could be that proxy is believed. A header from a direct client
/// is its own invention, and honouring it would let one address claim a fresh
/// identity for every comment and never be limited.
pub fn client_address(peer: SocketAddr, headers: &HeaderMap) -> String {
    let host = peer.ip();
    if let Some(forwarded) = header_of(headers, "x-forwarded-for") {
        if local_peer(host) {
            if let Some(first) = forwarded
                .split(',')
                .next()
                .map(str::trim)
                .filter(|f| !f.is_empty())
            {
                return first.to_string();
            }
        }
    }
    host.to_string()
}

/// True for the addresses a reverse proxy in front of this process connects
/// from: the loopback interface, or a private network alongside it.
pub fn local_peer(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return local_peer(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/* ------------------------------------------------------------ responses */

fn set(response: &mut Reply, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
}

pub fn write_json(status: u16, payload: &Value) -> Reply {
    let mut response = Response::new(Body::from(payload.to_string()));
    *response.status_mut() =
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    set(
        &mut response,
        "content-type",
        "application/json; charset=utf-8",
    );
    response
}

fn plain(status: u16, text: &str) -> Reply {
    let mut response = Response::new(Body::from(format!("{text}\n")));
    *response.status_mut() =
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    set(&mut response, "content-type", "text/plain; charset=utf-8");
    set(&mut response, "x-content-type-options", "nosniff");
    response
}

fn redirect(location: &str) -> Reply {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::FOUND;
    set(&mut response, "location", location);
    response
}

/// Serves one shell file, cached for a year if its bytes never change.
fn write_asset(asset: &ShellFile) -> Reply {
    let mut response = Response::new(Body::from(asset.body.clone()));
    set(&mut response, "content-type", asset.kind);
    // Every shell page -- index, reader, documentation -- is a place a hostile
    // site could otherwise iframe to phish against, since the reader carries a
    // session cookie. A document keeps its own CSP, set where it is served,
    // which already names the one origin allowed to frame it.
    if asset.kind.starts_with("text/html") {
        set(
            &mut response,
            "content-security-policy",
            "frame-ancestors 'none'",
        );
    }
    privacy_headers(&mut response);
    if asset.immutable {
        set(
            &mut response,
            "cache-control",
            "public, max-age=31536000, immutable",
        );
    } else {
        set(&mut response, "cache-control", "public, max-age=300");
    }
    response
}

/// Keeps an unlisted link unlisted. The slug is the only thing standing
/// between a document and the public, and a URL is easy to spill: a link in
/// the document sends it to whatever site the reader clicks through to, and a
/// crawler that finds it once has it for good.
fn privacy_headers(response: &mut Reply) {
    set(response, "referrer-policy", "no-referrer");
    set(response, "x-robots-tag", "noindex, nofollow, noarchive");
}

/* -------------------------------------------------------------- cookies */

/// The value of one cookie on a request, if it was sent.
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for line in headers.get_all(header::COOKIE) {
        let Ok(line) = line.to_str() else { continue };
        for pair in line.split(';') {
            let pair = pair.trim();
            if let Some((key, value)) = pair.split_once('=') {
                if key.trim() == name {
                    return Some(value.trim().to_string());
                }
            }
        }
    }
    None
}

/// A cookie as every one here is set: on the whole site, unreadable by
/// scripts, sent only on same-site navigations, and Secure wherever the
/// deployment is HTTPS -- behind a proxy that terminates TLS included, which
/// is why the scheme is read from the request rather than the connection.
fn set_cookie(name: &str, value: &str, max_age: i64, https: bool) -> String {
    let mut cookie = format!("{name}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax");
    if https {
        cookie.push_str("; Secure");
    }
    cookie
}

fn clear_cookie(name: &str, https: bool) -> String {
    set_cookie(name, "", -1, https).replace("Max-Age=-1", "Max-Age=0")
}

fn add_cookie(response: &mut Reply, cookie: &str) {
    if let Ok(value) = HeaderValue::from_str(cookie) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

fn url_escape(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn url_unescape(value: &str) -> String {
    url::form_urlencoded::parse(format!("v={value}").as_bytes())
        .find(|(k, _)| k == "v")
        .map(|(_, v)| v.to_string())
        .unwrap_or_default()
}
