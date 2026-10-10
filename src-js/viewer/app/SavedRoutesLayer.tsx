// The saved routes' reads: the project's connections document, and the path of
// every route a reader has switched on.
//
// Renders nothing. It is page-level and not a zone's because a saved route is
// shared project data drawn in every zone at once (each draws its own level's
// slice), and the document is also what the editor and the route bar write.
//
// **A route is re-asked whenever anything it depends on moves**: the scope, the
// rooms, or the document itself, since a saved zone or link is what changes which
// way a route can go. Nothing about a path is stored, so a route follows the
// building and says so when it cannot (a room that left the model is a message on
// the route, not a stale line).

import { useEffect } from "react";

import { connectionsUrl } from "../connections.js";
import { connectivityUrl, type Clearance, type RoutePath, type RouteResult } from "../route.js";
import { request } from "./connectionsApi.js";
import { pruneSavedRoutes, setConnections, setSavedRouteResult } from "./store.js";
import { useViewer } from "./useViewer.js";

export function SavedRoutesLayer(): null {
  const { scope, payload, connections, savedRoutes } = useViewer();
  const url = connectionsUrl(scope);
  const doc = connections.projectId === scope.projectId ? connections.doc : null;
  const revision = payload?.revision ?? "";
  const scopeKey = `${scope.projectId}|${scope.building}|${scope.milestone}`;

  // The document, on opening and when the project changes. The editor and the
  // route bar keep it current after their own saves.
  useEffect(() => {
    if (!url) return;
    const ac = new AbortController();
    void request(url, { signal: ac.signal }).then((r) => {
      if (ac.signal.aborted) return;
      setConnections(
        r.ok
          ? { projectId: scope.projectId, doc: r.body, error: null }
          : { projectId: scope.projectId, doc: null, error: r.message },
      );
    });
    return () => ac.abort();
    // `scope` is spread into the key; the object itself changes identity freely.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [url]);

  // A deleted route, or another project's, is not shown any more.
  useEffect(() => {
    if (doc) pruneSavedRoutes(new Set(doc.routes.map((r) => r.id)));
  }, [doc]);

  const shownKey = [...savedRoutes.shown].sort().join(",");
  const version = doc?.taken_at ?? "";
  useEffect(() => {
    if (!doc) return;
    const wanted = doc.routes.filter((r) => savedRoutes.shown.has(r.id));
    if (wanted.length === 0) return;
    const ac = new AbortController();
    for (const route of wanted) {
      const routeUrl = connectivityUrl(
        scope,
        route.from.room_id,
        route.to.room_id,
        route.method ?? null,
        { from: route.from_at ?? null, to: route.to_at ?? null },
        { from: route.from.model_id, to: route.to.model_id },
        route.width_mm ?? null,
        route.height_mm ?? null,
      );
      if (!routeUrl) continue;
      setSavedRouteResult(route.id, { state: "loading" });
      void answer(routeUrl, ac.signal).then((result) => {
        if (!ac.signal.aborted) setSavedRouteResult(route.id, result);
      });
    }
    return () => ac.abort();
    // The key stands for `shown`, and the doc's version for its routes.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shownKey, version, scopeKey, revision]);

  return null;
}

async function answer(url: string, signal: AbortSignal): Promise<RouteResult> {
  try {
    const res = await fetch(url, { cache: "no-store", signal });
    if (res.status === 204) return { state: "error", message: "Nothing has been pushed for this project." };
    if (!res.ok) return { state: "error", message: (await res.text()).trim() || `${url} -> ${res.status}` };
    const body = (await res.json()) as { path?: RoutePath | null; clearance?: Clearance };
    return body.path
      ? { state: "done", path: body.path, ...(body.clearance ? { clearance: body.clearance } : {}) }
      : { state: "error", message: "The server returned no route." };
  } catch (err) {
    return { state: "error", message: signal.aborted ? "" : `Could not read ${url}: ${String(err)}` };
  }
}
