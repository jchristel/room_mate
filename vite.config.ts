import { defineConfig } from "vite";
import { resolve } from "node:path";

// Library-mode build, not an app build. The viewer is `static/index.html`,
// served by axum's ServeDir — it is NOT a Vite entry point and Vite must never
// rewrite it. What this produces is ONE self-contained IIFE that publishes a
// single global, which `index.html`'s existing classic <script> then calls.
//
// That shape is deliberate and is the reason the migration is reviewable: the
// page keeps its inline script, and the typed renderer arrives beside it rather
// than replacing it. See docs/Superseded/PLAN-webgl-renderer.md, "The migration boundary".
export default defineConfig({
  build: {
    lib: {
      entry: resolve(import.meta.dirname, "src-js/renderer/index.ts"),
      name: "PlanRenderer",
      // IIFE, not ESM: `index.html` loads this with a plain <script> alongside
      // common.js and graph.js. A module build would need `type="module"`,
      // which defers execution past the inline script that calls it.
      formats: ["iife"],
      fileName: () => "renderer.bundle.js",
    },
    // Straight into the directory ServeDir already serves, so a fresh clone
    // plus `cargo run` gets a working viewer with no node installed. The
    // artifact is committed; CI re-builds it and fails on any diff, because a
    // committed generated file with no gate drifts silently.
    outDir: resolve(import.meta.dirname, "static/vendor"),
    // static/ holds hand-written, tracked files. Emptying it would delete
    // index.html.
    emptyOutDir: false,
    // Bundle everything. There is no module loader on the page to resolve a
    // bare import at runtime.
    rollupOptions: { external: [] },
    // No sourcemap in the committed artifact. It is tempting -- the bundle is
    // what ships, so it is what gets debugged -- but the map covers the BUNDLE,
    // which inlines Pixi, and that runs to megabytes of generated text in git
    // forever. `npm run dev` serves the TypeScript directly with full maps, and
    // that is where renderer debugging belongs. Keeping this false also means
    // the CI freshness gate diffs exactly one file.
    sourcemap: false,
    target: "es2022",
  },
  server: {
    // `npm run dev` serves the TypeScript with HMR while axum keeps answering
    // the API on 8080, so the renderer can be worked on without a `cargo run`
    // restart per edit. Not how the app ships — production is the committed
    // bundle above.
    proxy: {
      "/rooms": "http://127.0.0.1:8080",
      "/projects": "http://127.0.0.1:8080",
      "/doors": "http://127.0.0.1:8080",
      "/settings": "http://127.0.0.1:8080",
    },
  },
});
