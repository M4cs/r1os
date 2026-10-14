// Turn a compiled view into the json-render Spec the renderer gets.
//
// `data-enee-id`: json-render 0.21.0 passes components `element` but not its key (the key only reaches the DOM as
// `data-jr-key` on `display:contents` wrapper spans while devtools are active). For point-and-edit we want the id on
// the component's own root node, so normalization copies each element key into a reserved prop, `ENEE_ID_PROP`,
// and every catalog component spreads `eneeId(props)` onto its root. The compiled artifact itself is not modified.

import { type Spec, type UIElement, validateSpec } from "@json-render/core";

import type { ViewSpec } from "./types";

export const ENEE_ID_PROP = "data-enee-id";

/** Spread onto a component's root element: `<div {...eneeId(props)}>`. */
export function eneeId(props: object): { "data-enee-id"?: string } {
  const id = (props as Record<string, unknown>)[ENEE_ID_PROP];
  return typeof id === "string" ? { "data-enee-id": id } : {};
}

export function normalizeView(view: ViewSpec): Spec {
  const elements: Record<string, UIElement> = {};
  for (const [key, el] of Object.entries(view.elements)) {
    elements[key] = {
      ...(el as UIElement),
      props: { ...(el.props ?? {}), [ENEE_ID_PROP]: key },
    };
  }
  return { root: view.root, elements };
}

/** Structural problems json-render would otherwise swallow silently (missing children, misplaced `on`, …). */
export function specErrors(spec: Spec): string[] {
  const { issues } = validateSpec(spec);
  return issues.filter((i) => i.severity === "error").map((i) => (i.elementKey ? `${i.elementKey}: ${i.message}` : i.message));
}
