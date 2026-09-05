// Editing the same source as someone else, at the same time.
//
// The source is a Yjs document -- a CRDT -- so two people typing in the same
// sentence converge on the same text without either of them waiting for the
// other, and without a server that has to understand what they wrote. What
// travels is a small binary update per change; what arrives is applied in
// whatever order it turns up in.
//
// The server is a relay. It holds the updates of a live session and hands them
// to whoever joins, and that is the whole of its part: it never merges, never
// interprets, and needs no Yjs of its own. The durable copy of a document is
// still the source a save stores, which is what a session is seeded from when
// nobody is editing yet.
//
// Everything here is about the *source*. The rendered preview is not shared:
// each browser renders what it now has, which costs the deployment nothing and
// means every editor sees the document as it would be published.

import * as Y from "./vendor/yjs.js";

// Updates are binary and the room's socket carries JSON, so they travel
// base64-encoded. A keystroke is a few dozen bytes either way.
const encode = (bytes) => btoa(String.fromCharCode(...bytes));
const decode = (text) => Uint8Array.from(atob(text), (c) => c.charCodeAt(0));

// A session is one document being edited by however many people have it open.
export function join({ send, textarea, onText, onPeers }) {
  const doc = new Y.Doc();
  const text = doc.getText("source");
  // Set once the server has said whether this browser is the one to seed the
  // session. Until then nothing is typed into a document that may be about to
  // be replaced by everyone else's.
  let ready = false;
  let applying = false; // guards the two directions from each other

  // Anything this browser changes is sent on; anything that arrives is applied
  // with an origin that stops it being sent straight back out.
  doc.on("update", (update, origin) => {
    if (origin === "remote") return;
    send({ type: "y-update", update: encode(update) });
  });

  // --------------------------------------------------------------- textarea
  //
  // The binding is a diff rather than a replacement: replacing the value would
  // move everyone's caret to the end on every keystroke anyone made. The
  // common prefix and suffix are what did not change, and what is between them
  // is the edit -- which is exactly what a person typing, pasting or deleting
  // a selection produces.

  function localEdit() {
    if (!ready || applying) return;
    const was = text.toString();
    const now = textarea.value;
    if (was === now) return;

    let start = 0;
    while (start < was.length && start < now.length && was[start] === now[start]) start++;
    let end = 0;
    while (
      end < was.length - start &&
      end < now.length - start &&
      was[was.length - 1 - end] === now[now.length - 1 - end]
    ) {
      end++;
    }

    applying = true;
    doc.transact(() => {
      const removed = was.length - start - end;
      if (removed > 0) text.delete(start, removed);
      const added = now.slice(start, now.length - end);
      if (added) text.insert(start, added);
    });
    applying = false;
  }

  // The other direction: someone else's edit arrives, and the caret has to end
  // up where the person holding it would expect. An edit before the caret
  // moves it; an edit after it does not.
  text.observe((event) => {
    if (applying) return;
    const value = text.toString();
    if (textarea.value === value) return;
    const caret = { start: textarea.selectionStart, end: textarea.selectionEnd };
    let at = 0;
    let shift = 0;
    for (const part of event.delta) {
      if (part.retain) at += part.retain;
      else if (part.insert) {
        if (at <= caret.start + shift) shift += part.insert.length;
        at += part.insert.length;
      } else if (part.delete) {
        if (at < caret.start + shift) shift -= Math.min(part.delete, caret.start + shift - at);
      }
    }
    applying = true;
    textarea.value = value;
    textarea.setSelectionRange(caret.start + shift, caret.end + shift);
    applying = false;
    onText(value);
  });

  textarea.addEventListener("input", localEdit);

  return {
    text: () => text.toString(),

    // What the server says when this browser joins: the updates of a session
    // already under way, and whether this is the browser that starts one.
    start(state, source) {
      applying = true;
      for (const update of state.updates || []) {
        Y.applyUpdate(doc, decode(update), "remote");
      }
      applying = false;
      ready = true;

      if (state.seed) {
        // Nobody is editing yet, so this browser turns the stored source into
        // the shared document. Exactly one browser is told to do this: two
        // seeding independently would each build their own history of the same
        // words, and merging those means seeing the document twice.
        doc.transact(() => text.insert(0, source));
      }
      const value = text.toString();
      if (textarea.value !== value) {
        applying = true;
        textarea.value = value;
        applying = false;
      }
      onText(value);
    },

    // An update from somebody else.
    apply(update) {
      Y.applyUpdate(doc, decode(update), "remote");
    },

    // The session grew a long history, and the server asked for the whole
    // state in one update so it can forget the rest.
    snapshot() {
      return encode(Y.encodeStateAsUpdate(doc));
    },

    peers(count) {
      onPeers(count);
    },

    // Editing stopped: the document stays, but nothing more is sent.
    leave() {
      textarea.removeEventListener("input", localEdit);
      doc.destroy();
    },
  };
}
