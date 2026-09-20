// The React viewer, served at /viewer/ while it is being ported.
//
// **Parity port, slice by slice** (docs/PLAN-viewer-react.md). This file grows
// one area at a time — scope and data, zones, layers, selection, the grid, the
// bands, export — and each slice is checked against `static/index.html`, still
// running at `/`, on the same project in two windows.
//
// The elements keep the OLD PAGE'S IDS (`mainRow`, `zones`, `inspector`,
// `bottomRegion`) because `viewer.css` was moved whole and still selects on
// them. A React page written from scratch would use class names; this one is
// not written from scratch, and a stylesheet rewritten alongside the markup
// would make a visual difference impossible to attribute.

import { useEffect } from "react";

import { Grid } from "./Grid.js";
import { QaBand } from "./QaBand.js";
import { Header } from "./Header.js";
import { Inspector } from "./inspector/Inspector.js";
import { startPolling } from "./poll.js";
import { useViewer } from "./useViewer.js";
import { Zone } from "./Zone.js";

export function App() {
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
      {/* The bottom region, band 2. One instance per page, never per zone: it
          is scope-derived, and a region that multiplied with zone count would
          stop being a stable place a reader can point at. Band 1 -- QA, areas
          and adjacency -- lands in B8. */}
      <footer id="bottomRegion">
        {/* Band 1: page-level RESULTS, derived from the scope rather than from
            any one zone — which is why there is one of each, never one per
            zone. Areas and adjacency join QA here in the rest of B8. */}
        <div id="band1">
          <QaBand />
        </div>
        <Grid />
      </footer>
    </>
  );
}
