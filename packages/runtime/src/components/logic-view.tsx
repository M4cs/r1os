// A view the app's logic builds (r1os ADR-0041): `logic_view: board` shows the tree its script publishes at
// `logic.board`, drawn with the same components as the rest of the app, so the wheel's focus ring, the look and the
// agents' reading of it all hold.
//
// The script's tree is plain and nested: `{ type: "Stack", children: [ { type: "Button", label: "Roll", press:
// "roll" }, … ] }`. Every key but `type`, `children`, `press`, `args` and `bind` is a prop, checked against the
// component's own schema (catalog.ts); a node that fails is left out and reported, so the app can be fixed.
//   - `press: "action"` (and `args`) runs one of the app's actions when it's clicked.
//   - `bind: "key"` ties a Wheel's or a Toggle's value to the app's state key.
//   - A List takes `items` (rows), `title` and `caption` as the rows' field names, and `press`/`args` for its rows'
//     action (`"{field}"` in `args` is the row's field).

import type { Spec, UIElement } from "@json-render/core";
import { Renderer, useStateValue } from "@json-render/react";
import { useEffect, useMemo } from "react";

import { componentDefs } from "../catalog";
import { useRuntime } from "../context";
import { EneeError } from "../types";
import type { Ctx } from "./types";

/** What a script's view may use: the catalog, minus what needs code or typing of its own. */
const ALLOWED = new Set([
  "Stack",
  "Row",
  "Card",
  "Heading",
  "Text",
  "Stat",
  "Timer",
  "Gauge",
  "Progress",
  "Button",
  "Wheel",
  "Toggle",
  "BarChart",
  "LineChart",
  "DonutChart",
  "List",
  "EmptyState",
]);
const CONTAINERS = new Set(["Stack", "Row", "Card"]);
/** At most this many nodes in one view. */
const MAX_NODES = 300;

type Node = Record<string, unknown> & { type?: unknown; children?: unknown };

/** A row template's field (`{i}`), from a field name. */
const field = (name: unknown) => (typeof name === "string" && name ? { i: name } : undefined);

/** The script's tree as json-render's flat spec, and what was wrong with it. */
export function toSpec(tree: unknown, prefix: string): { spec: Spec | null; problems: string[] } {
  const elements: Record<string, UIElement> = {};
  const problems: string[] = [];
  let n = 0;
  const visit = (node: unknown, path: string): string | null => {
    if (++n > MAX_NODES) {
      if (n === MAX_NODES + 1) problems.push(`more than ${MAX_NODES} parts`);
      return null;
    }
    if (!node || typeof node !== "object" || Array.isArray(node)) {
      problems.push(`${path}: not a component ({ type, … })`);
      return null;
    }
    const { type, children, press, args, bind, ...rest } = node as Node;
    if (typeof type !== "string" || !ALLOWED.has(type)) {
      problems.push(`${path}: \`${String(type)}\` isn't a component a script's view can use`);
      return null;
    }
    const props: Record<string, unknown> = { ...rest };
    if (type === "List") {
      props.items = Array.isArray(props.items) ? props.items : [];
      props.title = field(props.title) ?? { i: "title" };
      if (props.caption !== undefined) props.caption = field(props.caption);
      if (props.image !== undefined) props.image = field(props.image);
      props.itemKey = typeof props.itemKey === "string" ? props.itemKey : "id";
      if (typeof press === "string") {
        const rowArgs: Record<string, unknown> = {};
        for (const [k, v] of Object.entries((args as Record<string, unknown>) ?? {})) {
          const m = typeof v === "string" ? /^\{(\w+)\}$/.exec(v) : null;
          rowArgs[k] = m ? { i: m[1] } : v;
        }
        props.rowActions = [{ icon: typeof props.icon === "string" ? props.icon : "caret-right", label: "Open", action: press, args: rowArgs }];
      }
      delete props.icon;
    }
    // Checked as the component receives them; a bound value is the state's, so it isn't checked here.
    const def = (componentDefs as Record<string, { props: { safeParse: (v: unknown) => { success: boolean; error?: { issues: { path: PropertyKey[]; message: string }[] } } } }>)[type];
    const checked = type === "List" ? { ...props, items: [], title: { i: "x" }, caption: undefined, image: undefined } : props;
    const result = def?.props.safeParse(checked);
    if (result && !result.success) {
      const issue = result.error?.issues[0];
      problems.push(`${path} (${type}): ${issue ? `${issue.path.join(".") || "props"} ${issue.message}` : "bad props"}`);
      return null;
    }
    if (typeof bind === "string" && (type === "Wheel" || type === "Toggle")) props.value = { $bindState: `/state/${bind}` };
    const key = `${prefix}-${n}`;
    const kids: string[] = [];
    if (CONTAINERS.has(type) && Array.isArray(children)) {
      children.forEach((c, i) => {
        const k = visit(c, `${path}.children[${i}]`);
        if (k) kids.push(k);
      });
    }
    const el: UIElement = { type, props, children: kids } as UIElement;
    if (typeof press === "string" && type !== "List") {
      (el as UIElement & { on?: unknown }).on = { press: { action: "run", params: { name: press, args: (args as Record<string, unknown>) ?? {} } } };
    }
    elements[key] = el;
    return key;
  };
  if (tree === null || tree === undefined) return { spec: null, problems };
  const root = visit(tree, "view");
  return { spec: root ? ({ root, elements } as Spec) : null, problems };
}

export function LogicView({ props }: Ctx<"LogicView">) {
  const { registry } = useRegistryRef();
  const tree = useStateValue<unknown>(`/logic/${props.value}`);
  const { spec, problems } = useMemo(() => toSpec(tree, `lv-${props.value}`), [tree, props.value]);
  const runtime = useRuntime();
  const report = problems.join("; ");
  useEffect(() => {
    if (report) runtime.reportError(new EneeError("E_LOGIC", `The view \`${props.value}\`: ${report}`));
  }, [report, runtime, props.value]);
  if (!spec) return null;
  return <Renderer spec={spec} registry={registry} />;
}

/** The registry, read lazily: it lists this component, so importing it here would be circular. */
function useRegistryRef(): { registry: Parameters<typeof Renderer>[0]["registry"] } {
  return { registry: registryRef.current as Parameters<typeof Renderer>[0]["registry"] };
}

export const registryRef: { current: unknown } = { current: null };
