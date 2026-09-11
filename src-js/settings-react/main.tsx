// Entry point for the React preview of the settings page. What the preview is
// for, and what it is compared against, is in App.tsx's header.

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App.js";
import "./style.css";

const root = document.getElementById("root");
if (!root) throw new Error("settings-react: index.html has no #root to mount into");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
