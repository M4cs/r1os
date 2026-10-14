import type { StateModel } from "@json-render/core";

import type { CompiledApp } from "./types";

/** The initial state model (contract §2): empty query results, setting defaults, a copy of `state`. */
export function initialModel(app: CompiledApp): StateModel {
  const q: Record<string, unknown> = {};
  for (const def of app.queries) q[def.name] = [];
  const settings: Record<string, unknown> = {};
  for (const [key, def] of Object.entries(app.settings)) settings[key] = def.default;
  // `/player` (ADR-0040): nothing playing until the host says otherwise.
  const player = { playing: false, state: "stopped", title: "", subtitle: "", song: "", url: "", error: "" };
  // `/logic` (ADR-0041): each value the script publishes, empty until it does.
  const logic: Record<string, unknown> = {};
  for (const name of app.logic?.values ?? []) logic[name] = null;
  return { q, settings, state: structuredClone(app.state ?? {}), services: {}, player, logic };
}

/** `/settings/<key>` updates from host values, falling back to each setting's default. */
export function settingsUpdates(app: CompiledApp, values: Record<string, unknown> | undefined): Record<string, unknown> {
  const updates: Record<string, unknown> = {};
  for (const [key, def] of Object.entries(app.settings)) updates[`/settings/${key}`] = values?.[key] ?? def.default;
  return updates;
}

/** `/state/<key>` updates from the host's kept values (`state._persist`): only the keys the app keeps now, and only
 * values shaped like the key's initial value (an edit may have turned text into a list). */
export function keptStateUpdates(app: CompiledApp, kept: Record<string, unknown> | undefined): Record<string, unknown> {
  const updates: Record<string, unknown> = {};
  for (const key of app.persist ?? []) {
    if (!kept || !Object.prototype.hasOwnProperty.call(kept, key)) continue;
    const value = kept[key];
    const initial = app.state[key];
    if (value === null || initial === null || initial === undefined || kindOf(value) === kindOf(initial)) {
      updates[`/state/${key}`] = value;
    }
  }
  return updates;
}

function kindOf(value: unknown): string {
  return Array.isArray(value) ? "array" : typeof value;
}
