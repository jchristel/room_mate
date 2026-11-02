// Classification tiers, the exclusions that withhold rooms from the aggregated
// footprints, and the canonical-property mappings.
//
// **`hierarchy_exclusions` has never had a UI.** It is the one section here that
// removes data from an answer — an excluded group or room is absent from the
// hierarchy areas — so editing it by hand in TOML meant the exclusions were
// invisible to whoever later read the areas table and wondered why it did not
// add up.

import { Block, Field, ListControls, OptionalText, Row, RowHead, Select, TextInput } from "../fields.js";
import { StringList } from "../stringList.js";
import type { BuiltinPropertyDef, HierarchyExclusion, HierarchyTier, Settings } from "../types.js";

const EXCLUSION_KINDS = [
  ["group", "a whole group (tier + value)"],
  ["rooms", "named room ids"],
] as const;

export function HierarchySection({
  settings,
  edit,
  propertyListId,
}: {
  settings: Settings;
  edit: (change: (s: Settings) => Settings) => void;
  propertyListId: string;
}) {
  const tiers = settings.hierarchy;
  const exclusions = settings.hierarchy_exclusions ?? [];
  const builtins = settings.builtin_properties;

  const setTiers = (next: HierarchyTier[]) => edit((s) => ({ ...s, hierarchy: next }));
  const setExclusions = (next: HierarchyExclusion[]) => edit((s) => ({ ...s, hierarchy_exclusions: next }));
  const setBuiltins = (next: BuiltinPropertyDef[]) => edit((s) => ({ ...s, builtin_properties: next }));

  return (
    <>
      <Block
        title="Hierarchy"
        hint='Classification tiers, outermost first. Each tier needs a code property and/or a name property. A tier named "Building" powers the viewer&apos;s building filter.'
      >
        <div className="rows">
          {tiers.map((tier, index) => (
            <Row key={`${index}:${tier.name}`}>
              <RowHead>{index + 1}.</RowHead>
              <span>tier</span>
              <TextInput
                value={tier.name}
                size={12}
                placeholder="Building"
                onChange={(value) => setTiers(tiers.map((t, i) => (i === index ? { ...t, name: value } : t)))}
              />
              <span>code prop</span>
              <OptionalText
                value={tier.code_property}
                size={14}
                list={propertyListId}
                onChange={(value) => setTiers(tiers.map((t, i) => (i === index ? { ...t, code_property: value } : t)))}
              />
              <span>name prop</span>
              <OptionalText
                value={tier.name_property}
                size={14}
                list={propertyListId}
                onChange={(value) => setTiers(tiers.map((t, i) => (i === index ? { ...t, name_property: value } : t)))}
              />
              <ListControls list={tiers} index={index} onChange={setTiers} />
            </Row>
          ))}
        </div>
        <button
          type="button"
          className="action"
          onClick={() => setTiers([...tiers, { name: "", code_property: null, name_property: null }])}
        >
          + tier
        </button>
      </Block>

      <Block
        title="Hierarchy exclusions"
        hint="Rooms withheld from the aggregated hierarchy footprints — a whole group (every room whose tier carries a value), or named room ids. Nothing is excluded by default, and an exclusion changes the areas table without changing any room."
      >
        <div className="rows">
          {exclusions.map((exclusion, index) => (
            <Row key={index}>
              <RowHead>{index + 1}.</RowHead>
              <Select
                value={exclusion.match}
                options={EXCLUSION_KINDS}
                onChange={(kind) =>
                  setExclusions(
                    exclusions.map((e, i) =>
                      i !== index ? e : kind === "group" ? { match: "group", tier: "", value: "" } : { match: "rooms", ids: [] },
                    ),
                  )
                }
              />
              {exclusion.match === "group" ? (
                <>
                  <span>tier</span>
                  <TextInput
                    value={exclusion.tier}
                    size={12}
                    onChange={(value) =>
                      setExclusions(exclusions.map((e, i) => (i === index && e.match === "group" ? { ...e, tier: value } : e)))
                    }
                  />
                  <span>value</span>
                  <TextInput
                    value={exclusion.value}
                    size={14}
                    onChange={(value) =>
                      setExclusions(exclusions.map((e, i) => (i === index && e.match === "group" ? { ...e, value } : e)))
                    }
                  />
                </>
              ) : (
                <StringList
                  values={exclusion.ids}
                  placeholder="room id"
                  addLabel="+ id"
                  onChange={(ids) =>
                    setExclusions(exclusions.map((e, i) => (i === index && e.match === "rooms" ? { ...e, ids } : e)))
                  }
                />
              )}
              <ListControls list={exclusions} index={index} onChange={setExclusions} />
            </Row>
          ))}
        </div>
        <button type="button" className="action" onClick={() => setExclusions([...exclusions, { match: "group", tier: "", value: "" }])}>
          + exclusion
        </button>
      </Block>

      <Block
        title="Builtin properties"
        hint='Canonical property name → each source&apos;s raw property name (e.g. canonical "Area" backed by revit&apos;s "Fläche"). Only needed when a raw name differs from the canonical one.'
      >
        <div className="rows">
          {builtins.map((builtin, index) => (
            <Row key={`${index}:${builtin.canonical}`}>
              <span>canonical</span>
              <TextInput
                value={builtin.canonical}
                size={12}
                placeholder="Area"
                onChange={(value) => setBuiltins(builtins.map((b, i) => (i === index ? { ...b, canonical: value } : b)))}
              />
              <span>←</span>
              {Object.entries(builtin.by_source).map(([source, raw]) => (
                <Field key={source} label={`${source}:`}>
                  <TextInput
                    value={raw}
                    size={12}
                    title={`raw property name in ${source}`}
                    onChange={(value) =>
                      setBuiltins(
                        builtins.map((b, i) => (i === index ? { ...b, by_source: { ...b.by_source, [source]: value } } : b)),
                      )
                    }
                  />
                </Field>
              ))}
              <SourceAdder
                existing={Object.keys(builtin.by_source)}
                onAdd={(source) =>
                  setBuiltins(
                    builtins.map((b, i) => (i === index ? { ...b, by_source: { ...b.by_source, [source]: "" } } : b)),
                  )
                }
              />
              <ListControls list={builtins} index={index} onChange={setBuiltins} />
            </Row>
          ))}
        </div>
        <button
          type="button"
          className="action"
          onClick={() => setBuiltins([...builtins, { canonical: "", by_source: { revit: "" } }])}
        >
          + property
        </button>
      </Block>
    </>
  );
}

/** Adds one source mapping to a builtin property. A tiny prompt-free control:
 *  the source names are free text (`revit`, an IFC exporter later), so it is a
 *  box plus a button rather than a picker over a closed list. */
function SourceAdder({ existing, onAdd }: { existing: readonly string[]; onAdd: (source: string) => void }) {
  return (
    <button
      type="button"
      className="action icon"
      title="add source mapping"
      onClick={() => {
        // `revit` first, then revit-2, …: a name that already maps would silently
        // overwrite the mapping beside it.
        let name = "revit";
        for (let n = 2; existing.includes(name); n += 1) name = `revit-${n}`;
        onAdd(name);
      }}
    >
      +
    </button>
  );
}
