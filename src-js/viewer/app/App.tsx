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

import { PortNotice } from "./PortNotice.js";

export function App() {
  return (
    <>
      <header>
        <h1>Room Plan</h1>
        <div className="links">
          <a href="/">the current viewer</a>
          <a href="/reports/">reports</a>
          <a href="/settings/">settings</a>
        </div>
      </header>
      <div id="mainRow">
        <main id="zones">
          <PortNotice />
        </main>
      </div>
    </>
  );
}
