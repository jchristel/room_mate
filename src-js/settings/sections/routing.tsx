// The routing policy: where a route's size checks read their data from.
//
// A route can be asked whether an object of a given width and height can make the
// trip, and the model states little of what that needs: no door carries a width or a
// height this server can rely on, and a room's clear height is whatever parameter the
// project's template calls it. So the sources are named here, per project, and every
// figure they yield is an ESTIMATE the route answer says so about.

import { useState } from "react";

import { Block, Check, Field, NumberInput, OptionalText, Row, RowHead } from "../fields.js";
import { StringList } from "../stringList.js";
import type { RoutingPolicy, Settings } from "../types.js";

/** The Rust defaults, shown as placeholders so an empty box reads as "the default" and
 *  not as "nothing". */
const DEFAULT_ROOM_HEIGHT_PROPERTY = "Ceiling Height";
const DEFAULT_FRAME_ALLOWANCE_MM = 150;
const DEFAULT_DOOR_HEIGHT_PROPERTIES = "Height, Door Height";

const ROUTING_DEFAULT: RoutingPolicy = { door_height_from_type_name: true, room_height_from_ceilings: true };

export function RoutingSection({
  settings,
  edit,
  propertyListId,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
  propertyListId: string;
}) {
  const routing: RoutingPolicy = settings.routing ?? ROUTING_DEFAULT;
  // Held as text while it is being typed -- see `NumberInput`. Seeded once per mounted
  // editor, which is per project: the editor is keyed by project id.
  const [allowance, setAllowance] = useState(routing.door_frame_allowance_mm == null ? "" : String(routing.door_frame_allowance_mm));
  const editRouting = (change: (r: RoutingPolicy) => RoutingPolicy) => edit((s) => ({ ...s, routing: change(routing) }));

  return (
    <Block
      title="Routing"
      hint={
        <>
          Where the route tool&apos;s <strong>width and height checks</strong> read their data from. Everything they
          yield is an <strong>estimate</strong> and the route answer says so: a door that cannot be read is let through
          and counted as unchecked, never guessed. A request can still override the room property and the frame
          allowance.
        </>
      }
    >
      <Row>
        <Field label="room height property">
          <OptionalText
            value={routing.room_height_property}
            size={20}
            placeholder={DEFAULT_ROOM_HEIGHT_PROPERTY}
            list={propertyListId}
            title="The room property holding a room's clear height, in millimetres. Matched ignoring case and runs of spaces, so Revit's 'Ceiling  Height' (two spaces) matches 'Ceiling Height'. A value under 100 is taken to be another unit and is not used."
            onChange={(value) => editRouting((r) => ({ ...r, room_height_property: value }))}
          />
        </Field>
        <Field label="door frame allowance (mm)">
          <NumberInput
            text={allowance}
            size={6}
            placeholder={String(DEFAULT_FRAME_ALLOWANCE_MM)}
            title="What a door's frame and stops take off the width its footprint measures, to give the clear opening. The footprint is overall width, frame included: a 970 mm leaf measures about 1,055 mm. Between 0 and 2000."
            onChange={(text, parsed) => {
              setAllowance(text);
              editRouting((r) => ({ ...r, door_frame_allowance_mm: parsed }));
            }}
          />
        </Field>
      </Row>
      <Row>
        <Check
          checked={routing.room_height_from_ceilings}
          title="Read a room's height from the ceilings over it, and from the room height property only for a room no ceiling covers. The lowest ceiling covering at least a quarter of the room wins; a ceiling at or below its level (a roof) is ignored. Reading ceilings is an extra pass, made only when a height is asked for."
          onChange={(checked) => editRouting((r) => ({ ...r, room_height_from_ceilings: checked }))}
        >
          read a room&apos;s height from its ceilings first
        </Check>
      </Row>
      <Row>
        <Check
          checked={routing.door_height_from_type_name}
          title="Read the size in a door's type name when none of the door height properties is present: '970 x 2040 TD01', 'DWWH-003 820W x 2100H'. The first 'A x B' wins. Turn off for a project whose names say something else that looks like a size."
          onChange={(checked) => editRouting((r) => ({ ...r, door_height_from_type_name: checked }))}
        >
          read a door&apos;s height from its type name
        </Check>
      </Row>
      <Row>
        <RowHead>door height properties — tried in order, instance before type, ahead of the type name</RowHead>
      </Row>
      <StringList
        values={routing.door_height_properties ?? []}
        list={propertyListId}
        placeholder={DEFAULT_DOOR_HEIGHT_PROPERTIES}
        addLabel="+ property"
        onChange={(next) => editRouting((r) => ({ ...r, door_height_properties: next }))}
      />
    </Block>
  );
}
