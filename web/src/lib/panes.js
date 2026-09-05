// The two splits, and how wide each pane may be.
//
// The pane either side of a separator can be made wider or narrower, within
// limits: never so wide that what it sits beside is a strip, never so narrow
// that it cannot be read. Each width is remembered per reader, not per
// document.
//
// The two differ only in which edge they are measured from. The comment pane
// is sized from the right of the window; the source pane, on the far side of
// the document, from the left.

import { read, write } from "./storage.js";

// What the document keeps between the two of them. Each pane's own ceiling
// bounds it against the whole window, which says nothing about the two of them
// together: dragged wide one after the other, they would leave the document a
// sliver. This is the width that is not theirs to take.
export const DOCUMENT_MIN = 360;

export const PANES = {
  editor: {
    key: "komodoc-editor",
    property: "--komodoc-editor",
    min: 320, // px, about the narrowest a line of source reads at
    max: 0.7,
    widthAt: (x) => x,
    edgeAt: (width) => width,
    grows: "ArrowRight", // the source grows rightwards
  },
  sidebar: {
    key: "komodoc-sidebar",
    property: "--komodoc-sidebar",
    min: 240, // px, about the narrowest a comment card reads at
    max: 0.6, // of the window, so what it sits beside always keeps 40%
    widthAt: (x) => innerWidth - x,
    edgeAt: (width) => innerWidth - width,
    grows: "ArrowLeft",
  },
};

export const storedWidth = (pane) => Number(read(pane.key, 0)) || pane.min;
export const rememberWidth = (pane, width) => write(pane.key, width);

/// What this pane may be, given what the others have and what the document
/// must keep. The minimum still wins on a window too narrow for any of this: a
/// pane too small to read is not an improvement on a document too small to
/// read.
export function clamp(pane, width, { widths, showing, separators }) {
  const other = pane === PANES.sidebar ? PANES.editor : PANES.sidebar;
  const otherName = pane === PANES.sidebar ? "source" : "comments";
  const spare =
    innerWidth -
    separators -
    (showing[otherName] ? widths[other.key] : 0) -
    (showing.preview ? DOCUMENT_MIN : 0);
  const ceiling = Math.max(pane.min, Math.min(innerWidth * pane.max, spare));
  return Math.round(Math.max(pane.min, Math.min(width, ceiling)));
}
