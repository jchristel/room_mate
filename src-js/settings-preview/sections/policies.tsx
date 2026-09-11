// The per-entity policies: doors, windows, FF&E and spaces.
//
// **None of these has ever had a UI**, and they are where a project's answers
// live for questions the viewer asks constantly: which room an opening belongs
// to, whether the server may work that out from geometry, and what "the same
// door" means across milestones. Editing them meant hand-writing TOML, so in
// practice they stayed at their defaults — `[doors] room_resolution` was lost
// once by a save from the old page and put 2,084 doors back to homeless.
//
// Doors and windows share `OpeningPolicy`, and the editor is shared with them;
// FF&E and spaces have their own shapes for reasons their Rust docs give.

import { Block, Field, NumberInput, OptionalSelect, OptionalText, Row, RowHead, Select } from "../fields.js";
import { StringList } from "../stringList.js";
import type {
  CompareMode,
  FfePolicy,
  FieldType,
  NestedComponents,
  OpeningPolicy,
  RoomAttribution,
  RoomResolution,
  Settings,
  SpaceFieldConfig,
  SpacePolicy,
} from "../types.js";

/** Rust defaults, for a project whose section is absent. */
const OPENING_DEFAULT: OpeningPolicy = {
  room_attribution: "to_room_then_from_room",
  room_resolution: "off",
  comparison_properties: [],
};
const FFE_DEFAULT: FfePolicy = {
  nested_components: "exclude",
  room_resolution: "off",
  comparison_properties: [],
};

const ATTRIBUTION: readonly (readonly [RoomAttribution, string])[] = [
  ["to_room_then_from_room", "to room, else from room (default)"],
  ["to_room", "to room only"],
  ["from_room", "from room only"],
  ["both", "both sides (counts twice)"],
  ["none", "none — report every opening homeless"],
];

const RESOLUTION: readonly (readonly [RoomResolution, string])[] = [
  ["off", "off — authored references only"],
  ["same_model", "same model — fill an absent side from geometry"],
  ["project", "project — search every model in the project"],
];

const NESTED: readonly (readonly [NestedComponents, string])[] = [
  ["exclude", "exclude — components ride their parent"],
  ["include", "include — components are items too"],
];

const FIELD_TYPES: readonly (readonly [FieldType, string])[] = [
  ["string", "string"],
  ["numeric", "numeric"],
  ["date", "date"],
];

const QA_MODES: readonly (readonly [CompareMode, string])[] = [
  ["exact", "exact"],
  ["ignore", "ignore"],
];

export function PoliciesSection({
  settings,
  edit,
  propertyListId,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
  propertyListId: string;
}) {
  const doors = settings.doors ?? OPENING_DEFAULT;
  const windows = settings.windows ?? OPENING_DEFAULT;
  const ffe = settings.ffe ?? FFE_DEFAULT;
  const spaces: SpacePolicy = settings.spaces ?? {};

  return (
    <>
      <Block
        title="Doors"
        hint={
          <>
            Which room a door belongs to, and how far the server may go to work that out. Attribution is derived at
            read time and never stored, so changing it changes every answer and rewrites nothing.
          </>
        }
      >
        <OpeningPolicyEditor
          policy={doors}
          propertyListId={propertyListId}
          entity="door"
          onChange={(next) => edit((s) => ({ ...s, doors: next }))}
        />
      </Block>

      <Block
        title="Windows"
        hint={
          <>
            The same policy under a second key, because the vocabulary differs: a window family carries a different
            room-reference parameter from a door family. <strong>Geometry resolution matters more here</strong> — a
            facade model holds windows and no rooms, so with it off every window in such a project is homeless.
          </>
        }
      >
        <OpeningPolicyEditor
          policy={windows}
          propertyListId={propertyListId}
          entity="window"
          onChange={(next) => edit((s) => ({ ...s, windows: next }))}
        />
      </Block>

      <Block
        title="FF&amp;E"
        hint="An item sits IN one room, so there is no attribution choice to make; what it adds instead is whether nested components count as items of their own."
      >
        <Row>
          <Field label="comparison key">
            <OptionalText
              value={ffe.comparison_key}
              size={16}
              placeholder="(none)"
              title='The item property identifying "the same item" across milestones. "$id" is usually right.'
              onChange={(value) => edit((s) => ({ ...s, ffe: { ...ffe, comparison_key: value } }))}
            />
          </Field>
          <Field label="nested components">
            <Select
              value={ffe.nested_components}
              options={NESTED}
              onChange={(value) => edit((s) => ({ ...s, ffe: { ...ffe, nested_components: value } }))}
            />
          </Field>
        </Row>
        <Row>
          <Field label="room resolution">
            <Select
              value={ffe.room_resolution}
              options={RESOLUTION}
              onChange={(value) => edit((s) => ({ ...s, ffe: { ...ffe, room_resolution: value } }))}
            />
          </Field>
          <Field label="room reference property">
            <OptionalText
              value={ffe.room_reference_property}
              size={20}
              placeholder="(check off)"
              onChange={(value) => edit((s) => ({ ...s, ffe: { ...ffe, room_reference_property: value } }))}
            />
          </Field>
        </Row>
        <Row>
          <RowHead>compared properties</RowHead>
        </Row>
        <StringList
          values={ffe.comparison_properties}
          list={propertyListId}
          placeholder="$room"
          onChange={(next) => edit((s) => ({ ...s, ffe: { ...ffe, comparison_properties: next } }))}
        />
      </Block>

      <Block
        title="Spaces"
        hint={
          <>
            How an MEP space is matched to a room, and which properties are compared once it is. This is the one
            entity comparing two <em>different</em> vocabularies — a space&apos;s and a room&apos;s — which is why the
            compared properties are pairs rather than names.
          </>
        }
      >
        <Row>
          <Field label="comparison key">
            <OptionalText
              value={spaces.comparison_key}
              size={16}
              placeholder="(none)"
              onChange={(value) => edit((s) => ({ ...s, spaces: { ...spaces, comparison_key: value } }))}
            />
          </Field>
          <Field label="room key">
            <OptionalText
              value={spaces.room_key}
              size={16}
              placeholder="(none)"
              title="The ROOM property a space's key is matched against."
              list={propertyListId}
              onChange={(value) => edit((s) => ({ ...s, spaces: { ...spaces, room_key: value } }))}
            />
          </Field>
        </Row>
        <Row>
          <RowHead>room models — which models supply the rooms spaces are matched to</RowHead>
        </Row>
        <StringList
          values={spaces.room_models ?? []}
          placeholder="ARCH-01"
          addLabel="+ model"
          onChange={(next) => edit((s) => ({ ...s, spaces: { ...spaces, room_models: next } }))}
        />
        <div className="rows">
          {(spaces.compared_properties ?? []).map((pair, index) => (
            <ComparedPropertyRow
              key={`${index}:${pair.space}`}
              pair={pair}
              index={index}
              propertyListId={propertyListId}
              onChange={(next) =>
                edit((s) => ({
                  ...s,
                  spaces: {
                    ...spaces,
                    compared_properties: (spaces.compared_properties ?? []).map((p, i) => (i === index ? next : p)),
                  },
                }))
              }
              onRemove={() =>
                edit((s) => ({
                  ...s,
                  spaces: {
                    ...spaces,
                    compared_properties: (spaces.compared_properties ?? []).filter((_, i) => i !== index),
                  },
                }))
              }
            />
          ))}
        </div>
        <button
          type="button"
          className="action"
          onClick={() =>
            edit((s) => ({
              ...s,
              spaces: {
                ...spaces,
                compared_properties: [
                  ...(spaces.compared_properties ?? []),
                  { space: "", type: "string" } satisfies SpaceFieldConfig,
                ],
              },
            }))
          }
        >
          + compared property
        </button>
      </Block>
    </>
  );
}

function OpeningPolicyEditor({
  policy,
  entity,
  propertyListId,
  onChange,
}: {
  policy: OpeningPolicy;
  entity: "door" | "window";
  propertyListId: string;
  onChange: (next: OpeningPolicy) => void;
}) {
  return (
    <>
      <Row>
        <Field label="comparison key">
          <OptionalText
            value={policy.comparison_key}
            size={16}
            placeholder="(none)"
            title={`The ${entity} property identifying "the same ${entity}" across milestones. "$id" is usually right here — unlike rooms, an ElementId identifies the same physical leaf across pushes.`}
            onChange={(value) => onChange({ ...policy, comparison_key: value })}
          />
        </Field>
        <Field label="room attribution">
          <Select
            value={policy.room_attribution}
            options={ATTRIBUTION}
            title="Which room this opening belongs to. Derived at read time, never stored."
            onChange={(value) => onChange({ ...policy, room_attribution: value })}
          />
        </Field>
      </Row>
      <Row>
        <Field label="room resolution">
          <Select
            value={policy.room_resolution}
            options={RESOLUTION}
            title="How far the server may go to work out the rooms from geometry. An authored reference always wins; geometry only fills an absent side."
            onChange={(value) => onChange({ ...policy, room_resolution: value })}
          />
        </Field>
        <Field label="room reference property">
          <OptionalText
            value={policy.room_reference_property}
            size={20}
            placeholder="(check off)"
            title="The property carrying an AUTHORED room reference, reconciled against the attributed room. Absent disables the check — which the QA report then says, rather than reporting clean."
            onChange={(value) => onChange({ ...policy, room_reference_property: value })}
          />
        </Field>
      </Row>
      <Row>
        <RowHead>compared properties</RowHead>
      </Row>
      <StringList
        values={policy.comparison_properties}
        list={propertyListId}
        placeholder="$to_room"
        onChange={(next) => onChange({ ...policy, comparison_properties: next })}
      />
    </>
  );
}

function ComparedPropertyRow({
  pair,
  index,
  propertyListId,
  onChange,
  onRemove,
}: {
  pair: SpaceFieldConfig;
  index: number;
  propertyListId: string;
  onChange: (next: SpaceFieldConfig) => void;
  onRemove: () => void;
}) {
  return (
    <Row>
      <RowHead>{index + 1}.</RowHead>
      <span>space</span>
      <OptionalText value={pair.space} size={14} onChange={(value) => onChange({ ...pair, space: value ?? "" })} />
      <span>room</span>
      <OptionalText
        value={pair.room}
        size={14}
        list={propertyListId}
        placeholder="(same name)"
        onChange={(value) => onChange({ ...pair, room: value })}
      />
      <span>type</span>
      <Select value={pair.type} options={FIELD_TYPES} onChange={(value) => onChange({ ...pair, type: value })} />
      <span>qa</span>
      <OptionalSelect
        value={pair.qa}
        options={QA_MODES}
        absentLabel="default"
        onChange={(value) => onChange({ ...pair, qa: value })}
      />
      <span>tol %</span>
      <NumberInput
        text={pair.tolerance_pct === null || pair.tolerance_pct === undefined ? "" : String(pair.tolerance_pct)}
        size={5}
        onChange={(_text, parsed) => onChange({ ...pair, tolerance_pct: parsed })}
      />
      <span>tol min</span>
      <NumberInput
        text={pair.tolerance_min === null || pair.tolerance_min === undefined ? "" : String(pair.tolerance_min)}
        size={5}
        onChange={(_text, parsed) => onChange({ ...pair, tolerance_min: parsed })}
      />
      <button type="button" className="action icon" title="remove" onClick={onRemove}>
        ✕
      </button>
    </Row>
  );
}
