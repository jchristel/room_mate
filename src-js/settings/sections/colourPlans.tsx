// Colour plans: the named colouring configs the viewer's picker offers.
//
// The server stores these verbatim and computes no colours — all colour maths is
// client-side. Every mode the contract defines is editable here; the JavaScript
// page could only edit property-compare plans and showed the other two as
// read-only "edit this in the TOML" rows.
//
// The generated `ColourMode` and `Colouring` are discriminated unions, so the
// editor branches on `mode.kind` / `colouring.style` and TypeScript checks that
// every branch builds a complete variant. That is the whole reason this section
// needs no "editing model" of its own: the JS page flattened all three modes
// into one object with fields for every mode, and rebuilt the wire shape on
// save.

import { Block, Field, ListControls, NumberInput, OptionalText, Row, RowHead, Select, TextInput } from "../fields.js";
import { StringList } from "../stringList.js";
import type { Band, Colouring, ColourMode, ColourPlan, CompareOp, Settings } from "../types.js";

const MODES = [
  ["propertycompare", "property compare"],
  ["hierarchy", "hierarchy"],
  ["daterange", "date range"],
] as const;

const STYLES = [
  ["match", "match (2 colours)"],
  ["diverging", "diverging ramp"],
  ["bands", "bands"],
] as const;

const OPS: readonly (readonly [CompareOp, string])[] = [
  ["diff", "A − B (diff)"],
  ["ratio", "A ÷ B (ratio)"],
];

const DIVERGING_SCHEMES = [
  ["RdBu", "RdBu"],
  ["RdYlGn", "RdYlGn"],
] as const;

const QUALITATIVE_SCHEMES = [
  ["Set2", "Set2"],
  ["Paired", "Paired"],
] as const;

const DEFAULT_BAND_COLOUR = "#b4541f";

/** A band bound as text: blank means an open end, and a half-typed "-" must read
 *  as no bound rather than as a lie that corrects itself on the next keystroke. */
const bandText = (value: number | null) => (value === null ? "" : String(value));

/** Whole numbers get an inclusive reading ("covers 0–5"), which is the one that
 *  prevents the mistake: the trap is authoring [0, 5) and expecting 5 in it. */
function bandCoverage(band: Band): string {
  const { lo, hi } = band;
  if (lo === null && hi === null) return "covers every value";
  if (lo !== null && hi !== null && lo >= hi) return "covers nothing — lo must be below hi";
  const whole = (n: number | null) => n === null || Number.isInteger(n);
  if (!whole(lo) || !whole(hi)) {
    if (lo === null) return `covers any value < ${hi}`;
    if (hi === null) return `covers any value ≥ ${lo}`;
    return `covers ${lo} ≤ value < ${hi}`;
  }
  if (lo === null) return `covers everything up to ${(hi ?? 0) - 1}`;
  if (hi === null) return `covers ${lo} and up`;
  return hi - 1 === lo ? `covers ${lo} only` : `covers ${lo}–${hi - 1}`;
}

/** Values that fall between two bands. A gap is LEGAL — the server only enforces
 *  sorted and non-overlapping — so this states the consequence rather than
 *  calling it an error: whatever lands there paints with the no-data colour. */
function bandGapNote(bands: readonly Band[]): string {
  const uncovered: string[] = [];
  for (let i = 0; i + 1 < bands.length; i += 1) {
    const hi = bands[i]?.hi ?? null;
    const lo = bands[i + 1]?.lo ?? null;
    if (hi === null || lo === null || hi >= lo) continue;
    uncovered.push(Number.isInteger(hi) && Number.isInteger(lo) ? (lo - 1 === hi ? `${hi}` : `${hi}–${lo - 1}`) : `${hi} up to (not including) ${lo}`);
  }
  return uncovered.length > 0 ? `⚠ not covered by any band, painted as no-data: ${uncovered.join(", ")}` : "";
}

export function ColourPlansSection({
  settings,
  edit,
  propertyListId,
  dateFormatListId,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
  propertyListId: string;
  dateFormatListId: string;
}) {
  const plans = settings.colour_plans;
  const setPlans = (next: ColourPlan[]) => edit((s) => ({ ...s, colour_plans: next }));
  const tierNames = settings.hierarchy.map((t) => t.name.trim()).filter((n) => n !== "");

  const setPlan = (index: number, next: ColourPlan) => setPlans(plans.map((p, i) => (i === index ? next : p)));

  return (
    <Block
      title="Colour plans"
      hint="Named room-colouring configs the viewer's colour picker offers. At most one may be active (the picker's default; “No colour” always overrides). All colouring is computed in the browser — the server just stores these."
    >
      <div className="rows">
        {plans.map((plan, index) => (
          <div key={`${index}:${plan.name}`}>
            <Row>
              <RowHead>{index + 1}.</RowHead>
              <label className="field" title="the viewer picker's default plan">
                <input
                  type="radio"
                  name="colourActive"
                  checked={plan.active}
                  // One active plan, enforced here as well as server-side: making
                  // one active clears the rest.
                  onChange={() => setPlans(plans.map((p, i) => ({ ...p, active: i === index })))}
                />
                active
              </label>
              <span>name</span>
              <TextInput value={plan.name} size={18} placeholder="Area check" onChange={(name) => setPlan(index, { ...plan, name })} />
              <span>mode</span>
              <Select
                value={plan.mode.kind}
                options={MODES}
                title="what this plan colours by"
                onChange={(kind) => setPlan(index, { ...plan, mode: emptyMode(kind) })}
              />
              <ListControls list={plans} index={index} onChange={setPlans} />
            </Row>
            <ModeEditor
              mode={plan.mode}
              tierNames={tierNames}
              propertyListId={propertyListId}
              dateFormatListId={dateFormatListId}
              onChange={(mode) => setPlan(index, { ...plan, mode })}
            />
          </div>
        ))}
      </div>
      <button
        type="button"
        className="action"
        onClick={() => setPlans([...plans, { name: "", active: false, mode: emptyMode("propertycompare") }])}
      >
        + colour plan
      </button>
    </Block>
  );
}

/** A fresh mode of the chosen kind. Switching mode discards the previous one's
 *  fields deliberately: they mean nothing in the new mode, and carrying them
 *  hidden is how the JS page's flat editing object could emit a plan nobody had
 *  configured. */
function emptyMode(kind: ColourMode["kind"]): ColourMode {
  if (kind === "hierarchy") return { kind, tiers: [], scheme: "Set2" };
  if (kind === "daterange") return { kind, property: "", near_date: "", scheme: "RdYlGn", format: null };
  return {
    kind: "propertycompare",
    property_a: "",
    property_b: "",
    op: "diff",
    colouring: { style: "match", tolerance: 0 },
  };
}

function ModeEditor({
  mode,
  tierNames,
  propertyListId,
  dateFormatListId,
  onChange,
}: {
  mode: ColourMode;
  tierNames: readonly string[];
  propertyListId: string;
  dateFormatListId: string;
  onChange: (next: ColourMode) => void;
}) {
  if (mode.kind === "hierarchy") {
    return (
      <>
        {tierNames.length === 0 && (
          <Row>
            <RowHead>↳ no hierarchy tiers defined — add tiers in the Hierarchy section first</RowHead>
          </Row>
        )}
        {tierNames.map((name) => (
          <Row key={name}>
            <RowHead>↳</RowHead>
            <label className="field">
              <input
                type="checkbox"
                checked={mode.tiers.includes(name)}
                onChange={(ev) =>
                  onChange({
                    ...mode,
                    tiers: ev.target.checked ? [...mode.tiers, name] : mode.tiers.filter((t) => t !== name),
                  })
                }
              />
              {` ${name}`}
            </label>
          </Row>
        ))}
        <Row>
          <RowHead>
            parent (hue) → child (tint): {mode.tiers.join(" → ") || "(none checked — pick at least the parent tier)"}
          </RowHead>
        </Row>
        <Row>
          <RowHead>↳</RowHead>
          <Field label="hue scheme">
            <Select
              value={mode.scheme as "Set2" | "Paired"}
              options={QUALITATIVE_SCHEMES}
              title="qualitative palette for the parent hues"
              onChange={(scheme) => onChange({ ...mode, scheme })}
            />
          </Field>
        </Row>
      </>
    );
  }

  if (mode.kind === "daterange") {
    return (
      <>
        <Row>
          <RowHead>↳</RowHead>
          <Field label="date property">
            <TextInput
              value={mode.property}
              size={16}
              list={propertyListId}
              placeholder="LastSync"
              onChange={(property) => onChange({ ...mode, property })}
            />
          </Field>
          <Field label="near date">
            <input
              type="date"
              value={/^\d{4}-\d{2}-\d{2}$/.test(mode.near_date) ? mode.near_date : ""}
              onChange={(ev) => onChange({ ...mode, near_date: ev.target.value })}
            />
          </Field>
        </Row>
        <Row>
          <RowHead>↳</RowHead>
          <Field label="scheme">
            <Select
              value={mode.scheme as "RdYlGn" | "RdBu"}
              options={[
                ["RdYlGn", "RdYlGn"],
                ["RdBu", "RdBu"],
              ]}
              title="near = green end, far = red end"
              onChange={(scheme) => onChange({ ...mode, scheme })}
            />
          </Field>
          <Field label="date format">
            <OptionalText
              value={mode.format}
              size={22}
              list={dateFormatListId}
              placeholder="(blank = ISO-8601)"
              onChange={(format) => onChange({ ...mode, format })}
            />
          </Field>
        </Row>
        <Row>
          <RowHead>
            format is the strftime pattern the dates are in (reuse a reference source&apos;s date field format); after
            near date → blue
          </RowHead>
        </Row>
      </>
    );
  }

  return (
    <>
      <Row>
        <RowHead>↳</RowHead>
        <Field label="A">
          <TextInput
            value={mode.property_a}
            size={16}
            list={propertyListId}
            placeholder="Area"
            onChange={(property_a) => onChange({ ...mode, property_a })}
          />
        </Field>
        <Select value={mode.op} options={OPS} title="how A and B combine" onChange={(op) => onChange({ ...mode, op })} />
        <Field label="B">
          <TextInput
            value={mode.property_b}
            size={16}
            list={propertyListId}
            placeholder="d_net_area"
            onChange={(property_b) => onChange({ ...mode, property_b })}
          />
        </Field>
      </Row>
      <ColouringEditor colouring={mode.colouring} onChange={(colouring) => onChange({ ...mode, colouring })} />
    </>
  );
}

function ColouringEditor({ colouring, onChange }: { colouring: Colouring; onChange: (next: Colouring) => void }) {
  const bands = colouring.style === "bands" ? colouring.bands : [];
  return (
    <>
      <Row>
        <RowHead>↳</RowHead>
        <Field label="colour by">
          <Select
            value={colouring.style}
            options={STYLES}
            title="how the number becomes a colour"
            onChange={(style) =>
              onChange(
                style === "match"
                  ? { style, tolerance: 0 }
                  : style === "diverging"
                    ? { style, scheme: "RdBu" }
                    : { style, bands: [] },
              )
            }
          />
        </Field>
        {colouring.style === "match" && (
          <Field label="tolerance">
            <NumberInput
              text={String(colouring.tolerance)}
              placeholder="0.5"
              onChange={(_text, parsed) => onChange({ ...colouring, tolerance: parsed ?? 0 })}
            />
          </Field>
        )}
        {colouring.style === "diverging" && (
          <Field label="scheme">
            <Select
              value={colouring.scheme as "RdBu" | "RdYlGn"}
              options={DIVERGING_SCHEMES}
              onChange={(scheme) => onChange({ ...colouring, scheme })}
            />
          </Field>
        )}
      </Row>

      {colouring.style === "bands" && (
        <>
          {bands.map((band, index) => (
            <Row key={index}>
              <RowHead>↳↳</RowHead>
              <span>[</span>
              <NumberInput
                text={bandText(band.lo)}
                size={6}
                placeholder="−∞"
                onChange={(_text, lo) => onChange({ style: "bands", bands: bands.map((b, i) => (i === index ? { ...b, lo } : b)) })}
              />
              <span>,</span>
              <NumberInput
                text={bandText(band.hi)}
                size={6}
                placeholder="+∞"
                onChange={(_text, hi) => onChange({ style: "bands", bands: bands.map((b, i) => (i === index ? { ...b, hi } : b)) })}
              />
              <span>)</span>
              <input
                type="color"
                value={/^#[0-9a-fA-F]{6}$/.test(band.colour) ? band.colour : DEFAULT_BAND_COLOUR}
                onChange={(ev) =>
                  onChange({ style: "bands", bands: bands.map((b, i) => (i === index ? { ...b, colour: ev.target.value } : b)) })
                }
              />
              <button
                type="button"
                className="action icon"
                title="remove band"
                onClick={() => onChange({ style: "bands", bands: bands.filter((_, i) => i !== index) })}
              >
                ✕
              </button>
              <RowHead>{bandCoverage(band)}</RowHead>
            </Row>
          ))}
          <Row>
            <RowHead>↳↳</RowHead>
            <button
              type="button"
              className="action"
              onClick={() => onChange({ style: "bands", bands: [...bands, { lo: null, hi: null, colour: DEFAULT_BAND_COLOUR }] })}
            >
              + band
            </button>
            <RowHead>half-open [lo, hi); blank = open end; must be sorted &amp; non-overlapping (server validates)</RowHead>
          </Row>
          {bandGapNote(bands) !== "" && (
            <Row>
              <RowHead>{bandGapNote(bands)}</RowHead>
            </Row>
          )}
        </>
      )}
    </>
  );
}
