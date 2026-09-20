// The React viewer's entry point, served at /viewer/ during the port.
//
// The stylesheet is imported here rather than linked from index.html so Vite
// emits it as this build's own asset; tokens.css stays a link, because it is
// shared with the other three pages and served from static/.

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App.js";
import "./viewer.css";

const root = document.getElementById("root");
if (!root) throw new Error("no #root: index.html and main.tsx disagree");
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
