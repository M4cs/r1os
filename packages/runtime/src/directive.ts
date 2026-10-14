// `$x`: the custom json-render directive that carries a Value IR (contract §3) inside element props.
//
// Implemented with `defineDirective` from @json-render/core 0.21.0 (re-exported by @json-render/directives' types).
// The renderer resolves props with `resolvePropValue`, which checks built-ins ($state, $cond, …) first and then
// looks up custom directives by key, so `{"$x": …}` reaches `resolve` below with the live state model and the
// repeat scope. Note: json-render does NOT validate the directive object against `schema` at runtime; the schema
// only documents the directive in `catalog.prompt({ directives })`. `evalValue` is therefore total (never throws).

import { defineDirective, type PropResolutionContext } from "@json-render/core";
import { z } from "zod";

import type { FilterName, Value } from "./types";
import { evalValue, type FormatOptions, readPointer } from "./value";

const FILTER_NAMES = [
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
] as const satisfies readonly FilterName[];

const FilterSchema = z.union([z.enum(FILTER_NAMES), z.array(z.enum(FILTER_NAMES))]);
const LiteralSchema = z.union([z.string(), z.number(), z.boolean(), z.null()]);

/** Zod schema of the Value IR (used for prompt docs and in tests). */
export const ValueSchema: z.ZodType<Value> = z.lazy(() =>
  z.union([
    LiteralSchema,
    z.strictObject({ p: z.string(), f: FilterSchema.optional(), a: z.unknown().optional() }),
    z.strictObject({ i: z.string(), f: FilterSchema.optional(), a: z.unknown().optional() }),
    z.strictObject({ t: z.array(ValueSchema) }),
  ]),
) as z.ZodType<Value>;

/** Formatting defaults for `$x` (user locale/zone). Swappable for tests. */
let directiveFormat: FormatOptions = {};
export function setDirectiveFormat(format: FormatOptions): void {
  directiveFormat = format;
}

export const xDirective = defineDirective({
  name: "$x",
  description:
    'Formatted or concatenated value. {"$x":{"p":"/q/today/0/total","f":"number"}} reads a state path and applies a ' +
    'filter; {"$x":{"t":["Hello ",{"p":"/state/name"}]}} concatenates. Filters: time, date, datetime, relative, ' +
    "duration, seconds, number(a=digits), percent, bytes, round(a=digits), upper, lower, plural(a=word), default(a=fallback).",
  schema: z.object({ $x: ValueSchema }),
  resolve(value, ctx: PropResolutionContext) {
    return evalValue(value.$x, {
      read: (path) => readPointer(ctx.stateModel, path),
      item: ctx.repeatItem,
      format: directiveFormat,
    });
  },
});

/** Every custom directive the runtime installs. Pass the same array to `catalog.prompt({ directives })`. */
export const DIRECTIVES = [xDirective];
