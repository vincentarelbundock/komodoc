// Enough typst to read a source by: what is a heading, what is emphasised,
// what is code, and what is maths. CodeMirror has no typst mode of its own,
// and the alternative -- typst's own parser, compiled to WebAssembly -- is
// thirty megabytes that only an editor already waiting on the compiler could
// justify. This colours the source while that downloads, and is enough on its
// own for prose.
export const typstLanguage = {
  name: "typst",

  startState() {
    return { math: false };
  },

  token(stream, state) {
    if (stream.sol() && stream.match(/^\s*=+\s/)) {
      stream.skipToEnd();
      return "heading";
    }
    if (stream.match("$")) {
      state.math = !state.math;
      return "keyword";
    }
    if (state.math) {
      stream.next();
      return "keyword";
    }
    if (stream.match(/^\/\/.*/)) return "comment";
    if (stream.match(/^\/\*/)) {
      stream.skipToEnd();
      return "comment";
    }
    // A function call, a binding, an import: everything typst spells with a
    // leading hash.
    if (stream.match(/^#[A-Za-z][\w-]*/)) return "keyword";
    if (stream.match(/^\*[^*\n]+\*/)) return "strong";
    if (stream.match(/^_[^_\n]+_/)) return "emphasis";
    if (stream.match(/^`[^`\n]*`/)) return "monospace";
    if (stream.match(/^@[\w:-]+/)) return "link";
    if (stream.match(/^<[\w:-]+>/)) return "labelName";
    stream.next();
    return null;
  },

  languageData: {
    commentTokens: { line: "//", block: { open: "/*", close: "*/" } },
  },
};
