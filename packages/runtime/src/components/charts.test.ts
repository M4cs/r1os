import { describe, expect, it } from "vitest";

import { chartSeries, donutSlices, heatmapDays, niceMax, tickIndices, yRange } from "./charts";

describe("yRange", () => {
  it("fits a line of readings far from zero, and keeps zero for bars and spread-out lines", () => {
    const btc = [86256.08, 86301.5, 86418.05, 86455.91];
    const [lo, hi] = yRange("line", btc);
    expect(lo).toBeGreaterThan(86000);
    expect(hi).toBeLessThan(86600);
    expect(lo).toBeLessThan(86256.08);
    expect(hi).toBeGreaterThan(86455.91);
    expect(yRange("bar", btc)).toEqual([0, 100000]);
    expect(yRange("line", [2, 10, 30])).toEqual([0, 30]);
    expect(yRange("line", [0, 5])).toEqual([0, 5]);
    const [flatLo, flatHi] = yRange("line", [121.61, 121.61]);
    expect(flatLo < 121.61 && flatHi > 121.61).toBe(true);
  });
});

describe("chartSeries", () => {
  it("zero-fills an integer xDomain (24 hourly buckets) and sums duplicates", () => {
    const points = chartSeries(
      [
        { hour: 9, reps: 25 },
        { hour: 14, reps: 30 },
        { hour: 14, reps: 5 },
        { hour: 30, reps: 99 }, // outside the domain
        { hour: "x", reps: 1 },
      ],
      "hour",
      "reps",
      [0, 23],
    );
    expect(points).toHaveLength(24);
    expect(points[0]).toEqual({ label: "0", y: 0 });
    expect(points[9]).toEqual({ label: "9", y: 25 });
    expect(points[14]).toEqual({ label: "14", y: 35 });
    expect(points.reduce((s, p) => s + p.y, 0)).toBe(60);
  });

  it("reads nested fields, as JSON services nest them (text numbers included)", () => {
    const history = [
      { t: "2026-10-06T12:00:00.000Z", data: { amount: "62000.5", base: "BTC" } },
      { t: "2026-10-06T12:01:00.000Z", data: { amount: "62010.25", base: "BTC" } },
      { t: "2026-10-06T12:02:00.000Z", data: null },
    ];
    expect(chartSeries(history, "t", "data.amount").map((p) => p.y)).toEqual([62000.5, 62010.25, 0]);
    expect(chartSeries([{ q: [{ p: 3 }] }], "x", "q.0.p")[0]?.y).toBe(3);
  });

  it("labels timestamps in local time, as fine as the span needs", () => {
    // A live service's history: seconds.
    const live = chartSeries(
      [
        { t: "2026-10-06T12:35:33.792Z", usage: 30.2 },
        { t: "2026-10-06T12:35:34.792Z", usage: 28 },
      ],
      "t",
      "usage",
    );
    expect(live[0]?.y).toBe(30.2);
    expect(live[0]?.label).toMatch(/^\d{1,2}:\d{2}:\d{2}/);
    expect(live[0]?.label).not.toBe(live[1]?.label);
    // A day of entries: the time of day, without seconds.
    const day = chartSeries([{ at: "2026-10-06T08:00:00.000Z" }, { at: "2026-10-06T16:00:00.000Z" }], "at", "n");
    expect(day[0]?.label).toMatch(/^\d{1,2}:\d{2}(?!:)/);
    // Weeks: dates.
    const weeks = chartSeries([{ at: "2026-09-01T08:00:00.000Z" }, { at: "2026-10-06T08:00:00.000Z" }], "at", "n");
    expect(weeks[0]?.label).not.toMatch(/:/);
    expect(weeks.every((p) => !p.label.includes("T"))).toBe(true);
  });

  it("keeps row order without a domain and tolerates missing data", () => {
    expect(chartSeries([{ d: "Mon", v: 2 }, { d: "Tue", v: "3" }], "d", "v")).toEqual([
      { label: "Mon", y: 2 },
      { label: "Tue", y: 3 },
    ]);
    expect(chartSeries(null, "x", "y", [1, 3])).toEqual([
      { label: "1", y: 0 },
      { label: "2", y: 0 },
      { label: "3", y: 0 },
    ]);
  });
});

describe("axis helpers", () => {
  it("niceMax rounds up to a nice step", () => {
    expect(niceMax(0)).toBe(1);
    expect(niceMax(7)).toBe(8);
    expect(niceMax(30)).toBe(30);
    expect(niceMax(31)).toBe(40);
    expect(niceMax(120)).toBe(120);
    expect(niceMax(1072)).toBe(1200);
    expect(niceMax(0.35)).toBe(0.4);
  });

  it("tickIndices labels about five slots", () => {
    expect(tickIndices(24)).toEqual([0, 6, 12, 18]);
    expect(tickIndices(5)).toEqual([0, 1, 2, 3, 4]);
    expect(tickIndices(0)).toEqual([]);
    // Wide labels (times with seconds) leave room for fewer.
    expect(tickIndices(120, 4).length).toBeLessThanOrEqual(4);
    expect(tickIndices(5, 3).length).toBeLessThanOrEqual(3);
  });
});

describe("donutSlices", () => {
  it("keeps the five biggest positive rows and sums the rest as Other", () => {
    const rows = [
      { category: "food", total: 120 },
      { category: "rent", total: 900 },
      { category: "fun", total: 0 },
      { category: "travel", total: 60 },
      { category: "books", total: 25 },
      { category: "gifts", total: 30 },
      { category: "misc", total: 10 },
      { category: "refund", total: -40 },
      { category: "coffee", total: "15" },
    ];
    const slices = donutSlices(rows, "category", "total");
    expect(slices.map((s) => s.label)).toEqual(["rent", "food", "travel", "gifts", "books", "Other"]);
    expect(slices.at(-1)).toMatchObject({ value: 25, other: true });
    expect(slices.reduce((sum, s) => sum + s.share, 0)).toBeCloseTo(1);
    expect(slices[0]!.share).toBeCloseTo(900 / 1160);
  });

  it("is empty without positive values, and names a missing label", () => {
    expect(donutSlices([{ k: "a", v: 0 }], "k", "v")).toEqual([]);
    expect(donutSlices(null, "k", "v")).toEqual([]);
    expect(donutSlices([{ v: 3 }], "k", "v")).toEqual([{ label: "–", value: 3, share: 1 }]);
  });
});

describe("heatmapDays", () => {
  // Wednesday 7 October 2026, local time.
  const today = new Date(2026, 9, 7, 15, 30);

  it("lays out weeks Monday first, ending with this one, and adds up values per day", () => {
    const rows = [
      { day: "2026-10-05", n: 2 },
      { day: "2026-10-05", n: 1 },
      { day: "2026-10-07", n: 6 },
      { day: "2026-09-28", n: 1 },
      { day: "2025-01-01", n: 9 },
      { day: "not a date", n: 4 },
    ];
    const weeks = heatmapDays(rows, "day", "n", 2, today);
    expect(weeks).toHaveLength(2);
    expect(weeks.map((w) => w[0]!.day)).toEqual(["2026-09-28", "2026-10-05"]);
    const thisWeek = weeks[1]!;
    expect(thisWeek.map((d) => d.value)).toEqual([3, 0, 6, 0, 0, 0, 0]);
    expect(thisWeek.map((d) => d.future)).toEqual([false, false, false, true, true, true, true]);
    expect(thisWeek.map((d) => d.level)).toEqual([2, 0, 4, 0, 0, 0, 0]);
    expect(weeks[0]![0]).toMatchObject({ value: 1, level: 1 });
  });

  it("is all empty without data", () => {
    const weeks = heatmapDays(null, "day", "n", 12, today);
    expect(weeks).toHaveLength(12);
    expect(weeks.flat().every((d) => d.level === 0)).toBe(true);
  });
});
