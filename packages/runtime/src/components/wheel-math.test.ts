import { describe, expect, it } from "vitest";

import { decimals, neighbours, QUICK_MS, QUICK_STRIDE, show, span, stride, turn } from "./wheel-math";

describe("wheel math", () => {
  it("steps numbers between the ends", () => {
    const spec = { min: 0, max: 10, step: 1 };
    expect(turn(3, 1, spec)).toBe(4);
    expect(turn(3, -1, spec)).toBe(2);
    expect(turn(10, 1, spec)).toBe(10);
    expect(turn(0, -1, spec)).toBe(0);
    // Nothing set yet: it starts from the bottom end.
    expect(turn(null, 1, spec)).toBe(1);
  });

  it("wraps round when asked", () => {
    const minutes = { min: 0, max: 55, step: 5, wrap: true };
    expect(turn(55, 1, minutes)).toBe(0);
    expect(turn(0, -1, minutes)).toBe(55);
    expect(span(minutes)).toBe(12);
  });

  it("keeps the step's decimals", () => {
    expect(decimals(0.1)).toBe(1);
    expect(decimals(0.25)).toBe(2);
    expect(decimals(1)).toBe(0);
    const spec = { min: 0, max: 1, step: 0.1 };
    expect(turn(0.2, 1, spec)).toBe(0.3);
    expect(show(0.3, spec)).toBe("0.3");
    expect(show(1, { step: 0.5 })).toBe("1.0");
    expect(show(null, spec)).toBe("–");
  });

  it("turns through options", () => {
    const spec = { options: ["small", "medium", "large"] };
    expect(turn("small", 1, spec)).toBe("medium");
    expect(turn("large", 1, spec)).toBe("large");
    expect(turn("large", 1, { ...spec, wrap: true })).toBe("small");
    expect(turn(null, 1, spec)).toBe("small");
    expect(turn(null, -1, spec)).toBe("large");
    expect(neighbours("small", spec)).toEqual([null, "medium"]);
  });

  it("spins quickly over long ranges only", () => {
    const long = { min: 0, max: 200 };
    const short = { min: 0, max: 10 };
    expect(stride(long, QUICK_MS - 10)).toBe(QUICK_STRIDE);
    expect(stride(long, QUICK_MS + 10)).toBe(1);
    expect(stride(short, 5)).toBe(1);
    expect(turn(20, 1, long, stride(long, 5))).toBe(25);
    expect(turn(198, 1, long, QUICK_STRIDE)).toBe(200);
  });

  it("has no ends without both of them", () => {
    expect(turn(5, 3, {})).toBe(8);
    expect(turn(1, -3, { min: 0 })).toBe(0);
    expect(span({ min: 0 })).toBe(Infinity);
  });
});
