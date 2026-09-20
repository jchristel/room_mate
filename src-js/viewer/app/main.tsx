// The React viewer's entry point, served at /viewer/ during the port.
//
// The stylesheet is imported here rather than linked from index.html so Vite
// emits it as this build's own asset; tokens.css stays a link, because it is
// shared with the other three pages and served from static/.

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App.js";
import { allHandles } from "./zoneRegistry.js";
import { elementsOnStorey, layerPayload } from "./layers.js";
import { getState } from "./store.js";
import "./viewer.css";

// A console handle, and a deliberate one.
//
// The old page kept everything in module-scope globals, so "what is this zone
// actually drawing" was answerable from the console — which is how every
// renderer bug in this codebase was actually found, including the two that
// passed their unit tests. Modules close that off, and a port whose only check
// is a human comparing two windows cannot afford to lose it. Read-only, and
// under one name.
(globalThis as unknown as { roommate: unknown }).roommate = {
  getState,
  zones: allHandles,
  elementsOnStorey,
  layerPayload,
};

const root = document.getElementById("root");
if (!root) throw new Error("no #root: index.html and main.tsx disagree");
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
