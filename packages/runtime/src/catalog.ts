// The v0 core catalog (contract §4) as a json-render catalog. The Zod schemas describe the *resolved* props a
// component receives (after $state/$x resolution). No free-form `className`/style props: token enums only
// (design-systems.md §5).
//
// Validation caveat (json-render 0.21.0, #378/#222): `catalog.validate()` does not check per-component props once a
// catalog has ≥2 components and strips `on`; never render its `.data`. The compiler validates specs; the runtime
// only runs the structural `validateSpec` (see spec.ts).

import { defineCatalog } from "@json-render/core";
import { schema } from "@json-render/react/schema";
import { z } from "zod";

import { ValueSchema } from "./directive";

const Gap = z.enum(["none", "sm", "md", "lg"]);
const FilterName = z.enum([
  "time",
  "date",
  "datetime",
  "relative",
  "duration",
  "seconds",
  "number",
  "percent",
  "bytes",
  "round",
  "upper",
  "lower",
  "plural",
  "default",
]);
const RowData = z.array(z.record(z.string(), z.unknown()));

export const RowActionSchema = z.object({
  icon: z.string(),
  label: z.string(),
  action: z.string(),
  args: z.record(z.string(), z.unknown()).optional(),
});

/** Each component's props and docs: the catalog's, and what a logic script's views are checked against (ADR-0041). */
export const componentDefs = {
    Stack: {
      props: z.object({ gap: Gap.optional() }),
      slots: ["default"],
      description: "Vertical stack of children.",
      example: { gap: "md" },
    },
    Row: {
      props: z.object({
        gap: Gap.optional(),
        align: z.enum(["start", "center", "end", "between"]).optional(),
        wrap: z.boolean().optional(),
      }),
      slots: ["default"],
      description: "Horizontal row of children. Buttons in a start-aligned row share the width equally.",
      example: { gap: "sm", wrap: true },
    },
    Card: {
      props: z.object({ title: z.string().optional() }),
      slots: ["default"],
      description: "Raised surface; children are stacked.",
      example: { title: "Log a set" },
    },
    Heading: {
      props: z.object({ text: z.string(), level: z.union([z.literal(1), z.literal(2), z.literal(3)]).optional() }),
      description: "Section heading.",
      example: { text: "Today", level: 2 },
    },
    Text: {
      props: z.object({ text: z.string(), tone: z.enum(["default", "muted", "accent", "danger"]).optional() }),
      description: "Body text.",
      example: { text: "Keep going", tone: "muted" },
    },
    Stat: {
      props: z.object({
        label: z.string(),
        value: z.union([z.number(), z.string()]).nullable(),
        caption: z.string().optional(),
        format: FilterName.optional(),
      }),
      description: "Big number with a label and an optional caption; `format` is a Value IR filter name.",
      example: { label: "Today", value: 120, caption: "6 sets" },
    },
    Timer: {
      props: z.object({
        label: z.string(),
        until: z.union([z.string(), z.number()]).nullable().optional(),
        since: z.union([z.string(), z.number()]).nullable().optional(),
        caption: z.string().optional(),
        done: z.string().optional(),
      }),
      description:
        "A stat that ticks every second: a countdown to `until` (showing `done` once it's reached) or a stopwatch from `since`.",
      example: { label: "Focus", until: "2026-10-06T10:25:00.000Z", caption: "left", done: "Time's up" },
    },
    Gauge: {
      props: z.object({
        label: z.string(),
        value: z.union([z.number(), z.string()]).nullable(),
        max: z.union([z.number(), z.string()]).nullable().optional(),
        unit: z.string().optional(),
        caption: z.string().optional(),
      }),
      description: "A half-circle meter from 0 to `max` (100 by default), with the value and its `unit` in the middle.",
      example: { label: "CPU", value: 37, unit: "%" },
    },
    Progress: {
      props: z.object({ value: z.number().nullable(), max: z.number().nullable(), label: z.string().optional() }),
      description: "Progress bar from 0 to max.",
      example: { value: 60, max: 100 },
    },
    Button: {
      props: z.object({
        label: z.string(),
        variant: z.enum(["primary", "secondary", "ghost", "danger"]).optional(),
        icon: z.string().optional(),
      }),
      events: ["press"],
      description: 'Button. Fire actions with on.press: {"action":"run","params":{"name":"<action>","args":{…}}}.',
      example: { label: "Log set", variant: "primary" },
    },
    Input: {
      props: z.object({
        label: z.string(),
        type: z.enum(["text", "number", "time", "date", "select"]).optional(),
        value: z.union([z.string(), z.number()]).nullable().optional(),
        placeholder: z.string().optional(),
        options: z.array(z.string()).optional(),
        min: z.number().optional(),
        max: z.number().optional(),
        step: z.number().positive().optional(),
      }),
      description:
        'Text/number/time/date field, or a choice among `options` (type "select"); bind with value: {"$bindState":"/state/…"}. A number is turned with the wheel (between `min` and `max`, by `step`), and a choice among more than four options is a reel.',
      example: { label: "Reps", type: "number" },
    },
    Wheel: {
      props: z.object({
        label: z.string(),
        value: z.union([z.string(), z.number()]).nullable().optional(),
        min: z.number().optional(),
        max: z.number().optional(),
        step: z.number().positive().optional(),
        options: z.array(z.string()).optional(),
        unit: z.string().optional(),
        wrap: z.boolean().optional(),
      }),
      description:
        'A value turned with the wheel: a number from `min` to `max` by `step` (or one of `options`), shown big with its `unit`. A click takes the wheel, each detent turns it, a click keeps it and a double click puts it back. Bind with value: {"$bindState":"/state/…"}.',
      example: { label: "Glasses", min: 0, max: 20, unit: "glasses" },
    },
    Game: {
      props: z.object({
        label: z.string(),
        script: z.string(),
        actions: z.array(z.string()).optional(),
        height: z.number().optional(),
      }),
      description:
        "A small game in JavaScript, sandboxed (no network, no DOM): it draws on a canvas and gets the wheel, the click and the button while it's played (the `r1` API).",
      example: { label: "Catch", script: "r1.loop((dt) => {});", actions: ["save_score"] },
    },
    LogicView: {
      props: z.object({ value: z.string() }),
      description: "A view the app's logic builds (ADR-0041): the component tree its script publishes at logic.<value>.",
      example: { value: "board" },
    },
    Toggle: {
      props: z.object({ label: z.string(), value: z.boolean().nullable().optional() }),
      description: 'On/off switch; bind with value: {"$bindState":"/state/…"}.',
      example: { label: "Remind me" },
    },
    CalendarHeatmap: {
      props: z.object({
        data: RowData.nullable(),
        date: z.string(),
        value: z.string(),
        weeks: z.number().int().min(1).max(53).optional(),
        title: z.string().optional(),
      }),
      description: "Days as squares, a column per week (Monday first), shaded by the `value` of each `date`.",
      example: { date: "day", value: "n", weeks: 12, title: "Last 12 weeks" },
    },
    DonutChart: {
      props: z.object({ data: RowData.nullable(), x: z.string(), y: z.string(), title: z.string().optional() }),
      description: "Shares of a whole: the five biggest rows by `y`, named by `x`, and the rest as Other, with a legend.",
      example: { x: "category", y: "total", title: "Spending" },
    },
    BarChart: {
      props: z.object({
        data: RowData.nullable(),
        x: z.string(),
        y: z.string(),
        xDomain: z.tuple([z.number(), z.number()]).optional(),
        title: z.string().optional(),
      }),
      description: "Bar chart of rows; integer x buckets in xDomain are zero-filled.",
      example: { x: "hour", y: "reps", xDomain: [0, 23], title: "Reps per hour" },
    },
    LineChart: {
      props: z.object({
        data: RowData.nullable(),
        x: z.string(),
        y: z.string(),
        xDomain: z.tuple([z.number(), z.number()]).optional(),
        title: z.string().optional(),
      }),
      description: "Line chart of rows; integer x buckets in xDomain are zero-filled.",
      example: { x: "day", y: "total", title: "Last 7 days" },
    },
    List: {
      props: z.object({
        items: RowData.nullable(),
        itemKey: z.string(),
        title: ValueSchema,
        caption: ValueSchema.optional(),
        image: ValueSchema.optional(),
        rowActions: z.array(RowActionSchema).optional(),
        empty: z.string().optional(),
      }),
      description:
        "Rows from an array; title/caption/image are Value IR row templates ({i} = row field). image is a thumbnail: one of the app's own files (files/<sha256>.thumb.png) or a data:image URL.",
      example: { itemKey: "id", title: { t: [{ i: "reps" }, " reps"] }, empty: "No sets yet" },
    },
    EmptyState: {
      props: z.object({ text: z.string(), icon: z.string().optional() }),
      description: "Placeholder for empty content.",
      example: { text: "Nothing here yet", icon: "inbox" },
    },
};

export const catalog = defineCatalog(schema, {
  components: componentDefs,
  actions: {
    run: {
      params: z.object({ name: z.string(), args: z.record(z.string(), z.unknown()).optional() }),
      description: "Run an app action by name. Params: { name: string, args?: object }.",
    },
  },
});

export type EneeCatalog = typeof catalog;
