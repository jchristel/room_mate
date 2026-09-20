// One zone's visibility menu: a checkbox per layer, behind one button.
//
// **Per zone, where the old page had eight page-level buttons.** Two zones on
// one storey — one showing ceilings, one not — is the comparison this makes
// possible, and it is the reason the layers moved onto `ZoneRow`. The cost is
// stated in the plan's critique: "is FF&E on?" stops having a single answer,
// which is why the button carries the count rather than only a caret.
//
// **Each element layer keeps its storey-match suffix**, computed for THIS
// zone's storey. A fallback nobody can see — a layer resolving by elevation, or
// showing every level — is the failure this area keeps producing, and hiding it
// behind a closed menu would be a new way to produce it. So the button also
// carries a marker when any shown layer fell back.

import { DropMenu } from "./DropMenu.js";
import { LAYERS, layerToggleLabel } from "./layers.js";
import { setZoneLayer, setZoneSpacesModel, type ZoneRow } from "./store.js";
import { useViewer } from "./useViewer.js";

export function LayerMenu({ zone }: { zone: ZoneRow }) {
  const { spacesModels } = useViewer();

  const on = LAYERS.filter((l) => zone.layers[l.entity]).length + (zone.showRooms ? 1 : 0) + (zone.showLabels ? 1 : 0);
  // A layer whose storey match was a GUESS, surfaced on the closed button:
  // the suffix inside the menu says which, this says that.
  const fellBack = LAYERS.some((l) => zone.layers[l.entity] && layerToggleLabel(l, zone.levelId).includes("("));

  return (
    <DropMenu label={`Layers: ${on}${fellBack ? " ⚠" : ""}`} title="Which layers this zone draws">
      <label>
        <input type="checkbox" checked={zone.showRooms} onChange={(e) => setZoneLayer(zone.id, { showRooms: e.target.checked })} />{" "}
        Rooms
      </label>
      <label>
        <input type="checkbox" checked={zone.showLabels} onChange={(e) => setZoneLayer(zone.id, { showLabels: e.target.checked })} />{" "}
        Labels
      </label>
      {LAYERS.map((layer) => (
        <div key={layer.entity}>
          <label>
            <input
              type="checkbox"
              checked={zone.layers[layer.entity]}
              onChange={(e) => setZoneLayer(zone.id, { layers: { [layer.entity]: e.target.checked } })}
            />{" "}
            {layerToggleLabel(layer, zone.levelId)}
          </label>
          {/* The services-model picker belongs to the Spaces entry, and only
              appears when there is a choice: it exists because RHH keeps one
              services file per service, so "all models" stacks four
              near-identical outlines on every room in one colour. */}
          {layer.entity === "spaces" && zone.layers.spaces && spacesModels.length > 1 ? (
            <select
              className="ctl"
              title="Which services model's spaces this zone draws"
              value={zone.spacesModel}
              onChange={(e) => setZoneSpacesModel(zone.id, e.target.value)}
            >
              <option value="">All models ({spacesModels.length})</option>
              {spacesModels.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          ) : null}
        </div>
      ))}
    </DropMenu>
  );
}
