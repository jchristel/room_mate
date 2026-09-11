import type { Settings } from "./types.js";

/** The top-level fields a user can *clear*, and therefore the ones that must go
 *  over the wire as an explicit `null`. See `saveBody`. */
const CLEARABLE = ["name", "comparison_key", "anchor_model"] as const;

/**
 * The PUT body for a save: the whole settings object as it was read, with the
 * clearable scalars trimmed and — the part that matters — sent as an explicit
 * `null` when blank.
 *
 * **Why null and not absent.** A PUT is merged over the stored file key by key
 * (`merge_over_stored`), so a key the body omits keeps its stored value. The
 * server writes a `None` by leaving the key out, so a page that echoed that
 * shape back could never clear one — and nor could one that dropped the key with
 * `delete` or set it to `undefined`, which `JSON.stringify` also leaves out. The
 * Rust+WASM page in the framework comparison hit exactly this, by echoing the
 * Rust type's own serialization.
 *
 * **Only TOP-LEVEL fields need this.** The merge is deliberately top-level only,
 * so a section this page edits — `[areas]`, `[doors]`, a milestone — is replaced
 * whole, and an absent key *inside* it simply deserializes to its serde default.
 * That is why the list is three scalars rather than every `Option` in the tree.
 */
export function saveBody(settings: Settings): Settings {
  const body: Settings = { ...settings };
  for (const key of CLEARABLE) {
    const value = body[key];
    const trimmed = typeof value === "string" ? value.trim() : "";
    body[key] = trimmed === "" ? null : trimmed;
  }
  return body;
}
