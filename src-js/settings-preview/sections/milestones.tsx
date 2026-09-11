// Milestones: a named date, and what each entity's data was at that date.
//
// **Six pin maps, where the JavaScript page shows one.** Rooms, doors, windows,
// FF&E, spaces and ceilings are pushed independently and their snapshot ids do
// not correspond, so each entity carries its own map — and the old page rebuilt
// a milestone from four fields on every save, which silently emptied the other
// five. That is not a display gap; it is data loss, and it is why this page
// holds the whole object and sends it back.

import { Block, ListControls, Row, RowHead, TextInput } from "../fields.js";
import type { Milestone, ModelSnapshots, Settings } from "../types.js";

/** Every per-model pin map on a milestone, in the order the entities shipped. */
const PIN_MAPS = [
  ["attachments", "rooms"],
  ["door_attachments", "doors"],
  ["window_attachments", "windows"],
  ["ffe_attachments", "FF&E"],
  ["space_attachments", "spaces"],
  ["ceiling_attachments", "ceilings"],
] as const;

type PinMap = (typeof PIN_MAPS)[number][0];

export function MilestonesSection({
  settings,
  edit,
  models,
  snapshotsBySource,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
  /** What the store actually holds, per model — `null` while loading. */
  models: ModelSnapshots[] | null;
  snapshotsBySource: Readonly<Record<string, string[] | null>>;
}) {
  const milestones = settings.milestones;
  const setMilestones = (next: Milestone[]) => edit((s) => ({ ...s, milestones: next }));
  const sourceNames = Object.keys(settings.sources.reference ?? {});

  return (
    <Block
      title="Milestones"
      hint="Named dates with data snapshots pinned to them. The viewer's milestone dropdown lists these; a pinned model is served from that snapshot instead of its latest, and an unpinned model doesn't appear in the milestone view. Each entity pins separately, because each is pushed separately."
    >
      <div className="rows">
        {milestones.map((milestone, index) => (
          <MilestoneRows
            key={`${index}:${milestone.name}`}
            milestone={milestone}
            index={index}
            milestones={milestones}
            models={models}
            sourceNames={sourceNames}
            snapshotsBySource={snapshotsBySource}
            onChange={(next) => setMilestones(milestones.map((m, i) => (i === index ? next : m)))}
            onList={setMilestones}
          />
        ))}
      </div>
      <button
        type="button"
        className="action"
        onClick={() =>
          setMilestones([
            ...milestones,
            {
              name: "",
              date: "",
              reference_snapshots: {},
              attachments: {},
              door_attachments: {},
              window_attachments: {},
              ffe_attachments: {},
              space_attachments: {},
              ceiling_attachments: {},
            },
          ])
        }
      >
        + milestone
      </button>
    </Block>
  );
}

function MilestoneRows({
  milestone,
  index,
  milestones,
  models,
  sourceNames,
  snapshotsBySource,
  onChange,
  onList,
}: {
  milestone: Milestone;
  index: number;
  milestones: readonly Milestone[];
  models: ModelSnapshots[] | null;
  sourceNames: readonly string[];
  snapshotsBySource: Readonly<Record<string, string[] | null>>;
  onChange: (next: Milestone) => void;
  onList: (next: Milestone[]) => void;
}) {
  const setPin = (map: PinMap, model: string, taken_at: string) => {
    const pins = { ...milestone[map] };
    if (taken_at === "") delete pins[model];
    else pins[model] = taken_at;
    onChange({ ...milestone, [map]: pins });
  };

  return (
    <>
      <Row>
        <RowHead>{index + 1}.</RowHead>
        <span>name</span>
        <TextInput
          value={milestone.name}
          size={18}
          placeholder="Design Freeze"
          onChange={(name) => onChange({ ...milestone, name })}
        />
        <span>date</span>
        <input
          type="date"
          value={/^\d{4}-\d{2}-\d{2}$/.test(milestone.date) ? milestone.date : ""}
          onChange={(ev) => onChange({ ...milestone, date: ev.target.value })}
        />
        <ListControls list={milestones} index={index} onChange={onList} />
      </Row>

      {sourceNames.map((source) => {
        const snapshots = snapshotsBySource[source];
        if (!snapshots || snapshots.length === 0) return null;
        const pinned = milestone.reference_snapshots[source] ?? "";
        return (
          <Row key={`ref:${source}`}>
            <RowHead>⟐</RowHead>
            <span>{source}</span>
            <select
              value={pinned}
              onChange={(ev) => {
                const pins = { ...milestone.reference_snapshots };
                if (ev.target.value === "") delete pins[source];
                else pins[source] = ev.target.value;
                onChange({ ...milestone, reference_snapshots: pins });
              }}
            >
              <option value="">— current {source} —</option>
              {snapshots.map((taken_at) => (
                <option key={taken_at} value={taken_at}>
                  {taken_at}
                </option>
              ))}
              {/* A pin whose snapshot is no longer stored stays visible and
                  removable rather than vanishing — removing it is the user's call. */}
              {pinned !== "" && !snapshots.includes(pinned) && (
                <option value={pinned}>{pinned} (missing)</option>
              )}
            </select>
          </Row>
        );
      })}

      {models === null && (
        <Row>
          <RowHead>loading stored snapshots…</RowHead>
        </Row>
      )}
      {models !== null && models.length === 0 && (
        <Row>
          <RowHead>no snapshots stored for this project yet — push data first, then pin it here</RowHead>
        </Row>
      )}
      {models?.map((model) => (
        <Row key={model.id}>
          <RowHead>↳</RowHead>
          <span>
            {model.name} ({model.id})
          </span>
          {PIN_MAPS.map(([map, label]) => {
            const pinned = milestone[map][model.id] ?? "";
            return (
              <label className="field" key={map} title={`${label} snapshot pinned to this milestone`}>
                {label}
                <select value={pinned} onChange={(ev) => setPin(map, model.id, ev.target.value)}>
                  <option value="">—</option>
                  {model.snapshots.map((taken_at) => (
                    <option key={taken_at} value={taken_at}>
                      {taken_at}
                    </option>
                  ))}
                  {pinned !== "" && !model.snapshots.includes(pinned) && (
                    <option value={pinned}>{pinned} (missing)</option>
                  )}
                </select>
              </label>
            );
          })}
        </Row>
      ))}

      {/* Pins for models the server has no data for any more: shown, not hidden
          — a dangling pin should be visible and deliberately removable. */}
      {PIN_MAPS.flatMap(([map, label]) =>
        Object.entries(milestone[map])
          .filter(([model]) => !(models ?? []).some((m) => m.id === model))
          .map(([model, taken_at]) => (
            <Row key={`${map}:${model}`}>
              <RowHead>↳</RowHead>
              <span>
                {label}: {model} → {taken_at} (no stored data for this model)
              </span>
              <button type="button" className="action icon" title="remove pin" onClick={() => setPin(map, model, "")}>
                ✕
              </button>
            </Row>
          )),
      )}
    </>
  );
}
