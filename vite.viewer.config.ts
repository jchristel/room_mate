import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

// The React viewer — an APP build, the reports config verbatim but for the
// paths. See docs/PLAN-viewer-react.md.
//
// **Served at /viewer/ until the port is finished**, beside the page it is
// replacing rather than over it. That is the whole check on a 5,568-line
// rewrite: both pages run in the same server, so every slice is compared
// against the old page on the same project, in two windows. The last slice
// makes static/index.html a shell that loads this bundle and deletes the old
// page in the same commit; this `base` goes to "/" then.
//
// Committed output, like the other two pages: a fresh clone plus `cargo run`
// serves it with no node installed, and .github/workflows/frontend.yml rebuilds
// and fails on any difference.
export default defineConfig({
  root: resolve(import.meta.dirname, "src-js/viewer/app"),
  base: "/viewer/",
  plugins: [react()],
  // Nothing to copy: tokens.css, common.js, graph.js and the renderer bundle
  // are served from static/ by axum and referenced by absolute URL, which Vite
  // leaves alone.
  publicDir: false,
  build: {
    outDir: resolve(import.meta.dirname, "static/viewer"),
    // Safe because the directory is this build's alone — never static/ itself,
    // which holds hand-written files.
    emptyOutDir: true,
    sourcemap: false,
    target: "es2022",
    rollupOptions: {
      output: {
        // Stable names, so a rebuild diffs as the same files rather than as a
        // delete plus an add.
        entryFileNames: "viewer.js",
        chunkFileNames: "[name].js",
        assetFileNames: "[name][extname]",
      },
    },
  },
});
