// The endpoints the model settings page calls.

import { apiGet, apiSend } from "./common.js";
import type { Draft } from "./model.js";

const base = (project: string) => `/projects/${encodeURIComponent(project)}`;

export const getDraft = (project: string, model: string) =>
  apiGet<Draft>(`${base(project)}/modelsettings?model=${encodeURIComponent(model)}`);

/** The server's own text on a refusal, shown verbatim. */
export async function saveDraft(
  project: string,
  model: string,
  settings: Record<string, unknown>,
): Promise<{ ok: true; draft: Draft } | { ok: false; error: string }> {
  const r = await apiSend("PUT", `${base(project)}/modelsettings`, { model_id: model, settings });
  if (!r.ok) return { ok: false, error: r.text || `HTTP ${r.status}` };
  return { ok: true, draft: JSON.parse(r.text) as Draft };
}
