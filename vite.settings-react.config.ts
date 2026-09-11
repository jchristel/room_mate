import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";

// The React preview of the settings page — an APP build, which is why it is a
// second config rather than a second entry in vite.config.ts. That file is a
// library build (one IIFE the viewer calls), and Vite's IIFE output takes one
// entry; this one owns an index.html of its own.
//
// Same destination shape as the renderer bundle: straight
// into a directory axum's ServeDir already serves, committed, so a fresh clone
// plus `cargo run` serves /settings-react/ with no node installed. CI rebuilds it
// and fails on any change (.github/workflows/frontend.yml).
export default defineConfig({
  root: resolve(import.meta.dirname, "src-js/settings-react"),
  base: "/settings-react/",
  plugins: [react()],
  // Nothing to copy: tokens.css and common.js are served from static/ by axum
  // and referenced by absolute URL, which Vite leaves alone.
  publicDir: false,
  build: {
    outDir: resolve(import.meta.dirname, "static/settings-react"),
    // Safe because the directory is this build's alone — never static/ itself.
    emptyOutDir: true,
    sourcemap: false,
    target: "es2022",
    rollupOptions: {
      output: {
        // Stable names, so a rebuild diffs as the same files rather than as a
        // delete plus an add.
        entryFileNames: "settings.js",
        chunkFileNames: "[name].js",
        assetFileNames: "[name][extname]",
      },
    },
  },
});
