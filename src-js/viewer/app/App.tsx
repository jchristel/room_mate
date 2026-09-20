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

import { Header } from "./Header.js";
import { PortNotice } from "./PortNotice.js";
import { startPolling } from "./poll.js";

export function App() {
  // One loop for the page, started once and stopped on unmount — StrictMode
  // runs an effect twice in development, and a loop with no cleanup would
  // double the request rate against a server this page polls every 2s.
  useEffect(() => startPolling(), []);

  return (
    <>
      <Header />
      <div id="mainRow">
        <main id="zones">
          <PortNotice />
        </main>
      </div>
    </>
  );
}
