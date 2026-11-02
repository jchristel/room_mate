import { defineConfig } from "vitest/config";

// Separate from vite.config.ts on purpose: that file is a LIBRARY build (IIFE,
// one entry, output into static/vendor/) and none of it applies to running
// tests. Merging them would mean every test run carries a build config it
// ignores, and every build carries a test environment it never uses.
export default defineConfig({
  test: {
    // jsdom, because the SVG painter is DOM code and the thing worth pinning is
    // the SERIALIZED document -- `XMLSerializer`, attribute order, and text
    // escaping all have to behave as they do in a browser. A mocked DOM would
    // test the mock.
    environment: "jsdom",
    include: ["src-js/**/*.test.ts"],
  },
});
