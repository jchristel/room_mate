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
import { setZoneFfeModel, setZoneLayer, setZoneSpacesModel, type ZoneRow } from "./store.js";
import type { TagEntity } from "../../renderer/elementTags.js";
import { TAG_ENTITIES } from "../elementTags.js";
import { useViewer } from "./useViewer.js";

function isTagEntity(entity: string): entity is TagEntity {
  return (TAG_ENTITIES as readonly string[]).includes(entity);
}

export function LayerMenu({ zone }: { zone: ZoneRow }) {
  const { spacesModels, ffeModels } = useViewer();

  const on = LAYERS.filter((l) => zone.layers[l.entity]).length + (zone.showRooms ? 1 : 0) + (zone.showLabels ? 1 : 0) + (zone.showRouteTags ? 1 : 0) + (zone.showReachTags ? 1 : 0) + TAG_ENTITIES.filter((e) => zone.tags[e]).length;
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
      <label title="Write each hop's length on the routes, and the walk so far in brackets">
        <input type="checkbox" checked={zone.showRouteTags} onChange={(e) => setZoneLayer(zone.id, { showRouteTags: e.target.checked })} />{" "}
        Route tags
      </label>
      <label title="Write the walk from the nearest start room on each cell of the reach map">
        <input type="checkbox" checked={zone.showReachTags} onChange={(e) => setZoneLayer(zone.id, { showReachTags: e.target.checked })} />{" "}
        Reach tags
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
          {/* A layer's tag: the text written on each element, sized to it. Offered for
              the four layers that have a settings entry for it, and only while the
              layer is on, since text on an element nobody can see is a floating caption. */}
          {isTagEntity(layer.entity) && zone.layers[layer.entity] ? (
            <label className="sub" title="Write each element's tag on it, as set in the project's Element tags">
              <input
                type="checkbox"
                checked={zone.tags[layer.entity]}
                onChange={(e) => setZoneLayer(zone.id, { tags: { [layer.entity]: e.target.checked } })}
              />{" "}
              Tags
            </label>
          ) : null}
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
          {/* FF&E can come from several models at once (one per fit-out package),
              so it is a set of checkboxes and not a single choice: each model is
              drawn unless unticked. */}
          {layer.entity === "ffe" && zone.layers.ffe && ffeModels.length > 1
            ? ffeModels.map((m) => (
                <label key={m} className="sub" title="Draw this model's FF&E in this zone">
                  <input
                    type="checkbox"
                    checked={!zone.ffeModelsOff.includes(m)}
                    onChange={(e) => setZoneFfeModel(zone.id, m, e.target.checked)}
                  />{" "}
                  {m}
                </label>
              ))
            : null}
        </div>
      ))}
    </DropMenu>
  );
}
