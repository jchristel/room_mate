// Check rows as CSV, plus the download.
//
// **This is the viewer's `buildValidationCsv`, moved.** That function built the
// QA export in the browser from the same validation response this page reads,
// which meant two renderings of one answer: the moment either was fixed they
// would disagree. The viewer's copy is deleted with this landing.
//
// It is still client-side, and that is a staging decision rather than the end
// state: STRATEGY-REPORTS.md has CSV rendering server-side behind a `format`
// parameter so an MCP host and a download cannot differ. Until that exists,
// this file is the one renderer, which is already better than two.

import type { CheckRow } from "./checkRows.js";

/** RFC-4180 escaping: quote when the value carries a separator, quote or newline. */
export function csvEscape(value: string): string {
  const text = value ?? "";
  return /[",\r\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

/**
 * Header row then one line per row, CRLF-joined.
 *
 * `severity` is a real column rather than a filter: a reader sorting the sheet
 * wants the expected rows visible and marked, not silently dropped — the
 * "signal, not error" rule, carried into the export.
 */
export function buildCsv(columns: string[], rows: CheckRow[]): string {
  const header = [...columns, "severity"];
  const lines = rows.map((row) => [...columns.map((c) => row.cells[c] ?? ""), row.severity]);
  return [header, ...lines].map((line) => line.map(csvEscape).join(",")).join("\r\n");
}

/** Turn CSV text into a download. Named for what it does to the page, not the data. */
export function downloadCsv(filename: string, csvText: string): void {
  const blob = new Blob([csvText], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = Object.assign(document.createElement("a"), { href: url, download: filename });
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
