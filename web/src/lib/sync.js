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

// How far to look for something findable when the caret is somewhere that has
// no words of its own. Three lines either way reaches the paragraph around a
// formula, a fenced block, or the blank line between two paragraphs, and stops
// well short of a different section.
const NEARBY_LINES = 3;

// The places worth trying, nearest first: where the caret actually is, then
// the start of each line around it. A caret on a blank line, in a formula or
// in a code fence has nothing to match on -- but the prose beside it does, and
// that is what the reader is looking at.
function* nearby(text, caret) {
  yield caret;
  const starts = [];
  for (let at = 0; at !== -1; at = text.indexOf("\n", at + 1)) starts.push(at === 0 ? 0 : at + 1);
  const here = starts.findIndex((start, index) => start <= caret && (starts[index + 1] ?? Infinity) > caret);
  if (here === -1) return;
  for (let step = 1; step <= NEARBY_LINES; step++) {
    // Backwards first: a formula or a fence belongs to the prose that
    // introduced it more often than to what follows.
    if (starts[here - step] !== undefined) yield starts[here - step];
    if (starts[here + step] !== undefined) yield starts[here + step];
  }
}

// Where in the document the caret in the source is pointing. `rendered` is the
// text the frame published; the offset returned is into that text, which is
// what the frame anchors everything else by.
export function documentPlaceFor(source, caret, rendered) {
  // The rendered text is searched flattened, and flattening only ever replaces
  // one character with one space, so the offset means the same in both.
  const haystack = flatten(rendered);
  for (const at of nearby(source, caret)) {
    const wanted = phrase(source, at, true);
    if (!wanted) continue;
    const found = findOnce(haystack, wanted);
    if (found) return found;
  }
  return null;
}

// And the other way: where in the source a place in the document is, as an
// offset for a caret rather than a scroll. The rendered text has no lines to
// speak of -- it is one long run -- so what is tried instead is a little
// further along it each time, which lands past whatever could not be matched.
export function sourcePlaceFor(rendered, at, source) {
  const haystack = flatten(source);
  for (let step = 0; step <= NEARBY_LINES; step++) {
    for (const from of step === 0 ? [at] : [at - step * WINDOW, at + step * WINDOW]) {
      if (from < 0 || from >= rendered.length) continue;
      const wanted = phrase(rendered, from, false);
      if (!wanted) continue;
      const found = findOnce(haystack, wanted);
      if (found) return found.at;
    }
  }
  return null;
}
