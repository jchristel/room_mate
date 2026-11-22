import { describe, expect, it } from "vitest";

import { matchesAll } from "./match.js";

describe("matchesAll", () => {
  it("needs every word, in any case, anywhere in the text", () => {
    expect(matchesAll("Bedroom RLS 01.02", "rls bed")).toBe(true);
    expect(matchesAll("Bedroom RLS 01.02", "RLS kitchen")).toBe(false);
  });

  it("matches everything when nothing is typed", () => {
    expect(matchesAll("anything", "   ")).toBe(true);
    expect(matchesAll("", "")).toBe(true);
  });
});
