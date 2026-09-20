// What a hover over the plan reads out, per entity.
//
// **A table like the appearance grid, and for the same reason**: the question
// a reader brings here is "what do I want to see when I sweep the pointer over
// this floor", and seven rows answered side by side is what makes that one
// decision instead of seven.
//
// **Free text, not a picker, and the empty state says why.** The server knows
// the ROOM property vocabulary — that is the datalist the room label and the
// comparison fields use — and knows nothing about a door's or a ceiling's,
// because those live in the element reads rather than in settings. Offering
// the room list against every row would suggest names that cannot work; so
// every row is typed, and a name nothing carries costs the reader the feature
// rather than the tooltip.

import { Block, Field, TextInput } from "../fields.js";
import type { HoverProperties, Settings } from "../types.js";

const EMPTY: HoverProperties = {};

/** The entities, in the order the pick stack returns them — so this table and
 *  the viewer's own menus read in one order. */
const ENTITIES: readonly { key: keyof HoverProperties; label: string; placeholder: string }[] = [
  { key: "rooms", label: "Rooms", placeholder: "Department" },
  { key: "doors", label: "Doors", placeholder: "Fire Rating" },
  { key: "windows", label: "Windows", placeholder: "Mark" },
  { key: "ffe", label: "FF&E", placeholder: "ItemCode" },
  { key: "spaces", label: "Spaces", placeholder: "Actual Supply Airflow" },
  { key: "ceilings", label: "Ceilings", placeholder: "Height Offset From Level" },
  { key: "floors", label: "Floors", placeholder: "Type Mark" },
];

export function HoverSection({
  settings,
  edit,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
}) {
  const hover: HoverProperties = settings.hover ?? EMPTY;
  const set = (key: keyof HoverProperties, value: string) =>
    edit((s) => ({ ...s, hover: { ...(s.hover ?? EMPTY), [key]: value.trim() || null } }));

  return (
    <Block
      title="Hover"
      hint={
        <>
          Which property the viewer shows in the tooltip when the pointer rests on each kind of element. Left blank, it
          shows the element&apos;s name and then its id, which is what it has always done &mdash; and a name no element
          carries falls back the same way, so a typo costs the tooltip its <em>content</em>, never the tooltip. The
          instance parameter is read first and the type parameter second, so a property that is blank on the instance
          still answers from the type.
        </>
      }
    >
      <div className="hover-grid">
        {ENTITIES.map((e) => (
          <Field key={e.key} label={e.label}>
            <TextInput
              value={hover[e.key] ?? ""}
              size={24}
              placeholder={e.placeholder}
              title={`The property a hover over ${e.label.toLowerCase()} reads out. Blank follows the name, then the id.`}
              onChange={(v) => set(e.key, v)}
            />
          </Field>
        ))}
      </div>
    </Block>
  );
}
