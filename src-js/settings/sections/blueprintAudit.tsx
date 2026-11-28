// The room-state audit's tolerances: how close an element has to be to a blueprint's layout to count as "in
// place", "moved", or not the layout's item at all.
//
// Settings rather than constants because the right numbers belong to the office and the model. They only
// decide what the audit REPORTS; nothing is moved or removed by a tolerance.

import { useState } from "react";

import { Block, Field, NumberInput, Row } from "../fields.js";
import {
  BLUEPRINT_AUDIT_DEFAULT,
  DEFAULT_NEAR_TOLERANCE_MM,
  DEFAULT_PLACE_TOLERANCE_MM,
  DEFAULT_TURN_TOLERANCE_DEG,
} from "../defaults.js";
import type { BlueprintAuditPolicy, Settings } from "../types.js";

export function BlueprintAuditSection({
  settings,
  edit,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
}) {
  const audit: BlueprintAuditPolicy = settings.blueprint_audit ?? BLUEPRINT_AUDIT_DEFAULT;
  // Held as text while typed -- see `NumberInput`. All three are required numbers, so an unparseable box
  // keeps the last good value and the server rejects a nonsense one on save.
  const [place, setPlace] = useState(String(audit.place_tolerance_mm));
  const [near, setNear] = useState(String(audit.near_tolerance_mm));
  const [turn, setTurn] = useState(String(audit.turn_tolerance_deg));
  const editAudit = (change: (a: BlueprintAuditPolicy) => BlueprintAuditPolicy) =>
    edit((s) => ({ ...s, blueprint_audit: change(audit) }));

  return (
    <Block
      title="Blueprint audit"
      hint={
        <>
          How the audit judges an element in a room against the blueprint assigned to it. These only change what is{" "}
          <strong>reported</strong>: nothing is moved or removed by a tolerance. An element beyond the near distance is
          not paired with the item at all, so a different bed is never called the same bed.
        </>
      }
    >
      <Row>
        <Field label="in place within (mm)">
          <NumberInput
            text={place}
            size={6}
            placeholder={String(DEFAULT_PLACE_TOLERANCE_MM)}
            title="Within this distance an element is in place. A real Revit run read elements placed from a task back at 0.000 mm, so a small value is right; raise it only if the model snaps or rounds."
            onChange={(text, parsed) => {
              setPlace(text);
              if (parsed !== null) editAudit((a) => ({ ...a, place_tolerance_mm: parsed }));
            }}
          />
        </Field>
        <Field label="same item within (mm)">
          <NumberInput
            text={near}
            size={6}
            placeholder={String(DEFAULT_NEAR_TOLERANCE_MM)}
            title="Beyond the in-place distance and within this one, an element of the right code is the layout's item, moved. Beyond it the element and the item are reported separately. At most 5000."
            onChange={(text, parsed) => {
              setNear(text);
              if (parsed !== null) editAudit((a) => ({ ...a, near_tolerance_mm: parsed }));
            }}
          />
        </Field>
        <Field label="facing agrees within (degrees)">
          <NumberInput
            text={turn}
            size={5}
            placeholder={String(DEFAULT_TURN_TOLERANCE_DEG)}
            title="An element's facing within this many degrees of the layout's agrees. At most 45."
            onChange={(text, parsed) => {
              setTurn(text);
              if (parsed !== null) editAudit((a) => ({ ...a, turn_tolerance_deg: parsed }));
            }}
          />
        </Field>
      </Row>
    </Block>
  );
}
