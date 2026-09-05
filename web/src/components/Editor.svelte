<script>
  // The source, in CodeMirror, shared with everyone else editing it.
  //
  // The binding is y-codemirror: the editor's document *is* the Yjs text, so
  // two people typing in the same sentence converge without either waiting for
  // the other, and each of them keeps their own caret, selection and undo
  // history. Everyone else's caret is drawn where they are, labelled with
  // their name.
  import { EditorState } from "@codemirror/state";
  import { EditorView, keymap, lineNumbers, highlightActiveLine, drawSelection } from "@codemirror/view";
  import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
  import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
  import { syntaxHighlighting, defaultHighlightStyle, StreamLanguage } from "@codemirror/language";
  import { markdown } from "@codemirror/lang-markdown";
  import { yCollab } from "y-codemirror.next";

  import { typstLanguage } from "../lib/typst-mode.js";

  let { session, format, onchange, oncaret, onsave } = $props();

  let host = $state(null);
  let view = null;

  // What a document is written in decides how it is coloured. Markdown has a
  // maintained mode; typst has the small one beside this file, which knows the
  // handful of things worth telling apart in a source you are editing.
  function language(format) {
    return format === "typst" ? StreamLanguage.define(typstLanguage) : markdown();
  }

  /// The text as it stands, which is what a save publishes.
  export function text() {
    return view ? view.state.doc.toString() : "";
  }

  /// Where the caret is, as an offset into that text.
  export function caret() {
    return view ? view.state.selection.main.head : 0;
  }

  /// Puts the caret at an offset and scrolls it into the middle of the view:
  /// what a click in the document does when the two are kept in step.
  export function goTo(at) {
    if (!view) return;
    view.dispatch({
      selection: { anchor: at },
      effects: EditorView.scrollIntoView(at, { y: "center" }),
    });
    view.focus();
  }

  export function focus() {
    view?.focus();
  }

  $effect(() => {
    if (!host || !session || view) return;
    const state = EditorState.create({
      doc: session.text.toString(),
      extensions: [
        lineNumbers(),
        history(),
        drawSelection(),
        highlightActiveLine(),
        highlightSelectionMatches(),
        syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
        language(format),
        EditorView.lineWrapping,
        keymap.of([
          // Everyone tries Ctrl/Cmd-S in an editor.
          { key: "Mod-s", preventDefault: true, run: () => (onsave?.(), true) },
          indentWithTab,
          ...defaultKeymap,
          ...historyKeymap,
          ...searchKeymap,
        ]),
        // The shared document, and everyone else's cursors in it.
        yCollab(session.text, session.awareness),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) onchange?.();
          // Only a deliberate move counts: typing moves the caret constantly,
          // and scrolling the document on every keystroke would make the
          // preview unreadable.
          if (update.selectionSet && !update.docChanged) oncaret?.();
        }),
      ],
    });
    view = new EditorView({ state, parent: host });
    view.focus();
    return () => {
      view?.destroy();
      view = null;
    };
  });
</script>

<div class="editorhost" bind:this={host}></div>
