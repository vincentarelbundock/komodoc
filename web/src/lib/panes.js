// How the window is divided, and how wide each part of it may be.
//
// There are three arrangements and one of them is showing: the source and the
// document side by side, the source alone, or the document alone. Every one of
// them shows something, so there is no arrangement to refuse and no last pane
// to protect. The comments are not part of that: they are a column that is
// either there or not, in reading and editing alike.
//
// The two kinds of pane are measured in two different units, because they are
// two different kinds of thing. The source and the document share what is left
// of the window, so their split is a fraction: half stays half when the window
// is resized or the comments open, which a pixel width cannot do. A comment
// card wants a readable width whatever the screen is, so the comment column is
// pixels. Both are remembered per reader, not per document.

import { read, write } from "./storage.js";

// What the document keeps beside a source dragged as wide as it will go.
export const DOCUMENT_MIN = 360;

// The width of a separator, which is room neither pane has.
export const GRIP = 6;

export const LAYOUTS = ["split", "source", "document"];

// The ratios a drag sticks to, and the ones the menu offers by name. The two
// are the same list on purpose: what the pointer finds and what the menu says
// should not be different places.
export const RATIOS = [
  { share: 1 / 3, says: "Wide document" },
  { share: 1 / 2, says: "Even" },
  { share: 2 / 3, says: "Wide source" },
];
// How close to a ratio the pointer has to be to be taken there.
const MAGNET = 12;

export const PANES = {
  editor: {
    name: "editor",
    key: "komodoc-source-share",
    // A share of what the source and the document have between them.
    fraction: true,
    reset: 1 / 2,
    min: 320, // px, about the narrowest a line of source reads at
    max: 0.7,
  },
  sidebar: {
    name: "sidebar",
    key: "komodoc-sidebar",
    fraction: false,
    reset: 360, // px, a comfortable comment card
    min: 240, // px, about the narrowest a comment card reads at
    max: 0.6, // of the window, so what it sits beside always keeps 40%
  },
};

export const stored = (pane) => Number(read(pane.key, 0)) || pane.reset;
export const remember = (pane, size) => write(pane.key, size);

/// What is on the screen, given the arrangement and whether the comments are
/// open. Editing decides whether there is a source at all: a document nobody
/// can edit has one arrangement, and it is the document.
export function showing({ layout, comments, editing }) {
  return {
    source: editing && (layout === "split" || layout === "source"),
    document: !editing || layout === "split" || layout === "document",
    comments,
  };
}

/// The room the separators take, which is neither pane's to use.
export function separators(state) {
  const shown = showing(state);
  return (shown.source && shown.document ? GRIP : 0) + (shown.comments ? GRIP : 0);
}

/// What the source and the document have between them: the window, less the
/// comments and every separator in it.
export function surface(state) {
  const shown = showing(state);
  return (
    innerWidth - separators(state) - (shown.comments ? clamp(PANES.sidebar, state) : 0)
  );
}

/// The comment column, in pixels, within what the rest of the window can spare.
function clampSidebar(width, state) {
  const shown = showing(state);
  const spare = innerWidth - separators(state) - (shown.source || shown.document ? DOCUMENT_MIN : 0);
  const ceiling = Math.max(PANES.sidebar.min, Math.min(innerWidth * PANES.sidebar.max, spare));
  return Math.round(Math.max(PANES.sidebar.min, Math.min(width, ceiling)));
}

/// The source's share of the surface, within what both of them can read at.
/// On a surface too narrow for both minimums the source keeps its own, since a
/// pane too small to read is not an improvement on a document too small to.
function clampShare(share, state) {
  const room = surface(state);
  if (room <= 0) return share;
  const floor = PANES.editor.min / room;
  const ceiling = Math.max(floor, Math.min(PANES.editor.max, (room - DOCUMENT_MIN) / room));
  return Math.max(floor, Math.min(share, ceiling));
}

/// A pane's size, clamped, in whatever unit that pane is kept in. `state.sizes`
/// holds what the reader last chose; passing a size asks about that one instead.
export function clamp(pane, state, size = state.sizes[pane.key]) {
  return pane.fraction ? clampShare(size, state) : clampSidebar(size, state);
}

/// And in pixels, which is what the stylesheet is given.
export function pixels(pane, state) {
  return pane.fraction
    ? Math.round(clamp(pane, state) * surface(state))
    : clamp(pane, state);
}

/// A pointer at x, as a size for this pane, in the pane's own unit. Which edge
/// it is measured from depends on where the pane sits: the comments are always
/// last, and the source is on whichever side the reader put it.
export function sizeAt(pane, x, state) {
  if (!pane.fraction) return innerWidth - x;
  const room = surface(state);
  const width = state.sourceSide === "left" ? x : rightEdge(state) - x;
  return snap(width / room, room);
}

// Near one of the named ratios, take it: a split reachable in one drag without
// having to find it by hand.
function snap(share, room) {
  for (const ratio of RATIOS) {
    if (Math.abs(share - ratio.share) * room <= MAGNET) return ratio.share;
  }
  return share;
}

/// Whether a size is sitting on one of the named ratios, so the guide line can
/// say that it is.
export function snapped(pane, size) {
  return pane.fraction && RATIOS.some((ratio) => ratio.share === size);
}

/// Where this pane's separator sits, for the line that follows the pointer
/// while it is dragged.
export function edgeAt(pane, size, state) {
  if (!pane.fraction) return innerWidth - clampSidebar(size, state);
  const width = clampShare(size, state) * surface(state);
  return state.sourceSide === "left" ? width : rightEdge(state) - width;
}

/// A step of an arrow key, in the pane's own unit: about the same distance on
/// the screen either way.
export function step(pane, state, forward) {
  const by = pane.fraction ? 24 / Math.max(surface(state), 1) : 24;
  return clamp(pane, state, state.sizes[pane.key] + (forward ? by : -by));
}

/// Which arrow key makes this pane wider, since a separator is focusable and
/// should move without a pointer.
export function grows(pane, state) {
  if (!pane.fraction) return "ArrowLeft";
  return state.sourceSide === "left" ? "ArrowRight" : "ArrowLeft";
}

// Where the source pane ends when it is on the right: the window, less the
// comments and the separator before them.
function rightEdge(state) {
  const shown = showing(state);
  return innerWidth - (shown.comments ? clamp(PANES.sidebar, state) + GRIP : 0);
}
