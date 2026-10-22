import { describe, expect, it } from "vitest";

import { attentionReasons, blueprintNeedsAttention } from "./common.js";

describe("blueprints that need attention", () => {
  it("a blueprint with sheets and nothing pending does not", () => {
    expect(blueprintNeedsAttention({ sheets: 2, pending: 0 })).toBe(false);
    expect(attentionReasons({ sheets: 2, pending: 0 })).toEqual([]);
  });

  it("one with no sheets does, and says why", () => {
    expect(blueprintNeedsAttention({ sheets: 0, pending: 0 })).toBe(true);
    expect(attentionReasons({ sheets: 0, pending: 0 })).toEqual(["no room layout sheets"]);
  });

  it("a pending revision does, and both reasons are given when both hold", () => {
    expect(blueprintNeedsAttention({ sheets: 1, pending: 1 })).toBe(true);
    expect(attentionReasons({ sheets: 0, pending: 2 })).toEqual(["no room layout sheets", "2 pending revisions to accept or reject"]);
  });
});
