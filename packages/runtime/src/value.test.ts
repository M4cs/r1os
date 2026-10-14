import { describe, expect, it } from "vitest";

import {
  applyFilter,
  evalValue,
  formatClock,
  formatDuration,
  isValueForm,
  parseInstant,
  readPointer,
  stateScope,
  toText,
  valuePaths,
  type FormatOptions,
} from "./value";

const fmt: FormatOptions = { locale: "en-US", timeZone: "UTC", now: () => Date.UTC(2026, 9, 5, 15, 0, 0) };
const state = {
  q: { today: [{ total: 120, n: 6 }], recent: [{ id: 1, reps: 20, at: "2026-10-05T14:05:09.123Z" }] },
  settings: { daily_goal: 100, remind_every: 3600 },
  state: { draft: { reps: 15 }, name: "max" },
};
const scope = stateScope(state, undefined, fmt);
const norm = (s: unknown) => String(s).replace(/\s/g, " "); // ICU uses U+202F before AM/PM

describe("valuePaths", () => {
  it("finds every path a value reads, nested and in templates", () => {
    const params = { km: { p: "/state/km" }, label: { t: ["to ", { p: "/state/unit", f: "upper" }] }, n: 3 };
    expect([...valuePaths(params)].sort()).toEqual(["/state/km", "/state/unit"]);
    expect(valuePaths("plain").size).toBe(0);
  });
});

describe("readPointer", () => {
  it("reads nested objects and array indices", () => {
    expect(readPointer(state, "/q/today/0/total")).toBe(120);
    expect(readPointer(state, "/q/today/1/total")).toBeUndefined();
    expect(readPointer(state, "/nope/deeper")).toBeUndefined();
    expect(readPointer({ "a/b": { "c~d": 1 } }, "/a~1b/c~0d")).toBe(1);
    expect(readPointer(state, "")).toBe(state);
  });
});

describe("evalValue", () => {
  it("passes literals through", () => {
    expect(evalValue(5, scope)).toBe(5);
    expect(evalValue("x", scope)).toBe("x");
    expect(evalValue(null, scope)).toBeNull();
    expect(evalValue(true, scope)).toBe(true);
  });

  it("reads {p} paths with their type intact", () => {
    expect(evalValue({ p: "/q/today/0/total" }, scope)).toBe(120);
    expect(evalValue({ p: "/state/draft" }, scope)).toEqual({ reps: 15 });
    expect(evalValue({ p: "/missing" }, scope)).toBeUndefined();
  });

  it("concatenates {t} parts, treating null/undefined as empty", () => {
    const caption = { t: [{ p: "/q/today/0/n" }, " sets · goal ", { p: "/settings/daily_goal" }] };
    expect(evalValue(caption, scope)).toBe("6 sets · goal 100");
    expect(evalValue({ t: ["a", { p: "/missing" }, "b", 1, false] }, scope)).toBe("ab1false");
  });

  it("reads {i} row fields from the scope item (field, dotted and pointer forms)", () => {
    const row = { id: 7, reps: 20, meta: { by: "max" } };
    const rowScope = stateScope(state, row, fmt);
    expect(evalValue({ t: [{ i: "reps" }, " reps"] }, rowScope)).toBe("20 reps");
    expect(evalValue({ i: "meta.by" }, rowScope)).toBe("max");
    expect(evalValue({ i: "/meta/by" }, rowScope)).toBe("max");
    expect(evalValue({ i: "id" }, scope)).toBeUndefined(); // no row in scope
  });

  it("applies filters and filter chains", () => {
    expect(evalValue({ p: "/q/today/0/total", f: "number", a: 1 }, scope)).toBe("120.0");
    expect(evalValue({ p: "/state/name", f: ["upper", "default"], a: [null, "-"] }, scope)).toBe("MAX");
    expect(evalValue({ p: "/missing", f: "default", a: "n/a" }, scope)).toBe("n/a");
  });

  it("evaluates plain objects field by field (action args, set maps)", () => {
    expect(evalValue({ id: { i: "id" }, fixed: 3 }, stateScope(state, { id: 9 }))).toEqual({ id: 9, fixed: 3 });
    expect(evalValue([{ p: "/settings/daily_goal" }, "x"], scope)).toEqual([100, "x"]);
  });

  it("accepts $state and nested $x for robustness", () => {
    expect(evalValue({ $state: "/settings/daily_goal" }, scope)).toBe(100);
    expect(evalValue({ $x: { t: ["#", { p: "/q/today/0/n" }] } }, scope)).toBe("#6");
  });

  it("treats objects with extra keys as data, not Value forms", () => {
    expect(isValueForm({ p: "/a" })).toBe(true);
    expect(isValueForm({ p: "/a", q: 1 })).toBe(false);
    expect(evalValue({ p: "/settings/daily_goal", note: "keep" }, scope)).toEqual({
      p: "/settings/daily_goal",
      note: "keep",
    });
  });
});

describe("filters", () => {
  it("are null-safe", () => {
    for (const f of ["time", "date", "datetime", "relative", "duration", "seconds", "number", "percent", "bytes", "round", "upper", "lower", "plural"]) {
      expect(applyFilter(null, f, undefined, fmt)).toBeNull();
      expect(applyFilter(undefined, f, undefined, fmt)).toBeNull();
    }
    expect(applyFilter(null, "default", "x")).toBe("x");
    expect(applyFilter("", "default", "x")).toBe("x");
    expect(applyFilter(0, "default", "x")).toBe(0);
  });

  it("time/date/datetime format stored UTC instants in the given zone", () => {
    const at = "2026-10-05T14:05:09.123Z";
    expect(norm(applyFilter(at, "time", undefined, fmt))).toBe("2:05 PM");
    expect(applyFilter(at, "date", undefined, fmt)).toBe("Oct 5");
    expect(applyFilter("2025-01-02T00:00:00.000Z", "date", undefined, fmt)).toBe("Jan 2, 2025");
    expect(norm(applyFilter(at, "datetime", undefined, fmt))).toBe("Oct 5, 2:05 PM");
    // A time of day (setting type `time`) is not zone-shifted.
    expect(norm(applyFilter("09:30", "time", undefined, { ...fmt, timeZone: "Asia/Tokyo" }))).toBe("9:30 AM");
    // Unparseable input is shown as-is.
    expect(applyFilter("soon", "time", undefined, fmt)).toBe("soon");
  });

  it("relative uses the injected clock", () => {
    expect(applyFilter("2026-10-05T14:59:40.000Z", "relative", undefined, fmt)).toBe("just now");
    expect(applyFilter("2026-10-05T14:55:00.000Z", "relative", undefined, fmt)).toBe("5 min. ago");
    expect(applyFilter("2026-10-05T12:00:00.000Z", "relative", undefined, fmt)).toBe("3 hr. ago");
    expect(applyFilter("2026-10-04T15:00:00.000Z", "relative", undefined, fmt)).toBe("yesterday");
  });

  it("duration and seconds", () => {
    expect(formatDuration(0)).toBe("0s");
    expect(formatDuration(45)).toBe("45s");
    expect(formatDuration(90)).toBe("1m 30s");
    expect(formatDuration(3600)).toBe("1h");
    expect(formatDuration(5400)).toBe("1h 30m");
    expect(formatDuration(2 * 86400 + 3 * 3600 + 60)).toBe("2d 3h");
    expect(applyFilter("3600", "duration")).toBe("1h");
    expect(formatClock(245)).toBe("4:05");
    expect(formatClock(3723)).toBe("1:02:03");
    expect(applyFilter(1500, "seconds")).toBe("25:00");
  });

  it("numbers, percent, bytes, round", () => {
    expect(applyFilter(1234.5, "number", undefined, fmt)).toBe("1,234.5");
    expect(applyFilter(1234.5, "number", 0, fmt)).toBe("1,235");
    expect(applyFilter("2", "number", "2", fmt)).toBe("2.00");
    expect(applyFilter(0.425, "percent", undefined, fmt)).toBe("43%");
    expect(applyFilter(0.425, "percent", 1, fmt)).toBe("42.5%");
    expect(applyFilter(512, "bytes", undefined, fmt)).toBe("512 B");
    expect(applyFilter(1536, "bytes", undefined, fmt)).toBe("1.5 KB");
    expect(applyFilter(5 * 1024 ** 3, "bytes", undefined, fmt)).toBe("5 GB");
    expect(applyFilter(3.14159, "round", 2)).toBe(3.14);
    expect(applyFilter(2.5, "round")).toBe(3);
    expect(applyFilter("abc", "number")).toBe("abc");
  });

  it("text filters and plural", () => {
    expect(applyFilter("Hello", "upper")).toBe("HELLO");
    expect(applyFilter("Hello", "lower")).toBe("hello");
    expect(applyFilter(1, "plural", "set", fmt)).toBe("1 set");
    expect(applyFilter(3, "plural", "set", fmt)).toBe("3 sets");
    expect(applyFilter(2, "plural", "entry", fmt)).toBe("2 entries");
    expect(applyFilter(2, "plural", "box", fmt)).toBe("2 boxes");
    expect(applyFilter(2, "plural", "person|people", fmt)).toBe("2 people");
  });

  it("unknown filters pass values through", () => {
    expect(applyFilter(5, "nope")).toBe(5);
  });
});

describe("parseInstant / toText", () => {
  it("parses the stored formats", () => {
    expect(parseInstant("2026-10-05T14:05:09.123Z")?.toISOString()).toBe("2026-10-05T14:05:09.123Z");
    expect(parseInstant("2026-10-05 14:05:09")?.toISOString()).toBe("2026-10-05T14:05:09.000Z");
    expect(parseInstant(1_759_673_109_123)?.getTime()).toBe(1_759_673_109_123);
    expect(parseInstant(1_759_673_109)?.getTime()).toBe(1_759_673_109_000);
    expect(parseInstant("2026-10-05")?.getHours()).toBe(0); // local midnight
    expect(parseInstant("garbage")).toBeNull();
  });

  it("stringifies for concatenation", () => {
    expect(toText(undefined)).toBe("");
    expect(toText(NaN)).toBe("");
    expect(toText({ a: 1 })).toBe('{"a":1}');
  });
});
