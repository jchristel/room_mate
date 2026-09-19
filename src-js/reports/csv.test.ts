import { describe, expect, it } from "vitest";

import { buildCsv, csvEscape } from "./csv.js";
import type { CheckRow } from "./checkRows.js";

describe("csvEscape", () => {
  it("leaves an ordinary value alone", () => {
    expect(csvEscape("G01 ENTRY")).toBe("G01 ENTRY");
  });

  // The three characters that would otherwise split a row or a field, which is
  // the whole reason this function exists rather than a bare join.
  it("quotes a value carrying a comma, a quote or a newline", () => {
    expect(csvEscape("Cardiology, North")).toBe('"Cardiology, North"');
    expect(csvEscape('room="A"')).toBe('"room=""A"""');
    expect(csvEscape("line\r\nbreak")).toBe('"line\r\nbreak"');
  });
});

describe("buildCsv", () => {
  const rows: CheckRow[] = [
    { severity: "finding", cells: { Source: "drofus", Room: "G01 ENTRY", Finding: "missing link value" } },
    { severity: "expected", cells: { Source: "drofus", Room: "", Finding: "pending: rooms not pushed yet" } },
  ];

  it("writes the header, then one CRLF-joined line per row", () => {
    const csv = buildCsv(["Source", "Room", "Finding"], rows);
    expect(csv.split("\r\n")).toEqual([
      "Source,Room,Finding,severity",
      "drofus,G01 ENTRY,missing link value,finding",
      "drofus,,pending: rooms not pushed yet,expected",
    ]);
  });

  // A column a row never filled is an empty cell, not a missing one: a short
  // line would shift every later column of that row by one.
  it("fills a column the row has no value for", () => {
    const csv = buildCsv(["Source", "Link value", "Finding"], rows);
    expect(csv.split("\r\n")[1]).toBe("drofus,,missing link value,finding");
  });

  // Severity rides the sheet rather than filtering it: an expected row that
  // vanished from the export would read as "nothing to see", which is the
  // opposite of what it says.
  it("carries severity as a column", () => {
    const csv = buildCsv(["Source"], rows);
    expect(csv.split("\r\n")[2]).toBe("drofus,expected");
  });
});
