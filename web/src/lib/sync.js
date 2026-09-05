// Keeping the source and the document pointing at the same place.
//
// The two are different texts: one has the markup in it and the other has what
// the markup produced. Nothing maps between them exactly -- a heading in typst
// is `= Title` and in the document it is `Title`, and a formula is neither.
// What they do share is the prose, so that is the bridge: take the words at
// the place someone just clicked, and find those words in the other one.
//
// It is a heuristic, and it says so by doing nothing when it is unsure. A
// wrong jump is worse than none: the reader loses their place and has to find
// it again themselves.

// Markup, as far as this is concerned: characters that are syntax in markdown
// or typst and never part of a word. Replaced by spaces rather than removed,
// so an offset into the flattened copy is still an offset into the original.
const MARKUP = /[#*_`~=$@<>[\]()|\\{}]/g;

// How much text to take. Long enough to be somewhere in particular, short
// enough not to cross whatever was reflowed between the two.
const WINDOW = 140;

// Below this a phrase is too common to identify a place: "the interval"
// appears throughout a document about intervals.
const ENOUGH = 16;

const flatten = (text) => text.replace(MARKUP, " ").replace(/\s+/g, " ");

// phrase takes the words at `at`, reading forwards -- what was clicked on is
// what follows the click, not what precedes it. Near the end of a paragraph
// there may not be enough of them, and then it reads backwards instead.
//
// withinLine keeps the window inside one line of the source, where a newline
// usually means a new paragraph and the words either side are not adjacent in
// the document. The rendered text has no such structure to respect: typst
// writes it as one line.
function phrase(text, at, withinLine) {
  const stop = withinLine ? text.indexOf("\n", at) : -1;
  const end = Math.min(text.length, at + WINDOW, stop === -1 ? Infinity : stop);

  // Never start mid-word: half a word matches nothing. Reading forward to the
  // next whole one rather than back to the start of this one, because the
  // rendered text has no spaces at element boundaries -- a heading runs
  // straight into the paragraph under it -- so backing up can produce a word
  // that appears in neither text ("noteThe interval").
  let start = at;
  if (start > 0 && !/\s/.test(text[start - 1])) {
    while (start < end && !/\s/.test(text[start])) start++;
  }

  let taken = flatten(text.slice(start, end)).trim();
  if (taken.length >= ENOUGH) return taken;

  // Not enough ahead, so look behind: the end of a paragraph is still a place.
  let back = Math.max(0, at - WINDOW);
  if (withinLine) {
    const line = text.lastIndexOf("\n", Math.max(0, at - 1));
    if (line !== -1) back = Math.max(back, line + 1);
  }
  taken = flatten(text.slice(back, end)).trim();
  return taken.length >= ENOUGH ? taken : "";
}

// findOnce is a match that means something: a phrase appearing twice
// identifies neither place, so an ambiguous one counts as not found. The
// phrase is shortened from the right until it either matches once or runs out,
// since its tail is the part most likely to have been reflowed away.
function findOnce(haystack, needle) {
  for (let words = needle.split(" "); words.length >= 2; words = words.slice(0, -1)) {
    const candidate = words.join(" ");
    if (candidate.length < ENOUGH) break;
    const first = haystack.indexOf(candidate);
    if (first === -1) continue;
    if (haystack.indexOf(candidate, first + 1) === -1) {
      return { at: first, length: candidate.length };
    }
  }
  return null;
}

// Where in the document the caret in the source is pointing. `rendered` is the
// text the frame published; the offset returned is into that text, which is
// what the frame anchors everything else by.
export function documentPlaceFor(source, caret, rendered) {
  const wanted = phrase(source, caret, true);
  if (!wanted) return null;
  // The rendered text is searched flattened, and flattening only ever replaces
  // one character with one space, so the offset means the same in both.
  return findOnce(flatten(rendered), wanted);
}

// And the other way: where in the source a place in the document is, as an
// offset for a caret rather than a scroll.
export function sourcePlaceFor(rendered, at, source) {
  const wanted = phrase(rendered, at, false);
  if (!wanted) return null;
  const found = findOnce(flatten(source), wanted);
  return found ? found.at : null;
}
