// The area policy: what the area figure MEANS, and the one tolerance the
// geometry needs.
//
// **Never had a UI**, and it is the section with the most consequence per field:
// `measurement_standard` is what a number may be *called* to anyone outside the
// project (STRATEGY-AREA-CALCULATION.md), and `max_wall_thickness` is the
// ceiling that decides whether a gap between two rooms is a wall to bridge or a
// courtyard to leave open.

import { useState } from "react";

import { Block, Field, NumberInput, OptionalSelect, Row } from "../fields.js";
import type { AreaPolicy, MeasurementStandard, RoomBoundary, Settings } from "../types.js";

/** The Rust default, `AreaPolicy::DEFAULT_MAX_WALL_THICKNESS_FT`. A project with
 *  no `[areas]` section behaves as this, so the editor shows it rather than a
 *  blank that would read as "no ceiling". */
const DEFAULT_MAX_WALL_THICKNESS_FT = 1.5;

const STANDARDS: readonly (readonly [MeasurementStandard, string])[] = [
  ["IPMS1", "IPMS 1"],
  ["IPMS2", "IPMS 2"],
  ["IPMS3", "IPMS 3"],
  ["DIN277", "DIN 277"],
  ["SIA416", "SIA 416"],
  ["BOMA", "BOMA"],
  ["RICS", "RICS"],
];

const BOUNDARIES: readonly (readonly [RoomBoundary, string])[] = [
  ["centreline", "centreline — rooms tile edge to edge"],
  ["finish_face", "finish face — rooms float inside their walls"],
];

export function AreasSection({
  settings,
  edit,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
}) {
  const areas: AreaPolicy = settings.areas ?? { max_wall_thickness: DEFAULT_MAX_WALL_THICKNESS_FT };
  // Held as text while it is being typed -- see `NumberInput`. Seeded once per
  // mounted editor, which is per project: the editor is keyed by project id.
  const [thickness, setThickness] = useState(String(areas.max_wall_thickness));

  const editAreas = (change: (a: AreaPolicy) => AreaPolicy) => edit((s) => ({ ...s, areas: change(areas) }));

  return (
    <Block
      title="Areas"
      hint={
        <>
          What this project&apos;s area figures mean, and how far apart two rooms may be before the gap stops being a
          wall. Read STRATEGY-AREA-CALCULATION.md before quoting an area to anyone outside the project.
        </>
      }
    >
      <Row>
        <Field label="measurement standard">
          <OptionalSelect
            value={areas.measurement_standard}
            options={STANDARDS}
            absentLabel="— undeclared (reported as such) —"
            title="What the reported area figure is measured to. Undeclared is honest, and is echoed as undeclared rather than presented as any standard."
            onChange={(value) => editAreas((a) => ({ ...a, measurement_standard: value }))}
          />
        </Field>
        <Field label="max wall thickness (ft)">
          <NumberInput
            text={thickness}
            title="The widest gap between finish-face rooms still counted as a wall; anything wider is a real void (courtyard, atrium, lightwell) and stays open. Must be positive and finite."
            onChange={(text, parsed) => {
              setThickness(text);
              // Keep the last good number when the text is mid-edit: a blank box
              // is a keystroke, not a decision, and the server rejects a
              // non-positive value anyway.
              if (parsed !== null) editAreas((a) => ({ ...a, max_wall_thickness: parsed }));
            }}
          />
        </Field>
      </Row>
      <Row>
        <Field label="boundary fallback">
          <OptionalSelect
            value={areas.boundary_location}
            options={BOUNDARIES}
            absentLabel="— none: fall back to finish face —"
            title="Fallback ONLY for models whose extractor predates the boundary regime on the envelope. A model that states its own regime always wins."
            onChange={(value) => editAreas((a) => ({ ...a, boundary_location: value }))}
          />
        </Field>
        <span className="row-head">
          a model that declares its own regime always wins — this answers only for ones that do not
        </span>
      </Row>
    </Block>
  );
}
