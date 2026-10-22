import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

// The React library page: an APP build, a sibling of vite.reports.config.ts and
// vite.settings.config.ts, for the same reasons (it owns an index.html; the library build in
// vite.config.ts takes one entry). Straight into a directory axum's ServeDir already serves,
// committed, so a fresh clone plus `cargo run` serves /library/ with no node installed. CI
// rebuilds it and fails on any change.
export default defineConfig({
  root: resolve(import.meta.dirname, "src-js/library"),
  base: "/library/",
  plugins: [react()],
  // Nothing to copy: tokens.css and common.js are served from static/ by axum.
  publicDir: false,
  build: {
    outDir: resolve(import.meta.dirname, "static/library"),
    // Safe because the directory is this build's alone, never static/ itself.
    emptyOutDir: true,
    sourcemap: false,
    target: "es2022",
    rollupOptions: {
      output: {
        // Stable names, so a rebuild diffs as the same files rather than as a delete plus an add.
        entryFileNames: "library.js",
        chunkFileNames: "[name].js",
        assetFileNames: "[name][extname]",
      },
    },
  },
});
