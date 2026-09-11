import type { Settings } from "./types.js";

/**
 * The PUT body for a save: the whole settings object as it was read, with the
 * display name trimmed and — the part that matters — an explicit `null` when it
 * is blank.
 *
 * **Why null and not absent.** A PUT is merged over the stored file key by key
 * (`merge_over_stored`), so a key the body omits keeps its stored value. The
 * server writes a `None` name by leaving the key out, so a page that echoed that
 * shape back could never clear a name — and nor could one that dropped the key
 * with `delete` or set it to `undefined`, which `JSON.stringify` also leaves out.
 * The Rust+WASM page in the framework comparison hit exactly this, by echoing
 * the Rust type's own serialization. TypeScript does not force it either way,
 * so a test pins it instead.
 */
export function saveBody(settings: Settings): Settings {
  const trimmed = typeof settings.name === "string" ? settings.name.trim() : "";
  return { ...settings, name: trimmed === "" ? null : trimmed };
}
