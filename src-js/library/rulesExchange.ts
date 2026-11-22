// The library's tag rules as a file, and a file back in through `POST /tagrules/import`: there is no other way to put a
// stored rule in, because the extractor's route takes tags read from Revit and works the rules out from them.
//
// **A file is room-agnostic**, like a sheets file. A rule bound to a room blueprint (`key.blueprint`, `key.instance`)
// is exported as the generic rule it would become if its blueprint went away: the references are cleared and its
// samples lose their boundary recipe, which was written against that blueprint's room (the server does the same on
// the way in, so a hand-made file cannot bring either back). What is left is what the rule IS: its key, the kind of
// tag and the samples it was learned from. The answer and counts are worked out again by the server.

import { safeName } from "../ui/file.js";
import type { RuleKey, Sample, TagRule } from "./types.js";

export const FILE_FORMAT = "roommate-tag-rules";

export interface ExportedRule {
  key: RuleKey;
  tag_kind: string;
  samples: Sample[];
}

export interface RulesFile {
  format: typeof FILE_FORMAT;
  version: 1;
  exported_from: string;
  exported_at: string;
  rules: ExportedRule[];
}

export interface ImportOutcome {
  rules_created: number;
  rules_updated: number;
  samples: number;
  empty: number;
  unused: number;
}

/** One rule as it is filed: generic, and without a boundary recipe a blueprint's room gave it. */
export function exportedRule(rule: TagRule): ExportedRule {
  const bound = rule.key.blueprint !== "" || rule.key.instance !== "";
  return {
    key: { ...rule.key, blueprint: "", instance: "" },
    tag_kind: rule.tag_kind,
    samples: bound ? rule.samples.map(({ boundary: _boundary, ...rest }) => rest) : rule.samples,
  };
}

export function toFile(rules: readonly TagRule[], project: string, now: Date = new Date()): RulesFile {
  return { format: FILE_FORMAT, version: 1, exported_from: project, exported_at: now.toISOString(), rules: rules.map(exportedRule) };
}

export const fileNameFor = (project: string, count: number, now: Date = new Date()): string =>
  `${safeName(project, "library")}.${count}-tag-rules.${now.toISOString().slice(0, 10)}.json`;

const isText = (v: unknown): v is string => typeof v === "string" && v.trim() !== "";

/** Parse a file's text, or say why it is not a tag rules export (the server's refusal would be less helpful). */
export function parseFile(text: string): { ok: true; file: RulesFile } | { ok: false; error: string } {
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return { ok: false, error: "That file is not JSON." };
  }
  const f = raw as Partial<RulesFile> | null;
  if (!f || typeof f !== "object" || f.format !== FILE_FORMAT) return { ok: false, error: "That file is not a tag rules export from the library." };
  if (f.version !== 1) return { ok: false, error: `That file is version ${String(f.version)}; this page reads version 1.` };
  if (!Array.isArray(f.rules) || f.rules.length === 0) return { ok: false, error: "That file holds no rules." };
  for (const [i, r] of f.rules.entries()) {
    const at = `Rule ${i + 1}`;
    if (!r?.key || !isText(r.key.tagged_category) || !isText(r.key.tag_category)) return { ok: false, error: `${at} has no tagged or tag category.` };
    if (!isText(r.tag_kind)) return { ok: false, error: `${at} has no tag kind.` };
    if (!Array.isArray(r.samples)) return { ok: false, error: `${at} has no samples.` };
  }
  return { ok: true, file: f as RulesFile };
}

/** "3 new, 1 updated (12 samples)", and what the person should know: empty rules left out, rules this project will not use. */
export function outcomeText(o: ImportOutcome): string {
  const parts = [`${o.rules_created} new, ${o.rules_updated} updated (${o.samples} sample${o.samples === 1 ? "" : "s"})`];
  if (o.empty > 0) parts.push(`${o.empty} had no samples and were left out`);
  if (o.unused > 0) parts.push(`${o.unused} read a type-code parameter other than this project's, so they stay unused until the settings name it`);
  return parts.join("; ");
}

/** The outcomes of every batch of one import, added up. */
export const sum = (all: readonly ImportOutcome[]): ImportOutcome =>
  all.reduce((a, o) => ({ rules_created: a.rules_created + o.rules_created, rules_updated: a.rules_updated + o.rules_updated, samples: a.samples + o.samples, empty: a.empty + o.empty, unused: a.unused + o.unused }), {
    rules_created: 0,
    rules_updated: 0,
    samples: 0,
    empty: 0,
    unused: 0,
  });
