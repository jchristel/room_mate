// The header's layer toggles, plus the two that are not layers at all.
//
// **Every toggle carries its storey match**, which is the point of the
// "(by elevation)" and "(all levels)" suffixes: a fallback nobody can see is
// the failure mode this area keeps producing — a reader switches a layer on,
// sees the wrong thing, and has no way to tell the layer from the data.
//
// The match is read from the FIRST zone's storey. With several zones open they
// can disagree, and the old page's answer was "last zone painted wins" for the
// same one-label-per-layer reason; the first is the same call made earlier and
// stated.

import { layerToggleLabel, LAYERS } from "./layers.js";
import { setLayer, setShowLabels, setShowRooms, setSpacesModel } from "./store.js";
import { useViewer } from "./useViewer.js";

export function LayerToggles() {
  const { layers, showRooms, showLabels, zones, spacesModel, spacesModels } = useViewer();
  const levelId = zones[0]?.levelId ?? null;

  return (
    <>
      <button className={`ctl${showLabels ? " on" : ""}`} title="Show or hide room labels (SVG export follows)" onClick={() => setShowLabels(!showLabels)}>
        Labels: {showLabels ? "on" : "off"}
      </button>
      <button
        className={`ctl${showRooms ? " on" : ""}`}
        title="Show or hide the rooms themselves, so the overlays can be read on their own (SVG export keeps them)"
        onClick={() => setShowRooms(!showRooms)}
      >
        Rooms: {showRooms ? "on" : "off"}
      </button>
      {LAYERS.map((layer) => (
        <span key={layer.entity} className="layer-toggle">
          <button
            className={`ctl${layers[layer.entity] ? " on" : ""}`}
            onClick={() => setLayer(layer.entity, !layers[layer.entity])}
          >
            {layerToggleLabel(layer, levelId)}
          </button>
          {/* The spaces model picker rides beside its own toggle, and only
              when there is a choice to make: it exists because RHH keeps one
              services file per service, so "all models" stacks four
              near-identical outlines on every room in one colour. */}
          {layer.entity === "spaces" && layers.spaces && spacesModels.length > 1 ? (
            <select
              className="ctl"
              title="Which services model's spaces to draw"
              value={spacesModel}
              onChange={(e) => setSpacesModel(e.target.value)}
            >
              <option value="">All models ({spacesModels.length})</option>
              {spacesModels.map((m) => (
                <option key={m} value={m}>
                  {m}
                </option>
              ))}
            </select>
          ) : null}
        </span>
      ))}
    </>
  );
}
