// The lock, over every place a caret can be in a real document.
//
// "Keep in step" is a heuristic: it takes the words at the caret and looks for
// them in the other pane. What matters is not that it is clever but that it
// almost never comes up empty -- a lock that says "nothing to jump to" every
// few clicks reads as broken, whatever it says about itself. It used to fail
// at 6% of the positions in the markdown example and 12% in the typst one,
// because it only ever looked at the caret's own line: a blank line between
// paragraphs, a formula and a fenced block have no words to find.
//
// Run by `make test`, against the examples as they are actually published.
import { readFileSync } from "node:fs";
import { documentPlaceFor, sourcePlaceFor } from "../src/lib/sync.js";
import { renderMarkdown, renderTypst } from "./render.js";

// A caret every few characters is enough to catch a whole line going missing,
// and keeps this a second rather than a minute.
const STEP = 3;
// What is tolerated. Zero would be a promise this cannot keep for every
// document anyone writes; a document where one position in fifty finds nothing
// is still a lock that works.
const ALLOWED = 0.02;

const EXAMPLES = [
  ["examples/regression-tables.md", renderMarkdown],
  ["examples/intervals.typ", renderTypst],
];

let bad = false;
for (const [file, render] of EXAMPLES) {
  const source = readFileSync(new URL(`../../${file}`, import.meta.url), "utf8");
  const rendered = await render(source, file);
  if (rendered === null) {
    console.log(`sync: ${file} is not built; run \`make examples\``);
    continue;
  }

  let missed = 0;
  let tried = 0;
  const kinds = {};
  for (let at = 0; at < source.length; at += STEP) {
    tried++;
    if (documentPlaceFor(source, at, rendered)) continue;
    missed++;
    const line = source.slice(source.lastIndexOf("\n", Math.max(0, at - 1)) + 1).split("\n")[0];
    const kind = line.trim() === "" ? "blank line" : /^\s*[#=]/.test(line) ? "heading" : "prose";
    kinds[kind] = (kinds[kind] || 0) + 1;
  }

  // And the other way, from a place in the document back to the source.
  let back = 0;
  let backTried = 0;
  for (let at = 0; at < rendered.length; at += STEP) {
    backTried++;
    if (sourcePlaceFor(rendered, at, source) === null) back++;
  }

  const rate = missed / tried;
  const backRate = back / backTried;
  console.log(
    `sync: ${file} — ${(rate * 100).toFixed(1)}% of carets and ${(backRate * 100).toFixed(1)}% of document places find nowhere`,
  );
  if (rate > ALLOWED || backRate > ALLOWED) {
    bad = true;
    console.error(`  over the ${(ALLOWED * 100).toFixed(0)}% this is allowed to be: ${JSON.stringify(kinds)}`);
  }
}

if (bad) process.exit(1);
