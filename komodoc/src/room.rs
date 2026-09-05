//! A room owns one document's comments and the open sockets of everyone
//! reading it, so a write by one reader reaches the others without anybody
//! polling. One process, so the mutex plays the part a single-threaded actor
//! would.
//!
//! Field names follow the W3C Web Annotation Data Model, so exporting is a
//! reshaping rather than a translation: exact, prefix and suffix are a
//! TextQuoteSelector, motivation is the standard vocabulary, and creator and
//! created mean what the spec says. resolved is ours; the spec has no notion
//! of it, and permits extra properties.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, Mutex};

use crate::blob::{room_key, room_lock_key, take_room_lock, BlobStore};
use crate::clock::{now_unix, timestamp};
use crate::config::Configuration;
use crate::util::{clean, new_id};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Reply {
    pub id: String,
    pub body: String,
    pub creator: String,
    pub created: String,
    /// Who actually posted this reply -- a github: or visitor: key, or "" for a
    /// caller with neither -- so a delete can be restricted to it. Never
    /// serialized: a reply is marshaled directly into broadcasts, snapshots and
    /// REST responses, none of which should carry it; `to_stored` below is the
    /// only shape that puts it on disk, and loading reads it back.
    #[serde(default, skip_serializing)]
    pub author: String,
}

/// A rectangle on an image, in percentages of the image's own size, so it
/// survives the document being displayed at any width.
///
/// Which image is a harder question than where on it. There is no text around
/// a figure to anchor to, so two identifiers are kept: a digest of the image
/// source, which survives the figure moving, and its position among the
/// document's images, which survives the image being re-encoded. The reader
/// tries the digest first.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Region {
    #[serde(default)]
    pub image_digest: String,
    #[serde(default)]
    pub image_index: i64,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    #[serde(default, rename = "w")]
    pub width: f64,
    #[serde(default, rename = "h")]
    pub height: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Comment {
    pub id: String,
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub motivation: String,
    #[serde(default)]
    pub exact: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    /// Where the passage sat when the comment was made. Null for a comment
    /// written before it was recorded, rather than claiming offset 0.
    #[serde(default)]
    pub position: Option<i64>,
    /// Set instead of the text selector when the annotation is on part of a
    /// figure rather than on a run of words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<Region>,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub created: String,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub resolved_at: Option<String>,
    #[serde(default)]
    pub replies: Vec<Reply>,
    /// Who actually posted this comment: "github:<login>" for a signed-in
    /// caller, "visitor:<sha256 of the visitor token>" for a verified
    /// anonymous browser, or "" for neither (including every seeded example,
    /// which belongs to nobody in particular). Kept out of every client-bound
    /// shape for the same reason as `Reply::author`.
    #[serde(default, skip_serializing)]
    pub author: String,
}

/// What lands on disk, one object per document. Author is excluded from a
/// comment's own JSON so that nothing marshaling one for a client leaks it by
/// accident; this is the one place that value is meant to travel.
#[derive(Deserialize)]
struct RoomState_ {
    #[serde(default)]
    seq: i64,
    #[serde(default)]
    comments: Vec<Comment>,
}

fn to_stored(items: &[Comment]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| {
                let mut stored = json!(item);
                if !item.author.is_empty() {
                    stored["author"] = json!(item.author);
                }
                stored["replies"] = Value::Array(
                    item.replies
                        .iter()
                        .map(|answer| {
                            let mut reply = json!(answer);
                            if !answer.author.is_empty() {
                                reply["author"] = json!(answer.author);
                            }
                            reply
                        })
                        .collect(),
                );
                stored
            })
            .collect(),
    )
}

/// What a caller is shown: every comment field a client ever sees, plus
/// whether this particular caller may delete it.
#[derive(Serialize)]
pub struct CommentView {
    #[serde(flatten)]
    pub comment: Comment,
    pub deletable: bool,
}

/// Rule H's authorization test: the document's owner may delete anything on
/// it, and everyone else only their own -- and "their own" never matches on
/// two callers who both have no author key, which is what an anonymous caller
/// with no visitor cookie and a nobody's-in-particular seeded example both
/// look like.
pub fn deletable(item: &Comment, author: &str, is_owner: bool) -> bool {
    is_owner || (!author.is_empty() && item.author == author)
}

/// One client frame. Every field is optional; `apply` decides which ones a
/// given type needs.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Message {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub motivation: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub exact: String,
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub suffix: String,
    #[serde(default)]
    pub position: Option<i64>,
    #[serde(default)]
    pub region: Option<Region>,
    #[serde(default)]
    pub comment_id: String,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub temp_id: String,
    /// A Yjs update, base64-encoded, and whether it is the whole state of the
    /// session rather than one more change to it.
    #[serde(default)]
    pub update: String,
    #[serde(default)]
    pub replace: bool,
}

/// What a room sends a connected socket: a text frame, or the order to close.
#[derive(Clone, Debug)]
pub enum Outgoing {
    Text(String),
    Close(&'static str),
}

pub type Sender = mpsc::UnboundedSender<Outgoing>;

/// One connected reader: where they are, which the rate limiter counts
/// against, and the channel their frames go out on.
pub struct Peer {
    #[allow(dead_code)]
    pub address: String,
    pub tx: Sender,
}

pub struct RoomState {
    pub seq: i64,
    pub comments: Vec<Comment>,
    pub sockets: HashMap<u64, Peer>,
    /// "address:hour" to count.
    pub rate: HashMap<String, i64>,
    /// The live editing session, if anyone is editing: the Yjs updates that
    /// have been sent since it began, and whether it has been started at all.
    ///
    /// The server relays and remembers; it never merges. Nothing here is
    /// persisted, because the durable copy of a document is the source a save
    /// stores -- when the last editor leaves, the session is over and the next
    /// one is seeded from that source again.
    pub edits: Vec<String>,
    pub seeded: bool,
}

/// How many updates a session keeps before the next one to send is asked for
/// the whole state instead, so a long session does not hand a latecomer
/// thousands of keystrokes to replay.
pub const EDIT_LOG_MAX: usize = 200;

pub struct Room {
    pub slug: String,
    blobs: Arc<dyn BlobStore>,
    config: Arc<Configuration>,
    /// True when another server holds this room's lock: it can be read and
    /// served, but nothing here may write over what that server is doing.
    pub read_only: bool,
    pub state: Mutex<RoomState>,
}

pub struct RoomSet {
    /// Who this server is, in the lock objects it takes. A name rather than a
    /// pid, because a pid means nothing to whoever reads the refusal.
    holder: String,
    /// Comments live wherever the documents do. On a bucket that makes the
    /// server genuinely stateless.
    pub blobs: Arc<dyn BlobStore>,
    config: Arc<Configuration>,
    rooms: Mutex<HashMap<String, Arc<Room>>>,
}

/// Names this process in a room lock: the machine it runs on and its pid,
/// which is enough to tell two of them apart and to tell a restart from a
/// second server.
fn this_server() -> String {
    let host = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "server".to_string());
    format!("{host}/{}", std::process::id())
}

impl RoomSet {
    pub fn new(blobs: Arc<dyn BlobStore>, config: Arc<Configuration>) -> RoomSet {
        RoomSet {
            holder: this_server(),
            blobs,
            config,
            rooms: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get(&self, slug: &str) -> Arc<Room> {
        let mut rooms = self.rooms.lock().await;
        if let Some(existing) = rooms.get(slug) {
            return existing.clone();
        }
        // Taken before anything is read, so a second server writing the same
        // bucket finds out it is second rather than interleaving its comments
        // with the first one's.
        let (held, by) = take_room_lock(self.blobs.as_ref(), slug, &self.holder).await;
        if !held {
            eprintln!("warning: {slug} is being written by {by}; comments here are read-only in this server");
        }
        let room = Arc::new(Room {
            slug: slug.to_string(),
            blobs: self.blobs.clone(),
            config: self.config.clone(),
            read_only: !held,
            state: Mutex::new(RoomState {
                seq: 0,
                comments: Vec::new(),
                sockets: HashMap::new(),
                rate: HashMap::new(),
                edits: Vec::new(),
                seeded: false,
            }),
        });
        room.load().await;
        rooms.insert(slug.to_string(), room.clone());
        room
    }

    /// Drops a document's comments and disconnects anyone still reading it.
    /// Reached only through the delete route, which checks ownership first.
    pub async fn purge(&self, slug: &str) {
        let room = self.get(slug).await;
        {
            let mut state = room.state.lock().await;
            state.comments.clear();
            state.seq = 0;
            for peer in state.sockets.values() {
                let _ = peer.tx.send(Outgoing::Close("document deleted"));
            }
        }
        let _ = self
            .blobs
            .delete(&[room_key(slug), room_lock_key(slug)])
            .await;
        self.rooms.lock().await.remove(slug);
    }
}

impl Room {
    async fn load(&self) {
        let Ok(raw) = self.blobs.get(&room_key(&self.slug)).await else {
            return;
        };
        let Ok(stored) = serde_json::from_slice::<RoomState_>(&raw) else {
            return;
        };
        let mut state = self.state.lock().await;
        state.seq = stored.seq;
        state.comments = stored.comments;
        state.comments.sort_by_key(|item| item.seq);
    }

    /// Rewrites the whole room. Comment volume per document is in the dozens,
    /// so this stays cheaper than any incremental scheme. The in-memory copy
    /// is the source of truth while anyone is connected; writing goes wherever
    /// the documents go, so the server holds nothing that only it has.
    pub async fn save(&self, state: &RoomState) -> Result<(), String> {
        if self.read_only {
            return Err("this room is held by another server".into());
        }
        let raw = json!({"seq": state.seq, "comments": to_stored(&state.comments)});
        let body = serde_json::to_vec(&raw).map_err(|err| err.to_string())?;
        self.blobs
            .put(&room_key(&self.slug), body, "application/json")
            .await
            .map_err(|err| err.to_string())
    }

    /// Every comment, for seeding and for the tests that read a room back.
    #[allow(dead_code)]
    pub async fn snapshot(&self) -> Vec<Comment> {
        self.state.lock().await.comments.clone()
    }

    /// The per-caller view of the whole thread: the hello frame and the REST
    /// listing both need this, since deletable differs by who is asking.
    pub async fn snapshot_for(&self, author: &str, is_owner: bool) -> Vec<CommentView> {
        let state = self.state.lock().await;
        state
            .comments
            .iter()
            .map(|item| CommentView {
                comment: item.clone(),
                deletable: deletable(item, author, is_owner),
            })
            .collect()
    }

    /// (total, open)
    pub async fn counts(&self) -> (usize, usize) {
        let state = self.state.lock().await;
        let open = state.comments.iter().filter(|item| !item.resolved).count();
        (state.comments.len(), open)
    }

    pub async fn attach(&self, id: u64, address: String, tx: Sender) {
        self.state
            .lock()
            .await
            .sockets
            .insert(id, Peer { address, tx });
    }

    pub async fn detach(&self, id: u64) {
        self.state.lock().await.sockets.remove(&id);
    }

    pub async fn broadcast(&self, payload: &Value) {
        self.broadcast_except(None, payload).await;
    }

    /// Sends to everyone but one socket: an editing update is relayed to the
    /// others, and the one that sent it already has it.
    pub async fn broadcast_except(&self, skip: Option<u64>, payload: &Value) {
        let message = payload.to_string();
        let state = self.state.lock().await;
        for (id, peer) in &state.sockets {
            if Some(*id) != skip {
                // A failing socket is on its way out; its own task cleans up.
                let _ = peer.tx.send(Outgoing::Text(message.clone()));
            }
        }
    }

    /// Counts writes per address per hour, and forgets older hours as it goes
    /// rather than accumulating an entry per address per hour.
    fn rate_ok(&self, state: &mut RoomState, address: &str) -> bool {
        if address.is_empty() {
            return true;
        }
        let hour = now_unix() / 3600;
        let key = format!("{}:{hour}", rate_key(address));
        let suffix = format!(":{hour}");
        state.rate.retain(|existing, _| existing.ends_with(&suffix));
        let count = state.rate.get(&key).copied().unwrap_or(0);
        if count + 1 > self.config.rate_per_hour {
            return false;
        }
        state.rate.insert(key, count + 1);
        true
    }

    /// Validates, persists, and returns the event to broadcast. The second
    /// result is false when the event is an error, which goes only to its
    /// sender. `author` is the caller's own author key, and `is_owner` says
    /// whether the caller owns the document this room belongs to; both come
    /// from the caller's identity and are never taken from the message itself.
    pub async fn apply(
        &self,
        incoming: Message,
        address: &str,
        author: &str,
        is_owner: bool,
    ) -> (Value, bool) {
        let mut state = self.state.lock().await;
        let config = self.config.clone();

        let fail = |text: &str| -> (Value, bool) {
            let mut payload =
                json!({"type": "error", "message": text, "temp_id": incoming.temp_id});
            // Named so the reader knows which optimistic row to roll back.
            if !incoming.comment_id.is_empty() {
                payload["comment_id"] = json!(incoming.comment_id);
            }
            (payload, false)
        };
        const UNSAVED: &str = "could not save that comment; try again";

        // Resolving and deleting cost a slot too, the same as posting: a
        // caller who could resolve or delete without limit could still make a
        // thread unusable, just by different means than flooding it with text.
        if !self.rate_ok(&mut state, address) {
            return fail("too many comments from this address; try later");
        }

        if incoming.kind == "resolve" {
            let Some(index) = state
                .comments
                .iter()
                .position(|item| item.id == incoming.comment_id)
            else {
                return fail("unknown comment");
            };
            let (was_resolved, was_resolved_at) = (
                state.comments[index].resolved,
                state.comments[index].resolved_at.clone(),
            );
            state.comments[index].resolved = incoming.resolved;
            state.comments[index].resolved_at = incoming.resolved.then(timestamp);
            if self.save(&state).await.is_err() {
                state.comments[index].resolved = was_resolved;
                state.comments[index].resolved_at = was_resolved_at;
                return fail(UNSAVED);
            }
            let target = &state.comments[index];
            return (
                json!({
                    "type": "resolve", "comment_id": target.id,
                    "resolved": target.resolved, "resolved_at": target.resolved_at,
                }),
                true,
            );
        }

        if incoming.kind == "delete" {
            let Some(index) = state
                .comments
                .iter()
                .position(|item| item.id == incoming.comment_id)
            else {
                return fail("unknown comment");
            };
            if !deletable(&state.comments[index], author, is_owner) {
                return fail("you may only delete your own comments");
            }
            let removed = state.comments.remove(index);
            if self.save(&state).await.is_err() {
                state.comments.insert(index, removed);
                return fail(UNSAVED);
            }
            return (
                json!({"type": "delete", "comment_id": incoming.comment_id}),
                true,
            );
        }

        let body = clean(&incoming.body, config.caps.body).trim().to_string();
        let motivation = config.allowed_motivation(&incoming.motivation);
        // A highlight is the passage itself: marking something as worth
        // returning to needs no words. Everything else is a remark, and a
        // remark with no words is nothing.
        if body.is_empty() && !(incoming.kind == "comment" && motivation == "highlighting") {
            return fail("comment body is required");
        }
        let mut creator = clean(&incoming.creator, config.caps.creator)
            .trim()
            .to_string();
        if creator.is_empty() {
            creator = "Anonymous".to_string();
        }

        match incoming.kind.as_str() {
            "reply" => {
                let Some(index) = state
                    .comments
                    .iter()
                    .position(|item| item.id == incoming.comment_id)
                else {
                    return fail("unknown comment");
                };
                if state.comments[index].replies.len() >= config.max_replies {
                    return fail("this comment has reached its reply limit");
                }
                let added = Reply {
                    id: new_id(),
                    body,
                    creator,
                    created: timestamp(),
                    author: author.to_string(),
                };
                state.comments[index].replies.push(added.clone());
                if self.save(&state).await.is_err() {
                    state.comments[index].replies.pop();
                    return fail(UNSAVED);
                }
                (
                    json!({
                        "type": "reply", "comment_id": state.comments[index].id,
                        "reply": added, "temp_id": incoming.temp_id,
                    }),
                    true,
                )
            }
            "comment" => {
                if state.comments.len() >= config.max_comments {
                    return fail("this document has reached its comment limit");
                }
                let exact = clean(&incoming.exact, config.caps.exact).trim().to_string();
                let spot = valid_region(incoming.region.as_ref());
                // An annotation is anchored to words or to part of a figure;
                // one or the other, never neither.
                if exact.is_empty() && spot.is_none() {
                    return fail("select some text or part of a figure to comment on");
                }
                state.seq += 1;
                // The selector is the durable anchor. Offsets are recomputed in
                // the reader against whatever version of the document is on
                // screen, so replacing a document needs no migration pass here.
                let added = Comment {
                    id: new_id(),
                    seq: state.seq,
                    motivation,
                    exact,
                    prefix: clean(&incoming.prefix, config.caps.context),
                    suffix: clean(&incoming.suffix, config.caps.context),
                    position: incoming.position.filter(|p| *p >= 0),
                    region: spot,
                    body,
                    tags: clean_tags(&incoming.tags, &config),
                    creator,
                    created: timestamp(),
                    resolved: false,
                    resolved_at: None,
                    replies: Vec::new(),
                    author: author.to_string(),
                };
                state.comments.push(added.clone());
                if self.save(&state).await.is_err() {
                    state.comments.pop();
                    state.seq -= 1;
                    return fail(UNSAVED);
                }
                (
                    json!({"type": "comment", "comment": added, "temp_id": incoming.temp_id}),
                    true,
                )
            }
            _ => fail("unknown message type"),
        }
    }

    /* ---------------------------------------------------------- editing */

    /// What a browser is given when it starts editing: the updates this
    /// session has seen, and whether it is the one to start the session from
    /// the stored source. Exactly one browser is ever told to seed -- two
    /// seeding separately would each build their own history of the same
    /// words, and merging those shows the document twice.
    pub async fn editing_state(&self) -> (Vec<String>, bool) {
        let mut state = self.state.lock().await;
        if !state.seeded {
            state.seeded = true;
            return (Vec::new(), true);
        }
        (state.edits.clone(), false)
    }

    /// Records an update and says whether the sender should follow it with
    /// the whole state. Relaying is the caller's job: this is only the memory.
    pub async fn editing(&self, update: String, replace: bool) -> bool {
        let mut state = self.state.lock().await;
        if replace {
            state.edits = vec![update];
            return false;
        }
        state.edits.push(update);
        state.edits.len() > EDIT_LOG_MAX
    }

    /// Forgets a session once the last person editing has gone. What they had
    /// is in the document they saved, or was never saved at all -- the same as
    /// an unsaved page closed today.
    pub async fn end_editing(&self) {
        let mut state = self.state.lock().await;
        if state.sockets.is_empty() {
            state.edits.clear();
            state.seeded = false;
        }
    }

    /// How many people have this document open, which is worth showing them.
    pub async fn editors(&self) -> usize {
        self.state.lock().await.sockets.len()
    }
}

/// What the rate limiter actually counts against: an IPv4 address used whole,
/// or an IPv6 address reduced to its /64 -- the block an ISP typically hands
/// one customer -- so a rotating address within that prefix does not buy a
/// fresh limit. A value that does not parse as an address is used as given.
pub fn rate_key(address: &str) -> String {
    let Ok(ip) = address.parse::<IpAddr>() else {
        return address.to_string();
    };
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.to_string();
            }
            let segments = v6.segments();
            format!(
                "{:x}:{:x}:{:x}:{:x}",
                segments[0], segments[1], segments[2], segments[3]
            )
        }
    }
}

/// Normalises labels: lowercased, trimmed, deduplicated, capped in both length
/// and number, so filtering by one of them is predictable.
pub fn clean_tags(tags: &[String], config: &Configuration) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags {
        let label = clean(tag, config.caps.tag).trim().to_lowercase();
        let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
        if label.is_empty() || out.contains(&label) {
            continue;
        }
        out.push(label);
        if out.len() == config.max_tags {
            break;
        }
    }
    out
}

/// Keeps a rectangle only if it is one: inside the image, with a size worth
/// drawing. Percentages, so it holds at any display width.
pub fn valid_region(spot: Option<&Region>) -> Option<Region> {
    let spot = spot?;
    let inside = |v: f64| (0.0..=100.0).contains(&v);
    if !inside(spot.x) || !inside(spot.y) || !inside(spot.width) || !inside(spot.height) {
        return None;
    }
    if spot.width < 0.5
        || spot.height < 0.5
        || spot.x + spot.width > 100.5
        || spot.y + spot.height > 100.5
    {
        return None;
    }
    if spot.image_index < 0 {
        return None;
    }
    Some(Region {
        image_digest: clean(&spot.image_digest, 64),
        image_index: spot.image_index,
        x: spot.x,
        y: spot.y,
        width: spot.width,
        height: spot.height,
    })
}
