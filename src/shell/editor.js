// The editor, inside the reader.
//
// A document published from markdown or typst keeps its source, so whoever may
// replace it can open it here rather than re-uploading a file. Rendering
// happens in this tab, in WebAssembly built from the engine crate -- the same
// crate the command line renders with -- so three things hold at once: the
// preview is the document that a save would store, an edit made here renders
// exactly as one made from the command line, and the deployment renders
// nothing at all.
//
// That last one is why this is affordable. A live preview costs no server CPU
// and no bandwidth: what changes on every keystroke stays in the tab, and only
// a save crosses the network.
//
// The preview is not shown in this page. It is sent to the document frame,
// which is on the documents origin, and painted there -- so an unpublished
// draft runs under exactly the isolation a published document does, and the
// comments re-anchor against it as it is typed.
//
// Two renderers, one interface. Both modules are plain WebAssembly with the
// same handful of exports rather than wasm-bindgen, so this is the whole of the
// glue: reserve memory in the module, write the source into it as UTF-8, call
// compile, read the page back out. Which one a document uses is what it was
// published from. The markdown module is a few hundred kilobytes; the typst one
// is typst itself plus the fonts it sets documents in, thirty megabytes, so it
// is only ever fetched by someone who opens a typst document to edit, and both
// are served immutable: once, per browser, per build.

const MODULES = {
  markdown: "/wasm/markdown.wasm",
  typst: "/wasm/typst.wasm",
};

const loads = {}; // each module's load, started once and shared

// load fetches and instantiates one renderer. Streaming, so it compiles as it
// downloads rather than after it.
function load(format) {
  const url = MODULES[format];
  if (!url) return Promise.reject(new Error(`no renderer for ${format}`));
  if (loads[format]) return loads[format];
  loads[format] = WebAssembly.instantiateStreaming(fetch(url), {})
    .then(({ instance }) => instance.exports)
    .catch(async (error) => {
      // Some servers do not send application/wasm, which streaming requires.
      // Falling back costs a copy of the module in memory, so it is a fallback
      // rather than the path.
      const response = await fetch(url);
      if (!response.ok) throw error;
      const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {});
      return instance.exports;
    })
    .then((wasm) => {
      // The compiler has no clock of its own; typst's datetime.today() is
      // whatever the tab says it is.
      const now = new Date();
      if (wasm.set_today) wasm.set_today(now.getFullYear(), now.getMonth() + 1, now.getDate());
      return wasm;
    });
  return loads[format];
}

// call writes each string into the module's memory, invokes the export with
// their (pointer, length) pairs, and reads the result back out. The module's
// memory can be replaced when it grows, so a view of it is taken after every
// call that might have grown it, never held across one.
function call(wasm, name, ...strings) {
  const encoder = new TextEncoder();
  const written = strings.map((text) => {
    const bytes = encoder.encode(text);
    const pointer = wasm.alloc(bytes.length);
    new Uint8Array(wasm.memory.buffer, pointer, bytes.length).set(bytes);
    return { pointer, length: bytes.length };
  });
  let length;
  try {
    length = wasm[name](...written.flatMap(({ pointer, length }) => [pointer, length]));
  } finally {
    for (const { pointer, length } of written) wasm.dealloc(pointer, length);
  }
  const out = new Uint8Array(wasm.memory.buffer, wasm.output_ptr(), length);
  return { text: new TextDecoder().decode(out), ok: wasm.ok() !== 0 };
}

// render turns the source into the page a save would store: the same call the
// command line makes when a document is published from it. A document that
// does not compile is an ordinary state of an editor, so a diagnostic comes
// back as a thrown error carrying the compiler's own message.
export async function render(source, title, format) {
  const wasm = await load(format);
  const { text, ok } = call(wasm, "compile", source, title);
  if (!ok) throw new Error(text);
  return text;
}

// titleOf is the document's first heading, which names a document that was
// never given a title of its own.
export async function titleOf(source, format) {
  const wasm = await load(format);
  return call(wasm, "title_of", source).text;
}

// warm starts the download before anyone asks to render, so the wait for the
// renderer overlaps with reading the document rather than following it. It
// matters more for typst, which is a far larger module.
export function warm(format) {
  load(format).catch(() => {
    /* reported when something is actually rendered */
  });
}
