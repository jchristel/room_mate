// Shared browser helpers for the roommate pages. Kept tiny and dependency-free
// (STRATEGY-BROWSER.md: vanilla JS, no build step). Served from static/ by
// axum's ServeDir and loaded as a classic <script> BEFORE each page's own
// <script>, so these are plain globals — no module wiring.
//
// settings.html and comparison.html both talk to the same settings API in the
// same shape, so the two helpers below were byte-identical copies in each page.
// index.html keeps its own `fetchJson` (a GET-only variant with a different
// error message) and does not use these.

// ---------------------------------------------------------------------------
// Palette (shared by the plan renderer in index.html and the adjacency graph in
// graph.js).
//
// These live here rather than inline in index.html for one reason: two views
// that disagree about what colour a department is are worse than either being
// arbitrary. The plan's hierarchy colour plan and the graph's node colouring
// MUST sample the same stops from the same function — a copy would drift the
// first time a scheme changed. See HANDOVER-adjacency.md "Prerequisites".
//
// Literal hex stops, no d3/npm: the browser layer stays a zero-build vanilla
// page. index.html keeps the colour-plan maths that reads these (sampleScheme,
// lighten) — those are plan-specific, and only the shared vocabulary moved.
// ---------------------------------------------------------------------------

// A few ColorBrewer schemes. Sequential/diverging ones are sampled at t∈[0,1]
// by index.html's `sampleScheme`; the categorical ones (Set2, Paired) are
// indexed by `qualitative` below.
const SCHEMES = {
  RdBu: ["#ca0020", "#f4a582", "#f7f7f7", "#92c5de", "#0571b0"],
  RdYlGn: ["#d7191c", "#fdae61", "#ffffbf", "#a6d96a", "#1a9641"],
  Greens: ["#edf8e9", "#bae4b3", "#74c476", "#31a354", "#006d2c"],
  Blues: ["#eff3ff", "#bdd7e7", "#6baed6", "#3182bd", "#08519c"],
  Set2: ["#66c2a5", "#fc8d62", "#8da0cb", "#e78ac3", "#a6d854", "#ffd92f", "#e5c494", "#b3b3b3"],
  Paired: ["#a6cee3", "#1f78b4", "#b2df8a", "#33a02c", "#fb9a99", "#e31a1c", "#fdbf6f", "#ff7f00", "#cab2d6", "#6a3d9a", "#ffff99", "#b15928"],
};

function hexToRgb(h) {
  const n = parseInt(h.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

function rgbToHex(r, g, b) {
  const c = v => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, "0");
  return `#${c(r)}${c(g)}${c(b)}`;
}

// The k-th distinct hue of a categorical scheme, wrapping. Callers pass a
// stable index (the position of a tier value in a sorted key list), so the same
// department gets the same colour in the plan overlay and in the graph.
function qualitative(scheme, k) {
  const stops = SCHEMES[scheme] || SCHEMES.Set2;
  return stops[k % stops.length];
}

// GET JSON with no-store caching; throws the server's error text (falling back
// to "<url> -> <status>") on a non-2xx so callers surface it verbatim.
async function apiGet(url) {
  const res = await fetch(url, { cache: "no-store" });
  if (!res.ok) throw new Error(await res.text() || `${url} -> ${res.status}`);
  return res.json();
}

// Send JSON (POST/PUT/…). Returns { ok, status, text } WITHOUT throwing, so the
// caller can show the server's 422 validation text verbatim.
async function apiSend(method, url, body) {
  const res = await fetch(url, {
    method,
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await res.text();
  return { ok: res.ok, status: res.status, text };
}

// ---------------------------------------------------------------------------
// Selection persistence (shared by index / settings / comparison).
//
// The three pages are separate static documents linked by plain <a href>, so a
// navigation drops all in-memory state. To keep the user's scope pick across
// pages, reloads, and bookmarks we persist it in two places with a deliberate
// precedence: the URL query wins (so a bookmarked/deep-linked URL is
// authoritative), localStorage is only the cross-page fallback seed, and the
// caller's own default (projects[0]) is the last resort. localStorage carries
// ONLY the project id — it's the one selection every page shares; the viewer's
// building/milestone live in the URL alone (they're per-project and
// viewer-specific, so they must not seed the other pages).
//
// Callers MUST still validate a restored id against the live /projects list
// before using it: a stored id the server no longer lists falls through to the
// default, exactly as an unknown pick does today.
// ---------------------------------------------------------------------------

// The single localStorage key every page reads/writes for the project id.
const LS_PROJECT_KEY = "roommate.project";

// Read a query param from the current URL, or null if absent/empty.
function urlParam(name) {
  const v = new URLSearchParams(location.search).get(name);
  return v ? v : null;
}

// The restore precedence for the project id: URL query > localStorage > null.
// Returns a *candidate* only — the caller still checks it against the server
// list and falls back to its own default if the candidate isn't offered.
function seedProjectId() {
  const fromUrl = urlParam("project");
  if (fromUrl) return fromUrl;
  try {
    return localStorage.getItem(LS_PROJECT_KEY) || null;
  } catch (_) {
    // Private-mode / storage-disabled: treat as no stored seed. Never throw
    // out of a seed read — a blocked storage API must not break page load.
    return null;
  }
}

// Persist the chosen project id: mirror it into the URL (replaceState — this is
// a selection, not a navigation, so it must not add Back-button history) and
// into localStorage as the cross-page seed. Pass extra viewer-only scope in
// `extraParams` (e.g. { building, milestone }) to round-trip it in the URL
// WITHOUT storing it (those keys are dropped when null/empty). A null projectId
// clears both the query and the stored seed.
function persistSelection(projectId, extraParams = {}) {
  const url = new URL(location.href);
  const p = url.searchParams;

  if (projectId) p.set("project", projectId);
  else p.delete("project");

  // Viewer-only scope: present when set, removed when null/empty. These live in
  // the URL only, never in localStorage (they're per-project + viewer-specific).
  for (const [k, v] of Object.entries(extraParams)) {
    if (v) p.set(k, v);
    else p.delete(k);
  }

  history.replaceState(null, "", url);

  try {
    if (projectId) localStorage.setItem(LS_PROJECT_KEY, projectId);
    else localStorage.removeItem(LS_PROJECT_KEY);
  } catch (_) {
    // Storage blocked: the URL still carries the pick for this session, so
    // in-page persistence degrades to URL-only rather than failing.
  }
}
