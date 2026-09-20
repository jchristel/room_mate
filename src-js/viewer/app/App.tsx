// The React viewer, served at /viewer/ while it is being ported.
//
// Ported slice by slice from the hand-written page it replaced
// (docs/Superseded/PLAN-viewer-react.md), each slice checked against that page
// running beside it until the cutover deleted it.
//
// The elements keep the OLD PAGE'S IDS (`mainRow`, `zones`, `inspector`,
// `bottomRegion`) because `viewer.css` was moved whole and still selects on
// them. A page written from scratch would use class names; this one was not,
// and a stylesheet rewritten alongside the markup would have made a visual
// difference impossible to attribute during the port.

import { useEffect, useState } from "react";

import { Grid } from "./Grid.js";
import { AdjacencyBand } from "./AdjacencyBand.js";
import { BandDivide, BandSplit, RegionDrag } from "./DragHandles.js";
import { AreasBand } from "./AreasBand.js";
import { QaBand } from "./QaBand.js";
import { Header } from "./Header.js";
import { Inspector } from "./inspector/Inspector.js";
import { PickList } from "./PickList.js";
import { startPolling } from "./poll.js";
import { useViewer } from "./useViewer.js";
import { Zone } from "./Zone.js";

export function App() {
  const [areasOpen, setAreasOpen] = useState(false);
  const [adjOpen, setAdjOpen] = useState(false);
  const [qaOpen, setQaOpen] = useState(false);
  // One loop for the page, started once and stopped on unmount — StrictMode
  // runs an effect twice in development, and a loop with no cleanup would
  // double the request rate against a server this page polls every 2s.
  useEffect(() => startPolling(), []);

  const { zones } = useViewer();

  return (
    <>
      <Header />
      <div id="mainRow">
        {/* The column count follows the zone count, capped at 3 — beyond that
            the zones wrap into rows rather than becoming slivers. The old page
            wrote this same rule onto the element from JS. */}
        <main id="zones" style={{ gridTemplateColumns: `repeat(${Math.min(zones.length, 3)}, 1fr)` }}>
          {zones.map((zone) => (
            <Zone key={zone.id} zone={zone} />
          ))}
        </main>
        <Inspector />
      </div>
      {/* Above everything, positioned at the click it answers. */}
      <PickList />
      {/* The bottom region, band 2. One instance per page, never per zone: it
          is scope-derived, and a region that multiplied with zone count would
          stop being a stable place a reader can point at. Band 1 -- QA, areas
          and adjacency -- lands in B8. */}
      <footer id="bottomRegion">
        <RegionDrag />
        {/* Band 1: page-level RESULTS, derived from the scope rather than from
            any one zone — which is why there is one of each, never one per
            zone. Areas and adjacency join QA here in the rest of B8. */}
        <div id="band1">
          <QaBand open={qaOpen} onToggle={() => setQaOpen(!qaOpen)} />
          <BandSplit
            visible={qaOpen && areasOpen}
            aboveId="qaBand"
            title="Drag to split space between QA and Hierarchy areas"
          />
          <AreasBand open={areasOpen} onToggle={() => setAreasOpen(!areasOpen)} />
          <BandSplit
            visible={areasOpen && adjOpen}
            aboveId="areasBand"
            title="Drag to split space between Hierarchy areas and Adjacency"
          />
          <AdjacencyBand open={adjOpen} onToggle={() => setAdjOpen(!adjOpen)} />
        </div>
        <BandDivide visible={areasOpen || adjOpen || qaOpen} />
        <Grid />
      </footer>
    </>
  );
}
