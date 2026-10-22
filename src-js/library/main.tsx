// Entry point for the library page. What the page is for is in App.tsx's header.

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App.js";
import "./style.css";

const root = document.getElementById("root");
if (!root) throw new Error("library: index.html has no #root to mount into");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
