// Entry point for the tasks page. What the page is for is in App.tsx's header.

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App.js";
import "../ui/base.css";
import "../ui/list-page.css";
import "../ui/sheet/preview.css";
import "./style.css";

const root = document.getElementById("root");
if (!root) throw new Error("tasks: index.html has no #root to mount into");

createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
