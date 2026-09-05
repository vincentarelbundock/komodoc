// Editing the same source as someone else, at the same time.
//
// The source is a Yjs document -- a CRDT -- so two people typing in the same
// sentence converge on the same text without either of them waiting for the
// other, and without a server that has to understand what they wrote. What
// travels is a small binary update per change, relayed by the room.
//
// The server is a relay and nothing more: it holds the updates of a live
// session and hands them to whoever joins. It never merges and never
// interprets. The durable copy of a document is still the source a save
// stores, which is what a session is seeded from when nobody is editing yet.
//
// Everything here is about the source. The rendered preview is not shared:
// each browser renders what it now has, which costs the deployment nothing and
// means every editor sees the document as it would be published.

import * as Y from "yjs";
import { Awareness, encodeAwarenessUpdate, applyAwarenessUpdate } from "y-protocols/awareness.js";

// Updates are binary and the room's socket carries JSON, so they travel
// base64-encoded. A keystroke is a few dozen bytes either way.
const encode = (bytes) => {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
};
const decode = (text) => Uint8Array.from(atob(text), (c) => c.charCodeAt(0));

// Someone else's caret is drawn in a colour of their own. Picked from a fixed
// set rather than at random so two people in a session rarely look alike, and
// so a person keeps the same colour for as long as they are in it.
const COLOURS = ["#2f5bd0", "#c2410c", "#15803d", "#7c3aed", "#be123c", "#0e7490"];

/// Joins the session for this document. `send` puts a message on the room's
/// socket; `onPeers` is told how many people are in the session, which is worth
/// saying only when it is more than one.
export function join({ send, onPeers, name }) {
  const doc = new Y.Doc();
  const text = doc.getText("source");
  const awareness = new Awareness(doc);
  // Set once the server has said whether this browser is the one to seed the
  // session. Until then nothing is typed into a document that may be about to
  // be replaced by everyone else's.
  let ready = false;

  awareness.setLocalStateField("user", {
    name: name || "Anonymous",
    color: COLOURS[Math.floor(Math.random() * COLOURS.length)],
  });

  // Anything this browser changes is sent on; anything that arrives is applied
  // with an origin that stops it being sent straight back out.
  doc.on("update", (update, origin) => {
    if (origin === "remote") return;
    send({ type: "y-update", update: encode(update) });
  });

  awareness.on("update", ({ added, updated, removed }) => {
    const changed = added.concat(updated, removed);
    send({ type: "y-awareness", update: encode(encodeAwarenessUpdate(awareness, changed)) });
    onPeers?.(awareness.getStates().size);
  });

  return {
    doc,
    text,
    awareness,
    get ready() {
      return ready;
    },

    /// The session as the server describes it: either this browser seeds it
    /// from what is published, or it replays what the session has seen.
    start(state, source) {
      if (ready) return;
      ready = true;
      if (state.seed) {
        // Exactly one browser is ever told to seed. Two seeding separately
        // would each build their own history of the same words, and merging
        // those shows the document twice.
        doc.transact(() => text.insert(0, source || ""));
        return;
      }
      doc.transact(() => {
        for (const update of state.updates || []) Y.applyUpdate(doc, decode(update), "remote");
      }, "remote");
    },

    apply(update) {
      Y.applyUpdate(doc, decode(update), "remote");
    },

    applyAwareness(update) {
      applyAwarenessUpdate(awareness, decode(update), "remote");
    },

    /// The whole state of the session, sent when its history has grown long
    /// enough that a latecomer would have to replay all of it.
    snapshot() {
      return encode(Y.encodeStateAsUpdate(doc));
    },

    text_() {
      return text.toString();
    },

    /// Says who this is, for the label on their caret.
    rename(who) {
      awareness.setLocalStateField("user", { ...awareness.getLocalState()?.user, name: who });
    },

    leave() {
      awareness.destroy();
      doc.destroy();
    },
  };
}
