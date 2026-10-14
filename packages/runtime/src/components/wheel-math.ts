// The arithmetic of a value turned with the R1's wheel (the `Wheel` component, number fields): steps, the ends,
// wrapping round, a list of options, and the quick spin the native pickers have (r1os ADR-0038).

export interface WheelSpec {
  min?: number | undefined;
  max?: number | undefined;
  step?: number | undefined;
  options?: readonly string[] | undefined;
  wrap?: boolean | undefined;
}

export type WheelValue = string | number | null;

/** Ticks closer than this are a quick spin (the alarm picker's QUICK_MS). */
export const QUICK_MS = 70;
/** A quick spin over a long range moves this many steps a tick. */
export const QUICK_STRIDE = 5;
/** A range needs this many steps before a quick spin speeds up. */
const QUICK_RANGE = 30;

function stepOf(spec: WheelSpec): number {
  return spec.step && spec.step > 0 ? spec.step : 1;
}

/** Decimal places a step implies (0.25 → 2), so 0.1 + 0.2 shows as 0.3. */
export function decimals(step: number | undefined): number {
  if (!step || Number.isInteger(step)) return 0;
  const s = String(step);
  const e = s.indexOf("e-");
  if (e >= 0) return Number(s.slice(e + 2));
  return s.length - s.indexOf(".") - 1;
}

function round(n: number, digits: number): number {
  const f = 10 ** digits;
  return Math.round(n * f) / f;
}

/** How many values the wheel can land on (Infinity when it has no ends). */
export function span(spec: WheelSpec): number {
  if (spec.options) return spec.options.length;
  if (spec.min === undefined || spec.max === undefined) return Infinity;
  return Math.floor((spec.max - spec.min) / stepOf(spec) + 1e-9) + 1;
}

/** How many steps a tick moves, given the time since the last one. */
export function stride(spec: WheelSpec, sinceLastMs: number): number {
  return sinceLastMs < QUICK_MS && span(spec) >= QUICK_RANGE ? QUICK_STRIDE : 1;
}

/** The value `delta` ticks (×`by` steps each) away from `value`, kept between the ends or wrapped round. */
export function turn(value: WheelValue, delta: number, spec: WheelSpec, by = 1): WheelValue {
  const moves = delta * by;
  if (spec.options) {
    const n = spec.options.length;
    if (n === 0) return null;
    const at = spec.options.indexOf(String(value));
    // Nothing chosen yet: the first tick lands on the first option (down) or the last (up).
    if (at < 0) return spec.options[delta > 0 ? 0 : n - 1] ?? null;
    const next = spec.wrap ? (((at + moves) % n) + n) % n : Math.max(0, Math.min(n - 1, at + moves));
    return spec.options[next] ?? null;
  }
  const step = stepOf(spec);
  const digits = decimals(step);
  const min = spec.min;
  const max = spec.max;
  const base = typeof value === "number" && Number.isFinite(value) ? value : min !== undefined ? min : 0;
  if (min === undefined || max === undefined) {
    const next = round(base + moves * step, digits);
    if (min !== undefined && next < min) return min;
    if (max !== undefined && next > max) return max;
    return next;
  }
  const count = span(spec);
  const at = Math.round((base - min) / step);
  const index = spec.wrap ? (((at + moves) % count) + count) % count : Math.max(0, Math.min(count - 1, at + moves));
  return round(min + index * step, digits);
}

/** The values either side of `value`, for the reel (null past an end). */
export function neighbours(value: WheelValue, spec: WheelSpec): [WheelValue, WheelValue] {
  const prev = turn(value, -1, spec);
  const next = turn(value, 1, spec);
  return [prev === value ? null : prev, next === value ? null : next];
}

/** A number as the wheel shows it: the step's decimals, nothing more. */
export function show(value: WheelValue, spec: WheelSpec): string {
  if (value === null || value === undefined || value === "") return "–";
  if (typeof value === "number") return round(value, decimals(spec.step)).toFixed(decimals(spec.step));
  return String(value);
}
