// Entry point for the reports page. What the page is for, and what it replaced,
// is in App.tsx's header.

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App.js";
import "./style.css";

const root = document.getElementById("root");
if (!root) throw new Error("reports: index.html has no #root to mount into");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
