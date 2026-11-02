// What the tag on an FF&E item, space, ceiling or floor SAYS: the settings' fields
// resolved against one element. Pure, with no store.
//
// The room label is resolved by the server; an element's tag is resolved here because
// an element response already carries its properties and the page already does this
// lookup for the hover. Where the tag goes and how big it is lives in the renderer
// (`renderer/elementTags.ts`), shared by the screen and the SVG export.

import type { TagEntity, TagRequest, TagsRequest } from "../renderer/elementTags.js";
import { tieredValue } from "./properties.js";

/** The settings' `[tags]`, as the page reads them. */
export interface TagSetting {
  fields: string[];
  primary?: string | null;
  secondary?: string | null;
}
export type TagSettings = Record<TagEntity, TagSetting>;

/** What the server defaults to, for a project whose settings could not be read. */
export const DEFAULT_TAG_SETTINGS: TagSettings = {
  ffe: { fields: ["$type_name"] },
  spaces: { fields: ["$name"] },
  ceilings: { fields: ["$type_name"] },
  floors: { fields: ["$type_name"] },
};

export const TAG_ENTITIES: readonly TagEntity[] = ["ffe", "spaces", "ceilings", "floors"];

type Bag = Record<string, { value?: string } | undefined> | undefined;

interface Taggable {
  id: string;
  name?: string | null;
  type_name?: string | null;
  properties?: Bag;
}

/** The lines one element's tag says, one per field that has something. */
export function tagLines(fields: readonly string[], element: Taggable, typeBag: Bag): string[] {
  const lines: string[] = [];
  for (const field of fields) {
    let value: string | null;
    switch (field) {
      case "$id":
        value = element.id;
        break;
      case "$name":
        value = element.name ?? null;
        break;
      case "$type_name":
        value = element.type_name ?? null;
        break;
      default:
        value = tieredValue(field, element.properties, typeBag);
    }
    if (value && value.trim() !== "") lines.push(value.trim());
  }
  return lines;
}

/**
 * The renderer's request for the entities a zone has tags on. `typeBagOf` resolves an
 * element's type tier against the payload it came from (a `type_properties_ref`
 * indexes its own response only).
 */
export function tagsRequest(
  settings: TagSettings,
  on: Readonly<Record<TagEntity, boolean>>,
  typeBagOf: (entity: TagEntity, element: unknown) => Bag,
): TagsRequest {
  const out: Record<string, TagRequest<never>> = {};
  for (const entity of TAG_ENTITIES) {
    if (!on[entity]) continue;
    const s = settings[entity];
    out[entity] = {
      text: (el: never) => tagLines(s.fields, el as Taggable, typeBagOf(entity, el)),
      primary: s.primary ?? null,
      secondary: s.secondary ?? null,
    };
  }
  return out as TagsRequest;
}
