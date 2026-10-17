// What the tag written on each FF&E item, space, ceiling and floor says, and in
// which colours.
//
// **Free text, not a picker**, for the reason the hover table is: the server knows the
// ROOM property vocabulary and nothing of a ceiling's or an item's, so offering the room
// list against these rows would suggest names that cannot work. A name no element
// carries just prints nothing for that element.
//
// The colours default to the layer's own line colour, so a tag reads as part of the
// layer it labels; the swatch shows that colour while the setting is unset.

import { Block, Field, OptionalColour } from "../fields.js";
import { StringList } from "../stringList.js";
import type { ElementTags, Settings, TagSettings } from "../types.js";

const INK = "#222222";
const ACCENT = "#c8102e";

const ENTITIES: readonly { key: keyof ElementTags; label: string; fallback: string; placeholder: string }[] = [
  { key: "ffe", label: "FF&E", fallback: INK, placeholder: "ItemCode" },
  { key: "spaces", label: "Spaces", fallback: ACCENT, placeholder: "Actual Supply Airflow" },
  { key: "ceilings", label: "Ceilings", fallback: INK, placeholder: "Type Mark" },
  { key: "floors", label: "Floors", fallback: INK, placeholder: "Type Mark" },
];

export function TagsSection({
  settings,
  edit,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
}) {
  const editTag = (key: keyof ElementTags, change: (t: TagSettings) => TagSettings) =>
    edit((s) => ({ ...s, tags: { ...s.tags, [key]: change(s.tags[key]) } }));

  return (
    <Block
      title="Element tags"
      hint={
        <>
          The text the viewer writes on each FF&amp;E item, space, ceiling and floor when its <strong>tags</strong> are
          switched on in the Layers menu. <code>$id</code>, <code>$name</code> and <code>$type_name</code> are the
          element&apos;s own; anything else is a property name, read from the instance first and then the type. The text
          is sized to fit the element&apos;s own outline. An empty list prints nothing. Colours left on{" "}
          <strong>theme</strong> follow the layer&apos;s line colour; the second colour is for the lines stacked under the
          first.
        </>
      }
    >
      {ENTITIES.map((e) => {
        const tag = settings.tags[e.key];
        return (
          <Field key={e.key} label={e.label}>
            <StringList
              values={tag.fields}
              placeholder={e.placeholder}
              onChange={(fields) => editTag(e.key, (t) => ({ ...t, fields }))}
            />
            <div className="tag-colours">
              <OptionalColour
                value={tag.primary}
                fallback={e.fallback}
                title="The first line of the tag."
                onChange={(primary) => editTag(e.key, (t) => ({ ...t, primary }))}
              />
              <OptionalColour
                value={tag.secondary}
                fallback={e.fallback}
                title="The lines stacked under the first. Follows the first colour while unset."
                onChange={(secondary) => editTag(e.key, (t) => ({ ...t, secondary }))}
              />
            </div>
          </Field>
        );
      })}
    </Block>
  );
}
