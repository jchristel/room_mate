// The room label: which fields the viewer prints on each room, in order.

import { Block } from "../fields.js";
import { StringList } from "../stringList.js";
import type { Settings } from "../types.js";

export function RoomLabelSection({
  settings,
  edit,
  propertyListId,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
  propertyListId: string;
}) {
  return (
    <Block
      title="Room label"
      hint={
        <>
          Ordered fields shown on each room in the viewer. <code>$name</code> and <code>$id</code> are the room&apos;s
          own name/id; <code>source.Column</code> reads a joined reference source; anything else is a property name. An
          empty list prints nothing — the viewer does not fall back, because a label the server withheld is not a label
          to invent.
        </>
      }
    >
      <StringList
        values={settings.room_label}
        list={propertyListId}
        placeholder="Area"
        onChange={(next) => edit((s) => ({ ...s, room_label: next }))}
      />
    </Block>
  );
}
