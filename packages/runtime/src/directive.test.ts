// Verifies that json-render 0.21.0's custom-directive mechanism carries our `$x` Value IR end to end:
// `resolveElementProps` (what the React renderer calls) → findDirective → xDirective.resolve → evalValue.

import { createDirectiveRegistry, resolveActionParam, resolveElementProps, resolvePropValue } from "@json-render/core";
import { describe, expect, it } from "vitest";

import { setDirectiveFormat, ValueSchema, xDirective } from "./directive";

setDirectiveFormat({ locale: "en-US", timeZone: "UTC" });

const stateModel = {
  q: { today: [{ total: 120, n: 6 }] },
  settings: { daily_goal: 100 },
  state: { draft: { reps: 15 } },
};
const ctx = { stateModel, directives: createDirectiveRegistry([xDirective]) };

describe("$x directive", () => {
  it("resolves the pushup Stat caption exactly like the compiled fixture uses it", () => {
    const props = {
      label: "Today",
      value: { $state: "/q/today/0/total" },
      caption: { $x: { t: [{ p: "/q/today/0/n" }, " sets · goal ", { p: "/settings/daily_goal" }] } },
    };
    expect(resolveElementProps(props, ctx)).toEqual({ label: "Today", value: 120, caption: "6 sets · goal 100" });
  });

  it("applies filters", () => {
    expect(resolvePropValue({ $x: { p: "/q/today/0/total", f: "number", a: 1 } }, ctx)).toBe("120.0");
    expect(resolvePropValue({ $x: { p: "/nope", f: "default", a: "–" } }, ctx)).toBe("–");
  });

  it("sees the repeat item for {i} values", () => {
    expect(resolvePropValue({ $x: { t: [{ i: "reps" }, " reps"] } }, { ...ctx, repeatItem: { reps: 20 } })).toBe("20 reps");
  });

  it("works inside action params (what emit() resolves before calling run)", () => {
    const args = { $x: { t: ["+", { p: "/state/draft/reps" }] } };
    expect(resolveActionParam(args, ctx)).toBe("+15");
    expect(resolveActionParam({ reps: { $x: { p: "/state/draft/reps" } } }, ctx)).toEqual({ reps: 15 });
  });

  it("leaves bare Value IR objects (List row templates) untouched", () => {
    const title = { t: [{ i: "reps" }, " reps"] };
    expect(resolvePropValue(title, ctx)).toEqual(title);
  });

  it("is ignored when the directive is not registered (json-render then treats it as data)", () => {
    expect(resolvePropValue({ $x: { p: "/settings/daily_goal" } }, { stateModel })).toEqual({
      $x: { p: "/settings/daily_goal" },
    });
  });

  it("documents the Value IR with a Zod schema", () => {
    expect(ValueSchema.safeParse({ t: ["a", { p: "/x", f: "time" }, { i: "y" }] }).success).toBe(true);
    expect(ValueSchema.safeParse({ p: "/x", f: "nope" }).success).toBe(false);
    expect(ValueSchema.safeParse({ p: "/x", extra: 1 }).success).toBe(false);
  });
});
