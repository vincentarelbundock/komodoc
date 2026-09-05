// The renderers, loaded into the editor.
//
// Both modules are the engine crate compiled to WebAssembly -- the same crate
// the command line renders with -- so the preview is the document a save would
// store, byte for byte, and an edit made here renders exactly as one made from
// the terminal. The deployment renders nothing: it stores what this browser
// produced.
//
// They are plain WebAssembly with a handful of exports rather than
// wasm-bindgen, so this is the whole of the glue: reserve memory in the
// module, write the source into it as UTF-8, call compile, read the page back
// out. The URLs carry a digest of each module's bytes and are handed to the
// page by the server, so a module cached for a year cannot outlive the loader
// that speaks to it.

const loads = {};

function urls() {
  return globalThis.KOMODOC_MODULES || {};
}

/// Whether this deployment serves a renderer for this format at all. The typst
/// module is thirty megabytes and optional, so a build may not have one.
export function available(format) {
  return Boolean(urls()[format]);
}

function load(format) {
  const url = urls()[format];
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
      // The compiler has no clock of its own, so typst's datetime.today() is
      // whatever this tab says it is.
      const now = new Date();
      if (wasm.set_today) wasm.set_today(now.getFullYear(), now.getMonth() + 1, now.getDate());
      return wasm;
    });
  return loads[format];
}

// Each string is written into the module's memory and passed as a (pointer,
// length) pair. The module's memory can be replaced when it grows, so a view
// of it is taken after every call that might have grown it, never held across
// one.
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

/// Renders the source into the page a save would store. A document that does
/// not compile is an ordinary state of an editor, so a diagnostic comes back
/// as a thrown error carrying the compiler's own message.
export async function render(source, title, format) {
  const wasm = await load(format);
  const { text, ok } = call(wasm, "compile", source, title);
  if (!ok) throw new Error(text);
  return text;
}

/// The document's first heading, which names a document that was never given a
/// title of its own.
export async function titleOf(source, format) {
  const wasm = await load(format);
  return call(wasm, "title_of", source).text;
}

/// Starts the download before anyone asks to render, so the wait for the
/// renderer overlaps with reading the document rather than following it.
export function warm(format) {
  load(format).catch(() => {
    /* reported when something is actually rendered */
  });
}
