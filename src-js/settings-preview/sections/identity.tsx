// Identity, the coordinate anchor, and the milestone-comparison keys.
//
// **Two of these have never had a UI**, which is why they are here rather than
// only in the TOML: `anchor_model` (which model's coordinate space the plans are
// drawn in) and the comparison key/properties, which `comparison.html` writes
// but nothing on a settings page ever showed. A field only editable by hand in a
// file is one nobody knows exists.

import { Block, Check, Field, OptionalText, Row, TextInput } from "../fields.js";
import { StringList } from "../stringList.js";
import type { Settings } from "../types.js";

export function IdentitySection({
  settings,
  edit,
  file,
  isNew,
  modelIds,
  propertyListId,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
  file: string | null;
  isNew: boolean;
  /** Model ids the server holds data for — the anchor's vocabulary. */
  modelIds: readonly string[];
  propertyListId: string;
}) {
  return (
    <>
      <Block
        title="Identity"
        hint={
          isNew
            ? "The id is matched against pushed payloads' project.id and becomes the settings file name. The display name is a label only, and can be changed at any time."
            : `Stored in ${file ?? "?"}. The id is the project's identity — create a new project to change it. The display name is a label only, and can be changed at any time.`
        }
      >
        <Row>
          <Field label="project id">
            <TextInput
              value={settings.project_id}
              size={28}
              readOnly={!isNew}
              onChange={(value) => edit((s) => ({ ...s, project_id: value }))}
            />
          </Field>
          <Field label="display name">
            <OptionalText
              value={settings.name}
              size={28}
              placeholder="(defaults to the project id)"
              onChange={(value) => edit((s) => ({ ...s, name: value }))}
            />
          </Field>
        </Row>
        <Row>
          <Check
            checked={settings.is_default}
            onChange={(checked) => edit((s) => ({ ...s, is_default: checked }))}
          >
            serve as default for unregistered projects
          </Check>
        </Row>
      </Block>

      <Block
        title="Coordinates"
        hint={
          <>
            Which model&apos;s coordinate space this project&apos;s plans are drawn in — every linked model is placed
            against it. Blank derives one: the lexicographically smallest model id that declares a placement. Which
            model is chosen cannot make the geometry wrong, but it decides where exported coordinates (SVG,
            /areas) sit, and a derived anchor <em>moves</em> if a lower-sorting model id is ever pushed. A name
            matching no placed model is a warning, not an error — the derived anchor answers instead.
          </>
        }
      >
        <Row>
          <Field label="anchor model">
            <OptionalText
              value={settings.anchor_model}
              size={28}
              placeholder="(derived: smallest model id)"
              list="anchorModelOptions"
              onChange={(value) => edit((s) => ({ ...s, anchor_model: value }))}
            />
          </Field>
          <datalist id="anchorModelOptions">
            {modelIds.map((id) => (
              <option key={id} value={id} />
            ))}
          </datalist>
        </Row>
      </Block>

      <Block
        title="Milestone comparison"
        hint={
          <>
            How rooms are matched across milestones, and which of their properties the comparison reports.{" "}
            <code>comparison.html</code> writes these too; they are shown here because a project saved from the old
            settings page could never see them. Unset means the comparison reports &quot;no comparison key
            configured&quot; rather than falling back to the room id.
          </>
        }
      >
        <Row>
          <Field label="comparison key">
            <OptionalText
              value={settings.comparison_key}
              size={20}
              placeholder="(none — no room matching)"
              list={propertyListId}
              onChange={(value) => edit((s) => ({ ...s, comparison_key: value }))}
            />
          </Field>
        </Row>
        <Row>
          <span className="row-head">compared properties</span>
        </Row>
        <StringList
          values={settings.comparison_properties}
          list={propertyListId}
          placeholder="Area"
          onChange={(next) => edit((s) => ({ ...s, comparison_properties: next }))}
        />
      </Block>
    </>
  );
}
