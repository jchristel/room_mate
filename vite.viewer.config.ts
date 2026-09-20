import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

// The React viewer — an APP build. See docs/Superseded/PLAN-viewer-react.md.
//
// **It IS the viewer**: this build writes `static/index.html`, which axum
// serves at "/". It spent the port at /viewer/, beside the hand-written page it
// replaced, so every slice could be compared against it in two windows; the
// cutover moved `base` to "/" and deleted that page.
//
// **`emptyOutDir` is FALSE and must stay false.** The out dir is `static/`
// itself, which holds hand-written files — `common.js`, `graph.js`,
// `tokens.css` — and the other two pages' committed output. Emptying it would
// delete all of them.
//
// Committed output, like the other two pages: a fresh clone plus `cargo run`
// serves it with no node installed, and .github/workflows/frontend.yml rebuilds
// and fails on any difference.
export default defineConfig({
  root: resolve(import.meta.dirname, "src-js/viewer/app"),
  base: "/",
  plugins: [react()],
  // Nothing to copy: tokens.css, common.js, graph.js and the renderer bundle
  // are served from static/ by axum and referenced by absolute URL, which Vite
  // leaves alone.
  publicDir: false,
  build: {
    outDir: resolve(import.meta.dirname, "static"),
    // NEVER true here: see the header. `static/` is shared with hand-written
    // files and the other pages' output.
    emptyOutDir: false,
    sourcemap: false,
    target: "es2022",
    rollupOptions: {
      output: {
        // Stable names, so a rebuild diffs as the same files rather than as a
        // delete plus an add.
        entryFileNames: "viewer.js",
        chunkFileNames: "viewer-[name].js",
        // `viewer.css`, not `index.css`: this directory is shared, and a file
        // named for its entry says which build owns it.
        assetFileNames: "viewer[extname]",
      },
    },
  },
});
