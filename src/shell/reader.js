import { anchorAll, flatten } from "./anchor.js";
import * as editor from "./editor.js";
import * as collab from "./collab.js";
import * as sync from "./sync.js";

const SLUG = location.pathname.split("/").pop();
const frame = document.getElementById("docframe");
const bar = document.getElementById("selectionbar");
const box = document.getElementById("comments");
const countEl = document.getElementById("count");
const connEl = document.getElementById("conn");
const dialog = document.getElementById("commentDialog");

let comments = [];
let pending = null;
let socket = null;
let backoff = 500;
let docText = null; // joined visible text, invariant across highlight renders
let docView = null; // flatten(docText), cached so anchoring does not redo it per call
let figureAt = []; // text offset of each figure, by its index in the document
let frameReady = false;
let commentsReady = false;
let identity = ""; // GitHub login, when signed in; comments are signed with it
let canModerate = false; // true when the caller owns this document (GET /api/documents/{slug})

// A browser cannot set a custom header on a cross-origin request without a
// CORS preflight, which the server never grants -- so this header is proof,
// to the server, that a state-changing request came from this page and not
// from a hostile document on the sibling docs host.
const SHELL_HEADERS = { "X-Komodoc-Client": "shell" };

// The Delete button is only ever real for: a comment the server says this
// caller may delete, one the caller just posted and is still waiting to be
// confirmed (temp_id/pending), or a caller who owns the document outright.
function canDelete(comment) {
  return Boolean(comment.deletable) || Boolean(comment.temp_id) || Boolean(comment.pending) || canModerate;
}

// One card per comment, kept across renders so a rebuild never wipes an open
// reply draft, an expanded quotation, or scroll position. Keyed by the
// comment object itself (not its id): `receive` mutates a comment in place
// when the server confirms it (Object.assign onto the same object), so the
// object's identity survives an id change and the card follows it for free.
const cards = new Map();

/* ----------------------------------------------------------------- frame */

// The document is on its own origin, so nothing here can touch it. agent.js,
// injected into the document, does the DOM work and reports back. Anchoring
// stays on this side: the agent sends text, this sends back offsets to paint.
//
// Everything arriving from the frame is untrusted. The agent shares an origin
// with the document, and a hostile document can rewrite it.

let docsOrigin = null; // learned from the API, and the only origin we accept

function tell(message) {
  if (!docsOrigin || !frame.contentWindow) return;
  frame.contentWindow.postMessage({ komodoc: true, ...message }, docsOrigin);
}

addEventListener("message", (event) => {
  if (!docsOrigin || event.origin !== docsOrigin || event.source !== frame.contentWindow) return;
  const message = event.data;
  if (!message || message.komodoc !== true) return;

  switch (message.type) {
    case "ready":
      // The whole visible text of the document, in one string.
      docText = typeof message.text === "string" ? message.text : "";
      // Computed once here, not once per anchorAll call.
      docView = flatten(docText);
      // Where each figure sits in that text, so a note on a figure can be
      // ordered against the notes on passages.
      figureAt = Array.isArray(message.images) ? message.images.map(Number) : [];
      offerBox(figureAt.length > 0);
      frameReady = true;
      // The agent's DOM was just rebuilt and rescanned, so whatever was
      // painted before is gone; the next applyHighlights must repaint in full.
      lastRegions = lastHighlight = null;
      reanchor();
      break;

    case "selection":
      showSelection(message.selector, message.rect);
      break;

    case "region":
      // A rectangle drawn on a figure, which anchors the same way a quotation
      // does: it becomes what the next annotation is about.
      pending = { exact: "", prefix: "", suffix: "", position: null, region: message.region };
      placeBar(message.rect);
      break;

    case "caret":
      followDocumentClick(Number(message.offset) || 0);
      break;

    case "focus":
      document
        .getElementById("comment-" + message.id)
        ?.scrollIntoView({ behavior: "smooth", block: "center" });
      break;
  }
});

// Hand the agent the positions to paint. It knows nothing about anchoring.
//
// The agent repaints the whole document on every "regions" or "highlight"
// message, so a call that changes nothing is not free even though it looks
// idempotent. Each is sent only when its payload actually differs from the
// last one sent -- reset when the frame republishes its text, since the
// agent's DOM was rebuilt then and needs the full repaint regardless.
let lastRegions = null;
let lastHighlight = null;

function applyHighlights() {
  if (!frameReady) return;
  // Annotations on figures are placed by the agent from the image identifiers,
  // since there is no text for this side to anchor against.
  const regions = JSON.stringify(
    comments
      .filter((comment) => comment.region)
      .map((comment) => ({
        id: comment.id,
        digest: comment.region.image_digest,
        index: comment.region.image_index,
        x: comment.region.x,
        y: comment.region.y,
        w: comment.region.w,
        h: comment.region.h,
        motivation: comment.motivation,
        resolved: Boolean(comment.resolved),
      })),
  );
  if (regions !== lastRegions) {
    lastRegions = regions;
    tell({ type: "regions", regions: JSON.parse(regions) });
  }

  const highlight = JSON.stringify(
    comments
      .filter((comment) => !comment.orphaned && comment.start != null)
      .map((comment) => ({
        id: comment.id,
        start: comment.start,
        end: comment.end,
        motivation: comment.motivation,
        resolved: Boolean(comment.resolved),
      })),
  );
  if (highlight !== lastHighlight) {
    lastHighlight = highlight;
    tell({ type: "highlight", ranges: JSON.parse(highlight) });
  }
}

function reanchor() {
  if (!frameReady || !commentsReady || docText === null) return;
  // A region annotation is placed by the agent, not by text matching, so it is
  // never orphaned for want of a quotation.
  anchorAll(docText, comments.filter((comment) => !comment.region), docView);
  render();
  applyHighlights();
}

/* -------------------------------------------------------------- selection */

// The agent reports a selector and where it sits inside the frame; the button
// goes over it, in this document's coordinates.
function showSelection(selector, rect) {
  if (!selector || !selector.exact) {
    bar.style.display = "none";
    pending = null;
    return;
  }
  pending = {
    exact: String(selector.exact),
    prefix: String(selector.prefix || ""),
    suffix: String(selector.suffix || ""),
    // A hint, not a claim: the server keeps it and anchoring uses it only to
    // choose between passages the context cannot separate.
    position: Number.isInteger(selector.position) && selector.position >= 0 ? selector.position : null,
  };
  placeBar(rect);
}

// The button goes over whatever was chosen, in this document's coordinates.
function placeBar(rect) {
  if (rect && !matchMedia("(max-width:760px)").matches) {
    const frameRect = frame.getBoundingClientRect();
    bar.style.left =
      Math.min(innerWidth - 105, frameRect.left + rect.left + (rect.right - rect.left) / 2 - 42) + "px";
    bar.style.top = Math.max(65, frameRect.top + rect.top - 42) + "px";
  }
  bar.style.display = "block";
}
/* --------------------------------------------------------------- sidebar */

function element(tag, text) {
  const el = document.createElement(tag);
  el.textContent = text;
  return el;
}

// <mark> is Pico's highlight, which is what a small status label wants to be.
function mark(text, motivation) {
  const el = element("mark", text);
  // The label wears the same hue the passage does, so the sidebar and the
  // document agree about what kind of annotation this is.
  if (motivation) el.dataset.motivation = motivation;
  return el;
}

// A passage can be a paragraph long, which would bury the comment made about
// it. The sidebar shows the opening words and expands on request.
const QUOTE_WORDS = 8;

// `card` is where "expanded or not" lives, since the quote itself gets
// rebuilt whenever anything else about the comment changes (see updateStatic)
// and a plain closure variable would forget the reader's click at that point.
function quoteOf(exact, card) {
  const quote = document.createElement("blockquote");
  const words = exact.split(/\s+/);
  if (words.length <= QUOTE_WORDS + 2) {
    quote.textContent = "“" + exact + "”";
    return quote;
  }

  const short = words.slice(0, QUOTE_WORDS).join(" ");
  const text = document.createElement("span");
  const toggle = document.createElement("a");
  toggle.href = "#";

  const draw = () => {
    text.textContent = card.quoteOpen ? "“" + exact + "”" : "“" + short;
    toggle.textContent = card.quoteOpen ? " less" : "… ”";
  };
  toggle.onclick = (event) => {
    event.preventDefault();
    event.stopPropagation(); // the card itself scrolls to the highlight
    card.quoteOpen = !card.quoteOpen;
    draw();
  };

  draw();
  quote.append(text, toggle);
  return quote;
}

// The confirmation is a modal dialog whose OK button is the default and has
// focus, so Enter confirms and Escape cancels.
const deleteDialog = document.getElementById("deleteDialog");
function confirmDelete() {
  return new Promise((resolve) => {
    deleteDialog.returnValue = "";
    deleteDialog.showModal();
    deleteDialog.addEventListener(
      "close",
      () => resolve(deleteDialog.returnValue === "ok"),
      { once: true },
    );
  });
}

const stamp = (value) => value.replace("T", " ").slice(0, 16) + " UTC";

// Filtering by tag: an empty set shows everything, and an annotation matches
// if it carries every tag chosen.
let chosenTags = new Set();

function toggleTag(tag) {
  chosenTags.has(tag) ? chosenTags.delete(tag) : chosenTags.add(tag);
  render();
}

function drawTagFilter() {
  const filter = document.getElementById("tagFilter");
  const all = new Set();
  for (const comment of comments) for (const tag of comment.tags || []) all.add(tag);

  filter.innerHTML = "";
  filter.hidden = all.size === 0;
  for (const tag of [...all].sort()) {
    const chip = element("button", tag);
    chip.type = "button";
    chip.className = chosenTags.has(tag) ? "tag chosen" : "tag";
    chip.onclick = () => toggleTag(tag);
    filter.appendChild(chip);
  }
}

// The sidebar reads in document order: an annotation is about a place in the
// text, so the column follows the page rather than the order things were
// written. A note on a figure sorts by where that figure sits in the text.
// Anything that could not be anchored has no place to sort by, so it goes to
// the end, in the order it was made.
function place(comment) {
  if (comment.region) {
    const at = figureAt[comment.region.image_index];
    if (Number.isFinite(at)) return at;
  }
  return Number.isFinite(comment.start) ? comment.start : Infinity;
}

// Everything that goes into the static part of a card (the part rebuilt
// wholesale when it changes) -- resolved/pending, replies and id are handled
// elsewhere, since those can change on their own without touching this.
function cardSignature(comment) {
  return JSON.stringify([
    comment.exact,
    comment.region,
    comment.body,
    comment.tags,
    comment.motivation,
    comment.creator,
    comment.created,
    comment.orphaned,
  ]);
}

// The quote, marks, body, tags and byline: whatever the reader
// cannot type into and cannot leave mid-edit, so rebuilding it from scratch
// on change costs nothing worth preserving.
function updateStatic(card, comment) {
  const wrap = card.staticWrap;
  wrap.innerHTML = "";
  if (comment.orphaned) wrap.appendChild(mark("Needs re-anchoring"));
  // The motivation is the W3C annotation type. Commenting is the default,
  // so only the others are worth showing.
  if (comment.motivation && comment.motivation !== "commenting") {
    wrap.appendChild(mark(comment.motivation, comment.motivation));
  }
  if (comment.region) {
    const where = element("blockquote", `Figure ${comment.region.image_index + 1}`);
    where.className = "figureref";
    wrap.appendChild(where);
  } else {
    wrap.appendChild(quoteOf(comment.exact, card));
  }
  if (comment.body) wrap.appendChild(element("p", comment.body));
  for (const tag of comment.tags || []) {
    const chip = element("button", tag);
    chip.type = "button";
    chip.className = "tag";
    chip.onclick = (event) => {
      event.stopPropagation();
      toggleTag(tag);
    };
    wrap.appendChild(chip);
  }
  wrap.appendChild(element("small", comment.creator + " · " + stamp(comment.created)));
}

// The one line a resolved card shows: what it was about, then what was said
// about it. Trimmed to a single line by CSS rather than by cutting the text,
// so the width of the column decides how much of it fits.
function summaryOf(comment) {
  const about = comment.region
    ? `Figure ${comment.region.image_index + 1}`
    : (comment.exact || "").trim();
  const said = (comment.body || "").trim();
  return [about, said].filter(Boolean).join(" — ") || "Resolved";
}

// Built once per comment and reused after that. `card.comment` is a
// reference into `comments`, and `receive` mutates that object in place
// (Object.assign), so handlers below always see the current id/resolved/etc
// without this needing to be told about it.
function makeCard(comment) {
  const el = document.createElement("article");

  // A resolved note is settled business: it collapses to this one line, and
  // stays out of the way until someone clicks it open again.
  const summary = document.createElement("div");
  summary.className = "summary";
  el.appendChild(summary);

  const staticWrap = document.createElement("div");
  el.appendChild(staticWrap);

  const repliesList = document.createElement("ul");
  repliesList.hidden = true;
  el.appendChild(repliesList);

  // Not role="group": that is Pico's segmented control, which joins its
  // buttons into one shape. These are two separate actions.
  const actions = document.createElement("div");
  actions.className = "actions";
  const resolveBtn = document.createElement("button");
  const replyButton = document.createElement("button");
  replyButton.textContent = "Reply";
  const deleteButton = document.createElement("button");
  deleteButton.textContent = "Delete";
  actions.append(resolveBtn, replyButton, deleteButton);
  el.appendChild(actions);

  const card = {
    el,
    summary,
    expanded: false, // a resolved card the reader clicked back open
    staticWrap,
    repliesList,
    repliesDrawn: [], // reply objects already rendered as <li>, in order
    actions,
    resolveBtn,
    form: null, // created lazily, on first "Reply" click
    nameInput: null,
    bodyInput: null,
    quoteOpen: false,
    sig: null,
    comment,
    deleteButton,
  };

  resolveBtn.onclick = (event) => {
    event.stopPropagation();
    // Optimistic: flip locally, then tell the room. The broadcast that
    // comes back is idempotent with what we already drew.
    const target = card.comment;
    target.resolved = !target.resolved;
    // Resolving collapses it again, however it was left open before.
    card.expanded = false;
    render();
    applyHighlights();
    send({ type: "resolve", comment_id: target.id, resolved: target.resolved });
  };

  deleteButton.onclick = async (event) => {
    event.stopPropagation();
    if (!(await confirmDelete())) return;
    // Optimistic, like resolve: drop it locally, then tell the room.
    const target = card.comment;
    comments = comments.filter((item) => item !== target);
    render();
    applyHighlights();
    send({ type: "delete", comment_id: target.id });
  };

  replyButton.onclick = (event) => {
    event.stopPropagation();
    ensureForm(card);
    card.form.hidden = !card.form.hidden;
    if (!card.form.hidden) card.bodyInput.focus();
  };

  el.onclick = (event) => {
    const target = card.comment;
    if (event.target.closest("button,input,textarea")) return;
    // Collapsed, the click is "show me this again"; open, it is "take me
    // to the place in the document this is about".
    if (target.resolved && !card.expanded) {
      card.expanded = true;
      render();
      return;
    }
    if (!target.orphaned) tell({ type: "reveal", id: target.id });
  };

  return card;
}

// Lazy, so a document with hundreds of comments does not carry hundreds of
// unused inputs, textareas and buttons that nobody ever clicked into.
function ensureForm(card) {
  if (card.form) return;
  const form = document.createElement("form");
  form.hidden = true;
  const name = document.createElement("input");
  name.placeholder = "Name";
  name.value = identity || localStorage.getItem("komodoc-author") || "Anonymous";
  name.maxLength = 80;
  // Signed in, the reply is signed by the account; there is nothing to type.
  name.hidden = Boolean(identity);
  const body = document.createElement("textarea");
  body.placeholder = "Reply";
  body.rows = 2;
  body.maxLength = 5000;
  body.required = true;
  const submit = document.createElement("button");
  submit.type = "submit";
  submit.textContent = "Add reply";
  form.append(name, body, submit);
  form.onclick = (event) => event.stopPropagation();
  form.onsubmit = (event) => {
    event.preventDefault();
    const comment = card.comment;
    if (!identity) localStorage.setItem("komodoc-author", name.value);
    const temp_id = crypto.randomUUID();
    comment.replies.push({
      id: temp_id,
      body: body.value,
      creator: name.value || "Anonymous",
      created: new Date().toISOString(),
      temp_id,
    });
    body.value = "";
    render();
    send({
      type: "reply",
      comment_id: comment.id,
      body: comment.replies[comment.replies.length - 1].body,
      creator: name.value,
      temp_id,
    });
  };
  card.el.appendChild(form);
  card.form = form;
  card.nameInput = name;
  card.bodyInput = body;
}

// Reuses cards across renders instead of wiping and rebuilding the whole
// column, so an open reply draft, an expanded quotation and the scroll
// position all survive a render triggered by someone else's comment.
function updateCard(card, comment) {
  card.el.id = "comment-" + comment.id;

  const sig = cardSignature(comment);
  if (card.sig !== sig) {
    updateStatic(card, comment);
    card.sig = sig;
  }

  // resolved/pending can flip on their own, independent of everything above.
  // A comment resolved by someone else while this card was open collapses too.
  if (!comment.resolved) card.expanded = false;
  const collapsed = Boolean(comment.resolved) && !card.expanded;
  card.el.className =
    (comment.resolved || comment.pending ? "resolved" : "") + (collapsed ? " collapsed" : "");
  if (collapsed) card.summary.textContent = summaryOf(comment);
  card.resolveBtn.textContent = comment.resolved ? "Reopen" : "Resolve";
  // Deletability can change too: an optimistic comment gets `deletable: true`
  // once the server confirms it, and canModerate can arrive after the first render.
  card.deleteButton.hidden = !canDelete(comment);

  // Replies are matched by object, not id: a reply's id changes from temp_id
  // to the server's (the same Object.assign-in-place pattern as a comment).
  // They are normally append-only, but a server error rolls an optimistic
  // reply back out of the list, so the list is redrawn whenever what is drawn
  // stops matching what is there.
  const drawn = card.repliesDrawn;
  const same =
    drawn.length <= comment.replies.length && drawn.every((reply, i) => reply === comment.replies[i]);
  if (!same) {
    card.repliesList.innerHTML = "";
    drawn.length = 0;
  }
  for (const reply of comment.replies.slice(drawn.length)) {
    const item = document.createElement("li");
    item.appendChild(element("span", reply.body));
    item.appendChild(document.createElement("br"));
    item.appendChild(element("small", reply.creator + " · " + stamp(reply.created)));
    card.repliesList.appendChild(item);
    drawn.push(reply);
  }
  card.repliesList.hidden = comment.replies.length === 0;

  // The name field is the one part of an open form that identity arriving
  // (or changing) should still touch.
  if (card.form) {
    card.nameInput.hidden = Boolean(identity);
    if (identity) card.nameInput.value = identity;
  }
}

function render() {
  drawTagFilter();
  const shown = comments
    .filter((comment) => [...chosenTags].every((tag) => (comment.tags || []).includes(tag)))
    .sort((a, b) => place(a) - place(b) || a.seq - b.seq);

  // A card survives being hidden by the tag filter (its draft and state stay
  // in `cards`); it is only ever dropped once its comment is gone for good.
  const alive = new Set(comments);
  for (const [comment, card] of cards) {
    if (!alive.has(comment)) {
      card.el.remove();
      cards.delete(comment);
    }
  }

  const elements = shown.map((comment) => {
    let card = cards.get(comment);
    if (!card) {
      card = makeCard(comment);
      cards.set(comment, card);
    }
    updateCard(card, comment);
    return card.el;
  });

  // Drop whatever is currently in `box` but not in this render's list (a
  // comment now filtered out, or deleted), then walk the rest into order.
  // Only a card that is out of place is moved: moving a node detaches and
  // reattaches it, which would blur a reply someone is in the middle of
  // typing every time another comment arrived.
  const keep = new Set(elements);
  for (const child of [...box.children]) {
    if (!keep.has(child)) box.removeChild(child);
  }
  elements.forEach((el, i) => {
    if (box.children[i] !== el) box.insertBefore(el, box.children[i] || null);
  });

  const open = comments.filter((comment) => !comment.resolved).length;
  countEl.textContent = comments.length ? `${open} open · ${comments.length} total` : "";
  // The instruction is onboarding, not a caption: it goes once there is
  // something in the column to read.
  document.getElementById("hint").hidden = comments.length > 0;
}

/* ------------------------------------------------------------------ wire */

// A socket that drops loses nothing: comments still post over the REST route
// and the hello on reconnect resends the list. What it does cost is seeing
// other people's comments as they arrive, which is worth saying -- but only
// once it has lasted longer than a blip, and only while it is true.
let dropped = null;

function setConnected(up) {
  clearTimeout(dropped);
  if (up) {
    connEl.hidden = true;
    return;
  }
  dropped = setTimeout(() => {
    connEl.hidden = false;
  }, 2000);
}

function send(message) {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(message));
    return;
  }
  // The socket is down; fall back to the REST route so the write is not lost.
  fetch(`/api/documents/${SLUG}/comments`, {
    method: "POST",
    headers: { "content-type": "application/json", ...SHELL_HEADERS },
    body: JSON.stringify(message),
  })
    .then((response) => response.json())
    .then(receive)
    .catch(() => setConnected(false));
}

function receive(event) {
  if (event.type === "hello") {
    comments = event.comments;
    commentsReady = true;
    reanchor();
    return;
  }
  if (event.type === "error") {
    // Roll the optimistic row back.
    if (event.temp_id) {
      comments = comments.filter((comment) => comment.temp_id !== event.temp_id);
      comments.forEach((comment) => {
        comment.replies = comment.replies.filter((reply) => reply.temp_id !== event.temp_id);
      });
      render();
      applyHighlights();
    }
    // A refused delete or resolve was applied optimistically before the
    // server had a say; the comment named in the refusal is re-fetched from
    // the REST route and the whole list replaced, the same way the initial
    // hello populates it, so the optimistic change is rolled back.
    if (event.comment_id) {
      fetch(`/api/documents/${SLUG}/comments`)
        .then((response) => response.json())
        .then((data) => receive({ type: "hello", comments: data.comments }))
        .catch(() => {});
    }
    alert(event.message);
    return;
  }
  // The shared source: the state of a session as it stands, one more change to
  // it, or how many people are in it. All three are meaningless unless this
  // browser is editing, and are ignored until it is.
  if (event.type === "y-state") {
    session?.start(event, savedSource);
    peers(event.count || 1);
    return;
  }
  if (event.type === "y-update") {
    session?.apply(event.update);
    return;
  }
  if (event.type === "y-snapshot") {
    // The session's history grew long enough that a latecomer would have to
    // replay all of it. This browser sends the whole state instead, and the
    // server keeps that one update in place of the rest.
    if (session) send({ type: "y-update", update: session.snapshot(), replace: true });
    return;
  }
  if (event.type === "y-peers") {
    peers(event.count || 1);
    return;
  }

  // Someone published a new version of this document -- another reader saving
  // in their own editor, or the same document republished from the command
  // line. What to do about it depends on what this reader is in the middle of.
  if (event.type === "published") {
    if (!event.sha || event.sha === shownSHA) return;
    // In a shared session everyone is typing into one source, so a save by any
    // of them publishes what all of them have. That is not a conflict, and
    // warning about it would make collaborating feel like colliding: the
    // others simply move on to the version that now exists.
    if (session) {
      shownSHA = event.sha;
      fetch(`/api/documents/${SLUG}/source`, { headers: SHELL_HEADERS })
        .then((response) => (response.ok ? response.json() : Promise.reject(new Error("no source"))))
        .then((payload) => {
          baseSHA = payload.sha || "";
          if ((payload.source || "") === session.text()) {
            // What was published is what this browser is looking at.
            savedSource = payload.source || "";
            saveButton.disabled = true;
            say("saved");
            return;
          }
          // Published from somewhere outside the session -- the command line,
          // or a browser that was not in it. Their work is not this page's to
          // discard, and the save that would discard it is refused anyway.
          say("a newer version was published — reload before saving", true);
        })
        .catch(() => {});
      return;
    }
    if (dirty()) {
      // Their work is not this page's to discard, and the save that would
      // discard theirs is refused anyway. Say so now rather than at save time.
      say("a newer version was published — reload before saving", true);
      return;
    }
    shownSHA = event.sha;
    if (editing) {
      // The source this editor is showing is no longer the document's, and
      // nothing has been typed over it, so take the new one.
      fetch(`/api/documents/${SLUG}/source`, { headers: SHELL_HEADERS })
        .then((response) => (response.ok ? response.json() : Promise.reject(new Error("no source"))))
        .then(async (payload) => {
          sourceBox.value = savedSource = payload.source || "";
          baseSHA = payload.sha || "";
          // After the repaint, which ends by saying whether there is anything
          // unsaved -- and would otherwise write over this.
          await paintPreview();
          say("updated by someone else");
        })
        .catch(() => {});
      return;
    }
    // Reading rather than editing: show the version that now exists. The
    // frame republishes its text on load, which re-anchors every comment.
    frame.src = `${docsOrigin}/raw/${SLUG}/${event.sha}.html`;
    return;
  }

  if (event.type === "comment") {
    const local = comments.find((comment) => comment.temp_id === event.temp_id);
    // Broadcasts carry no `deletable` field, so the caller's own comment,
    // reconciled here from its optimistic placeholder, stays deletable by
    // this browser regardless of what the server sent back.
    if (local) Object.assign(local, event.comment, { temp_id: undefined, pending: false, deletable: true });
    else if (!comments.some((comment) => comment.id === event.comment.id)) {
      comments.push(event.comment);
      // Someone else's comment: anchor just this one against the cached text.
      anchorAll(docText || "", [event.comment], docText === null ? null : docView);
    }
    render();
    applyHighlights();
    return;
  }
  if (event.type === "reply") {
    const comment = comments.find((item) => item.id === event.comment_id);
    if (!comment) return;
    const local = comment.replies.find((reply) => reply.temp_id === event.temp_id);
    if (local) Object.assign(local, event.reply, { temp_id: undefined });
    else if (!comment.replies.some((reply) => reply.id === event.reply.id)) {
      comment.replies.push(event.reply);
    }
    render();
    return;
  }
  if (event.type === "delete") {
    comments = comments.filter((item) => item.id !== event.comment_id);
    render();
    applyHighlights();
    return;
  }
  if (event.type === "resolve") {
    const comment = comments.find((item) => item.id === event.comment_id);
    if (!comment) return;
    comment.resolved = event.resolved;
    comment.resolved_at = event.resolved_at;
    render();
    applyHighlights();
  }
}

function connect() {
  socket = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws/${SLUG}`);
  socket.onopen = () => {
    backoff = 500;
    setConnected(true);
  };
  socket.onmessage = (event) => receive(JSON.parse(event.data));
  socket.onclose = () => {
    setConnected(false);
    // The hello on reconnect resends the full list, so a missed broadcast
    // during the gap heals itself.
    setTimeout(connect, backoff);
    backoff = Math.min(backoff * 2, 15000);
  };
  socket.onerror = () => socket.close();
}

/* ------------------------------------------------------------------ boot */

/* ------------------------------------------------------------------- link */

// The address bar already holds the link; the button spares the reader from
// selecting it. The icon turns into a tick for a moment, since a copy is
// otherwise invisible.
const copyButton = document.getElementById("copyLink");
const linkIcon = copyButton.innerHTML;
let linkTimer;

copyButton.onclick = async () => {
  try {
    await navigator.clipboard.writeText(location.href);
  } catch {
    return; // clipboard blocked; the address bar is still there
  }
  copyButton.classList.add("done");
  copyButton.innerHTML =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>';
  clearTimeout(linkTimer);
  linkTimer = setTimeout(() => {
    copyButton.classList.remove("done");
    copyButton.innerHTML = linkIcon;
  }, 1500);
};

// A GET can be forced onto a signed-in reader cross-site, so signing out is a
// POST, carrying the same header every other state change does.
document.getElementById("signOut").onclick = async () => {
  await fetch("/auth/logout", { method: "POST", headers: SHELL_HEADERS }).catch(() => {});
  location.reload();
};

/* ------------------------------------------------------------------ tools */

// The tool decides what a selection becomes. Highlighting is the one that
// needs no words, so it is made on the spot; the rest open the dialog with
// their kind already settled.
let tool = "commenting";
const toolButtons = [...document.querySelectorAll(".tool")];

// Box draws on a figure, so a document with no figures has nothing for it to
// do. Saying so is better than a button that silently does nothing.
function offerBox(any) {
  const box = toolButtons.find((button) => button.dataset.tool === "region");
  if (!box) return;
  box.disabled = !any;
  box.title = any ? "Drag a box on a figure" : "This document has no figures to draw on";
  // Leaving it chosen would arm a drag that can never start.
  if (!any && tool === "region") toolButtons[0].click();
}

for (const button of toolButtons) {
  button.onclick = () => {
    tool = button.dataset.tool;
    tell({ type: "tool", tool });
    for (const other of toolButtons) {
      other.setAttribute("aria-pressed", String(other === button));
      other.className = other === button ? "tool" : "tool outline";
    }
    bar.textContent = button.getAttribute("aria-label");
  };
}
// Everything but the pressed one starts outlined.
toolButtons.filter((b) => b.dataset.tool !== tool).forEach((b) => (b.className = "tool outline"));

bar.onclick = () => {
  if (!pending) return;
  bar.style.display = "none";

  if (tool === "highlighting") {
    // No dialog: the passage is the whole annotation.
    submitAnnotation({ motivation: "highlighting", body: "", tags: [] });
    return;
  }

  // If not logged in, ask how they want to identify
  if (!identity) {
    const identityDialog = document.getElementById("identityDialog");
    identityDialog.showModal();
    return;
  }

  showCommentDialog();
};

function showCommentDialog() {
  document.getElementById("selectedQuote").textContent = pending.region
    ? `Figure ${pending.region.image_index + 1}`
    : "“" + pending.exact + "”";
  document.getElementById("bodyLabel").textContent = "Comment";
  dialog.showModal();
  document.getElementById("body").focus();
}

// One path for every kind, whether it came from the dialog or straight from
// the highlight tool.
function submitAnnotation({ motivation, body, tags }) {
  if (!pending) return;
  const creator = document.getElementById("author").value;
  if (!identity) localStorage.setItem("komodoc-author", creator);
  const temp_id = crypto.randomUUID();
  const optimistic = {
    id: temp_id,
    temp_id,
    seq: Number.MAX_SAFE_INTEGER,
    ...pending,
    motivation,
    body,
    tags,
    creator: creator || "Anonymous",
    created: new Date().toISOString(),
    resolved: false,
    resolved_at: null,
    replies: [],
    pending: true,
  };
  // Draw it before the round trip; `receive` reconciles it by temp_id.
  anchorAll(docText || "", [optimistic], docText === null ? null : docView);
  comments.push(optimistic);
  render();
  applyHighlights();
  send({ type: "comment", ...pending, motivation, body, tags, creator, temp_id });
  pending = null;
}

document.getElementById("commentForm").onsubmit = (event) => {
  event.preventDefault();
  submitAnnotation({
    motivation: tool === "region" ? "commenting" : tool,
    body: document.getElementById("body").value,
    tags: parseTags(document.getElementById("tags").value),
  });
  document.getElementById("body").value = "";
  document.getElementById("tags").value = "";
  dialog.close();
};

// "methods, typo" becomes ["methods", "typo"]. The server normalises again,
// so this only has to be reasonable.
function parseTags(value) {
  return value
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
}

document.getElementById("author").value = localStorage.getItem("komodoc-author") || "Anonymous";

// Identity dialog handlers
const identityDialog = document.getElementById("identityDialog");
document.getElementById("signInChoice").onclick = () => {
  identityDialog.close();
  window.location.href = `/auth/login?next=${encodeURIComponent(location.pathname)}`;
};

document.getElementById("nameChoice").onclick = () => {
  identityDialog.close();
  showCommentDialog();
};

// Document bytes and comments are fetched in parallel; whichever lands last
// triggers the single anchoring pass.
connect();
fetch(`/api/documents/${SLUG}`)
  .then((response) => (response.ok ? response.json() : Promise.reject(new Error("not found"))))
  .then((doc) => {
    document.title = `${doc.title} · Komodoc`;
    document.getElementById("docTitle").textContent = doc.title;
    // Documents live on their own origin, which the deployment names.
    docsOrigin = doc.docs_origin || location.origin;
    shownSHA = doc.sha;
    frame.src = `${docsOrigin}/raw/${SLUG}/${doc.sha}.html`;
    // Whether this caller owns the document, which unlocks deleting anyone's
    // comment on it. This can arrive after comments have already rendered
    // once without it, so redraw to pick it up.
    canModerate = Boolean(doc.can_moderate);
    docTitle = doc.title;
    render();
    offerEditing(doc);
  })
  .catch(() => {
    document.getElementById("docTitle").textContent = "Document not found";
  });

/* ---------------------------------------------------------------- editing */

// A document published from markdown can be edited here by whoever may
// replace it. Everything below is inert for everyone else and for every
// document that has no source: the buttons stay hidden and the renderer is
// never fetched.

const readerPane = document.getElementById("reader");
const editorPane = document.getElementById("editorPane");
const sourceBox = document.getElementById("editorSource");
const sourceButton = document.getElementById("sourceToggle");
const previewButton = document.getElementById("previewToggle");
const commentsButton = document.getElementById("commentsToggle");
const saveButton = document.getElementById("saveDoc");
const editState = document.getElementById("editState");
const peerCount = document.getElementById("peerCount");
const linkButton = document.getElementById("linkToggle");

// Whether a click in one pane takes the other to the same place. Off by
// default -- it moves the document under the reader, which is helpful when
// asked for and startling when not -- and remembered per reader.
let linked = false;
try {
  linked = localStorage.getItem("komodoc-linked") === "1";
} catch {
  /* storage disabled; it starts off, as it does anyway */
}

let editing = false;
let savedSource = ""; // what is published, so "unsaved" is a fact not a guess
let docTitle = "";
let sourceFormat = ""; // what the document was published from, and renders as
// The version this editor opened, sent back with every save. If the document
// has moved on since -- someone else saved, or the same person in another tab
// -- the server refuses rather than letting this save discard their work.
let baseSHA = "";
// The version the document frame is showing. A "published" broadcast naming
// this one is our own save coming back, and means nothing new.
let shownSHA = "";
// The shared source, while this browser is editing. Everyone with the document
// open in an editor is typing into the same one.
let session = null;

const dirty = () => editing && sourceBox.value !== savedSource;

// What the document is called, which is what the rendered page is titled. The
// title it was published under wins; a document that never had one is named by
// its own first heading, the way `publish` names one.
async function headingOf(source) {
  return docTitle || (await editor.titleOf(source, sourceFormat)) || "Untitled";
}

function say(text, problem = false) {
  editState.hidden = !text;
  editState.textContent = text;
  editState.className = "editstate" + (problem ? " problem" : "");
}

// Painting the preview is sending it to the frame: the draft is a document,
// and a document belongs on the documents origin, not in this page. The agent
// republishes its text from there, which lands in the "ready" handler above
// and re-anchors every comment against what was just typed.
let issued = 0;
let painted = 0;

async function paintPreview() {
  const mine = ++issued;
  const source = sourceBox.value;
  try {
    const html = await editor.render(source, await headingOf(source), sourceFormat);
    // A slower render that resolves late must not paint over a newer one.
    if (mine <= painted) return;
    painted = mine;
    tell({ type: "preview", html });
    say(dirty() ? "unsaved changes" : "saved");
  } catch (error) {
    if (mine > painted) say(error.message || "could not render", true);
  }
}

// Short enough to read as live -- the renderer takes single-digit
// milliseconds -- and long enough that a burst of typing is one render.
let previewTimer = null;
sourceBox.addEventListener("input", () => {
  saveButton.disabled = !dirty();
  say(dirty() ? "unsaved changes" : "saved");
  clearTimeout(previewTimer);
  previewTimer = setTimeout(paintPreview, 60);
});

// A caret moved in the source. Only a deliberate move counts: typing moves the
// caret constantly, and scrolling the document on every keystroke would make
// the preview unreadable.
let stepTimer = null;
function followCaret() {
  if (!linked || !editing || docText === null) return;
  clearTimeout(stepTimer);
  stepTimer = setTimeout(() => {
    const place = sync.documentPlaceFor(sourceBox.value, sourceBox.selectionStart, docText);
    if (place) {
      tell({ type: "locate", start: place.at, length: place.length });
      lost(false);
      return;
    }
    // The words at the caret are not findable in the document: a formula, a
    // table cell, a heading that renders as something else. Said rather than
    // ignored, because a lock that silently does nothing is indistinguishable
    // from one that is broken.
    lost(true);
  }, 120);
}
sourceBox.addEventListener("click", followCaret);
sourceBox.addEventListener("keyup", (event) => {
  if (event.key.startsWith("Arrow") || event.key === "PageUp" || event.key === "PageDown") followCaret();
});

// And the other way: a click in the document puts the caret on the words it
// landed on. The textarea does not scroll to its own selection, so the line is
// worked out and scrolled to by hand.
function followDocumentClick(offset) {
  if (!linked || !editing || docText === null) return;
  const at = sync.sourcePlaceFor(docText, offset, sourceBox.value);
  if (at === null) {
    lost(true);
    return;
  }
  lost(false);
  sourceBox.focus();
  sourceBox.setSelectionRange(at, at);
  const line = sourceBox.value.slice(0, at).split("\n").length;
  const lineHeight = parseFloat(getComputedStyle(sourceBox).lineHeight) || 20;
  sourceBox.scrollTop = Math.max(0, (line - 4) * lineHeight);
}

// Whether the last attempt to keep the two in step found anywhere to go. Said
// beside the lock, and cleared by the next attempt that works.
function lost(yes) {
  if (!yes) {
    if (editState.textContent === NO_MATCH) say(dirty() ? "unsaved changes" : "saved");
    return;
  }
  say(NO_MATCH, true);
}
const NO_MATCH = "no matching passage — the two are still linked";

function setLinked(on) {
  linked = on;
  linkButton.setAttribute("aria-pressed", String(on));
  try {
    localStorage.setItem("komodoc-linked", on ? "1" : "0");
  } catch {
    /* it still applies to this page */
  }
}
linkButton.addEventListener("click", () => setLinked(!linked));

addEventListener("beforeunload", (event) => {
  if (dirty()) event.preventDefault();
});

// Ctrl/Cmd-S, because everyone tries it in an editor.
addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key === "s" && editing) {
    event.preventDefault();
    save();
  }
});

// Saving is publishing a revision: the same upload the CLI makes, carrying
// the source beside the rendered page so the next edit reopens what was
// actually saved. The comments stay where they are and re-anchor.
async function save() {
  if (!dirty()) return;
  saveButton.disabled = true;
  say("saving…");
  const source = session ? session.text() : sourceBox.value;
  try {
    const title = await headingOf(source);
    const html = await editor.render(source, title, sourceFormat);
    const response = await fetch("/api/documents", {
      method: "POST",
      headers: { "content-type": "application/json", ...SHELL_HEADERS },
      body: JSON.stringify({
        title, slug: SLUG, html, source, source_format: sourceFormat, base_sha: baseSHA,
      }),
    });
    const body = await response.json().catch(() => ({}));
    if (!response.ok) {
      // A conflict is the one refusal worth spelling out: nothing was written,
      // and what to do about it is the reader's decision, not this page's.
      say(
        response.status === 409
          ? "published elsewhere while you were editing — reload to see it before saving over it"
          : body.error || `save failed (${response.status})`,
        true,
      );
      saveButton.disabled = false;
      return;
    }
    savedSource = source;
    baseSHA = shownSHA = body.sha || baseSHA;
    say("saved");
  } catch (error) {
    say(error.message || "save failed", true);
    saveButton.disabled = false;
  }
}

saveButton.addEventListener("click", save);

// Editing a document and looking at its source are two different things, and
// conflating them was a bug: hiding the source pane used to leave the session
// altogether, so the preview stopped following what was being typed -- by
// anyone, including other people. The session lasts as long as the document is
// open; the pane is a view of it, which can be folded away like any other.
function startEditing() {
  if (editing) return;
  editing = true;
  saveButton.hidden = false;
  saveButton.disabled = !dirty();
  linkButton.hidden = false;
  setLinked(linked);
  say("saved");
  openSession();
  showSource(true);
  paintPreview();
  sourceBox.focus();
}

// Whether the source pane is on the screen. Nothing else changes with it: the
// text is still shared, the preview still follows it, and a save still saves.
function showSource(on) {
  editorPane.hidden = !on;
  readerPane.classList.toggle("editing", on);
  sourceButton.setAttribute("aria-pressed", String(on));
  // A pane just appeared or went away, so what each of the others may take has
  // changed with it.
  fitPanes();
}

// The shared source. Everyone editing this document is typing into one
// document, and what each of them sees rendered is what they now have --
// rendering stays in the browser, so a session costs the deployment nothing
// beyond relaying a few dozen bytes per keystroke.
function openSession() {
  if (session) return;
  session = collab.join({
    send,
    textarea: sourceBox,
    // Someone else's typing arrived: the preview follows it, and the save
    // button follows whether the shared source still matches what is
    // published.
    onText: () => {
      saveButton.disabled = !dirty();
      clearTimeout(previewTimer);
      previewTimer = setTimeout(paintPreview, 60);
    },
    onPeers: peers,
  });
  send({ type: "y-open" });
}

// Leaving the page. The session on the server ends when the last person in it
// disconnects, which the socket closing does on its own; this is only this
// browser letting go of its half.
function closeSession() {
  session?.leave();
  session = null;
}
addEventListener("pagehide", closeSession);

// How many people have this document open in an editor, said only when it is
// more than one: alone is the ordinary case and needs no notice.
function peers(count) {
  if (!editing) return;
  peerCount.hidden = count < 2;
  peerCount.textContent = count < 2 ? "" : `${count} editing`;
}

// The two panes beside the source fold away rather than unmount, so the frame
// keeps its document and the sidebar keeps its cards and its scroll.
// The three panes, and the buttons that show and hide them. Each button says
// whether its pane is showing; the group is the answer to "what am I looking
// at", which is a different question from what the annotation tools answer.
const showing = () => ({
  source: readerPane.classList.contains("editing"),
  preview: !readerPane.classList.contains("no-preview"),
  comments: !readerPane.classList.contains("no-comments"),
});

const paneButtons = {
  source: sourceButton,
  preview: previewButton,
  comments: commentsButton,
};

// Hiding the last one would leave the window empty, which is never what was
// meant: the button refuses, and stays pressed.
function togglePane(which) {
  const shown = showing();
  const turningOff = shown[which];
  if (turningOff && Object.values(shown).filter(Boolean).length === 1) return;

  if (which === "source") {
    showSource(!turningOff);
    return;
  }
  readerPane.classList.toggle(which === "preview" ? "no-preview" : "no-comments", turningOff);
  syncPaneButtons();
  fitPanes();
}

function syncPaneButtons() {
  const shown = showing();
  for (const [which, button] of Object.entries(paneButtons)) {
    button.setAttribute("aria-pressed", String(shown[which]));
  }
}

for (const [which, button] of Object.entries(paneButtons)) {
  button.addEventListener("click", () => togglePane(which));
}
syncPaneButtons();

// Offered only when there is something to edit and someone allowed to edit it.
// The source is asked for once, and the renderer starts downloading with it,
// so clicking Edit does not then wait for five megabytes.
function offerEditing(doc) {
  // can_edit, not can_moderate: moderating is about this document's comments,
  // editing is about replacing the document, and a reserved example is
  // everyone's to annotate and the deployment's to change.
  const mayEdit = doc.can_edit === undefined ? doc.can_moderate : doc.can_edit;
  if (!mayEdit || !doc.source_format) return;
  // A document is editable here only if this deployment can render what it
  // was written in. Markdown always; typst when its renderer was built, which
  // is thirty megabytes of WebAssembly and so optional. A document whose
  // format this deployment cannot render is read and annotated as usual, and
  // says so rather than offering an editor that could not save.
  const renderers = Array.isArray(doc.renderers) ? doc.renderers : ["markdown"];
  if (!renderers.includes(doc.source_format)) {
    say(`${doc.source_format} documents are edited with komodoc serve`);
    return;
  }
  sourceFormat = doc.source_format;
  fetch(`/api/documents/${SLUG}/source`, { headers: SHELL_HEADERS })
    .then((response) => (response.ok ? response.json() : Promise.reject(new Error("no source"))))
    .then((payload) => {
      sourceBox.value = savedSource = payload.source || "";
      baseSHA = payload.sha || "";
      sourceButton.hidden = false;
      editor.warm(sourceFormat);
      // A document with a source opens ready to be worked on: that is what
      // its author came for.
      startEditing();
    })
    .catch(() => {
      /* not editable here; the reader is unchanged */
    });
}

// Who you are decides how your comments are signed. Signed in, the name is
// your GitHub login and there is nothing to type; the server ignores anything
// sent in its place. Signed out, you type a name, unless this deployment
// requires an account to comment at all.
fetch("/api/me")
  .then((response) => response.json())
  .then((me) => {
    identity = me.login || "";
    // The nav shows who you are on every page, landing or document.
    document.getElementById("who").hidden = !identity;
    document.getElementById("who").textContent = identity ? "@" + identity : "";
    const signIn = document.getElementById("signIn");
    signIn.hidden = Boolean(identity) || !me.can_sign_in;
    signIn.href = `/auth/login?next=${encodeURIComponent(location.pathname)}`;
    document.getElementById("signOut").hidden = !identity;
    const name = document.getElementById("author");
    const label = document.getElementById("nameLabel");

    if (identity) {
      name.value = identity;
      name.hidden = label.hidden = true;
      const note = document.getElementById("signedInAs");
      note.textContent = "commenting as @" + identity;
      note.hidden = false;
      render(); // reply forms lose their name field too
      return;
    }
    if (me.comments_need_login) {
      name.hidden = label.hidden = true;
      document.getElementById("signInToComment").hidden = false;
      document.getElementById("commentForm").querySelector('button[type="submit"]').disabled = true;
    }
  })
  .catch(() => {});

/* ------------------------------------------------------------------- grip */

// Both splits are draggable, and they work the same way: the pane either side
// of a separator can be made wider or narrower, within limits -- never so wide
// that what it sits beside is a strip, never so narrow that it cannot be read.
// Each width is remembered per reader, not per document.
//
// The two differ only in which edge they are measured from. The comment pane
// is sized from the right of the window; the source pane, on the far side of
// the document, from the left.
const reader = document.querySelector("main.reader");
const gripGuide = document.getElementById("gripGuide");

// What the document keeps between the two of them. Each pane's own ceiling
// bounds it against the whole window, which says nothing about the two of them
// together: dragged wide one after the other, they would leave the document a
// sliver. This is the width that is not theirs to take.
const DOCUMENT_MIN = 360;

const panes = {
  sidebar: {
    grip: document.getElementById("grip"),
    property: "--komodoc-sidebar",
    key: "komodoc-sidebar",
    min: 240, // px, about the narrowest a comment card reads at
    max: 0.6, // of the window, so what it sits beside always keeps 40%
    // Distance from the right edge, since the comments are the last column.
    widthAt: (x) => innerWidth - x,
    edgeAt: (width) => innerWidth - width,
    // Which arrow key grows this pane: the comments grow leftwards.
    grows: "ArrowLeft",
    shown: () => !reader.classList.contains("no-comments"),
  },
  editor: {
    grip: document.getElementById("editorGrip"),
    property: "--komodoc-editor",
    key: "komodoc-editor",
    min: 320, // px, about the narrowest a line of source reads at
    max: 0.7,
    widthAt: (x) => x,
    edgeAt: (width) => width,
    grows: "ArrowRight",
    shown: () => reader.classList.contains("editing"),
  },
};

function clamp(pane, width) {
  const other = pane === panes.sidebar ? panes.editor : panes.sidebar;
  // What is left once the other pane has what it has and the document has what
  // it must keep -- and nothing is kept for a document that is folded away.
  const separators = Object.values(panes)
    .filter((each) => each.grip && each.shown())
    .reduce((total, each) => total + each.grip.offsetWidth, 0);
  const spare =
    innerWidth -
    separators -
    (other.shown() ? widthOf(other) : 0) -
    (reader.classList.contains("no-preview") ? 0 : DOCUMENT_MIN);
  // The minimum still wins on a window too narrow for any of this: a pane too
  // small to read is not an improvement on a document too small to read.
  const ceiling = Math.max(pane.min, Math.min(innerWidth * pane.max, spare));
  return Math.round(Math.max(pane.min, Math.min(width, ceiling)));
}

function setWidth(pane, width) {
  const limit = clamp(pane, width);
  reader.style.setProperty(pane.property, limit + "px");
  return limit;
}

// The width this pane currently has, read back from the element rather than
// from the variable, so a pane that has never been dragged reports the width
// the stylesheet gave it.
function widthOf(pane) {
  const current = parseInt(getComputedStyle(reader).getPropertyValue(pane.property), 10);
  return Number.isFinite(current) ? current : pane.min;
}

function remember(pane, width) {
  try {
    localStorage.setItem(pane.key, String(width));
  } catch {
    /* the width still applies to this page */
  }
}

for (const pane of Object.values(panes)) {
  try {
    const stored = Number(localStorage.getItem(pane.key));
    if (stored) setWidth(pane, stored);
  } catch {
    /* storage disabled; the default width stands */
  }

  const grip = pane.grip;
  if (!grip) continue;

  // While dragging, only the guide line moves; the real width -- and the
  // iframe reflow that comes with it -- is applied once, on release.
  const guideAt = (width) => {
    if (gripGuide) gripGuide.style.left = pane.edgeAt(clamp(pane, width)) + "px";
  };

  grip.addEventListener("pointerdown", (event) => {
    event.preventDefault();
    grip.setPointerCapture(event.pointerId);
    grip.classList.add("dragging");
    // The frame swallows pointer events while it has them, so it is deafened
    // for the duration of the drag rather than the drag being lost over it.
    frame.style.pointerEvents = "none";
    if (gripGuide) {
      guideAt(pane.widthAt(event.clientX));
      gripGuide.hidden = false;
    }
  });

  grip.addEventListener("pointermove", (event) => {
    if (!grip.hasPointerCapture(event.pointerId)) return;
    guideAt(pane.widthAt(event.clientX));
  });

  const finish = (event) => {
    if (!grip.hasPointerCapture(event.pointerId)) return;
    grip.releasePointerCapture(event.pointerId);
    grip.classList.remove("dragging");
    frame.style.pointerEvents = "";
    if (gripGuide) gripGuide.hidden = true;
    remember(pane, setWidth(pane, pane.widthAt(event.clientX)));
  };
  grip.addEventListener("pointerup", finish);
  grip.addEventListener("pointercancel", finish);

  // Keyboard: the separator is focusable, so it should move without a pointer.
  grip.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
    event.preventDefault();
    const step = event.key === pane.grows ? 24 : -24;
    remember(pane, setWidth(pane, widthOf(pane) + step));
  });
}

// Re-clamped whenever the room changes: a narrower window, the editor
// opening, or a pane folding away. Each is set from what it has now, so the
// only thing that moves is a pane that no longer fits.
function fitPanes() {
  for (const pane of Object.values(panes)) setWidth(pane, widthOf(pane));
}
addEventListener("resize", fitPanes);

// When this document was last opened, kept per reader so the landing page can
// sort by it. Nothing about it leaves the browser.
try {
  const seen = JSON.parse(localStorage.getItem("komodoc-viewed") || "{}");
  seen[SLUG] = new Date().toISOString();
  localStorage.setItem("komodoc-viewed", JSON.stringify(seen));
} catch {
  /* storage disabled; the column will read "never" */
}
