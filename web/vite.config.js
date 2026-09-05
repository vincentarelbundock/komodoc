// The reader shell, built into src/shell, which the binary embeds.
//
// Two pages rather than one application with a router: the server already
// routes -- "/" is the landing page and "/docs/<slug>" is the reader -- and a
// document's own address is the thing readers pass around. Nothing is gained
// by taking that over in JavaScript.
import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { resolve } from "node:path";

export default defineConfig({
  plugins: [svelte()],
  // The pages are served from the site root by the Go-free Rust server, which
  // knows nothing about this build beyond where the files are.
  base: "/",
  build: {
    outDir: resolve(import.meta.dirname, "../src/shell"),
    emptyOutDir: false, // the wasm modules and the README live there too
    // A stale bundle behind a fresh page is the failure this avoids: every
    // asset is named for a digest of its own bytes, so a browser holding the
    // previous build cannot be handed it, and the server can cache them for a
    // year without ever serving one that has moved on.
    assetsDir: "assets",
    rollupOptions: {
      input: {
        index: resolve(import.meta.dirname, "index.html"),
        reader: resolve(import.meta.dirname, "reader.html"),
        documentation: resolve(import.meta.dirname, "documentation.html"),
        notfound: resolve(import.meta.dirname, "404.html"),
      },
    },
    // The typst module is thirty megabytes; nothing here comes close, and a
    // warning about a 600 KB chunk would be noise.
    chunkSizeWarningLimit: 2000,
    target: "es2022",
  },
});
