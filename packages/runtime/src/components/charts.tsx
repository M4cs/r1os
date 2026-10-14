// In-house SVG charts (contract §4: BarChart / LineChart). Integer x values inside `xDomain` are bucketed and
// missing buckets are zero-filled, so "reps per hour" always shows 24 slots. Geometry is computed in CSS pixels
// from the measured width (crisp at any panel size, no non-uniform viewBox scaling).

import { type CSSProperties, type PointerEvent, useId, useLayoutEffect, useMemo, useRef, useState } from "react";

import { useRuntime } from "../context";
import { eneeId } from "../spec";
import { formatDate, formatNumber, formatTime, parseInstant, toText } from "../value";
import type { Ctx } from "./types";

export interface ChartPoint {
  label: string;
  y: number;
}

const MAX_BUCKETS = 1000;

const INSTANT = /^\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}/;
const MINUTE_MS = 60_000;
const DAY_MS = 86_400_000;
const WITH_SECONDS = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit", second: "2-digit" });

/**
 * Labels for an x column of timestamps (what the host stores; a service sample's `t`), in local time, as fine as the
 * series needs: seconds under 15 minutes (a live service's history), the time of day under a day, dates beyond.
 * `null` unless every value is a timestamp.
 */
function timeLabels(values: readonly unknown[]): string[] | null {
  if (values.length === 0 || !values.every((v) => typeof v === "string" && INSTANT.test(v))) return null;
  const instants = values.map((v) => parseInstant(v));
  const times = instants.map((d) => d?.getTime() ?? NaN).filter(Number.isFinite);
  const span = times.length > 0 ? Math.max(...times) - Math.min(...times) : 0;
  return values.map((v, i) => {
    const d = instants[i];
    if (!d) return toText(v);
    if (span < 15 * MINUTE_MS) return WITH_SECONDS.format(d);
    return toText(span < DAY_MS ? formatTime(v) : formatDate(v));
  });
}

function num(v: unknown): number {
  const n = typeof v === "number" ? v : typeof v === "string" ? Number(v) : NaN;
  return Number.isFinite(n) ? n : 0;
}

/** A row's field; `a.b.0` reaches into nested values (a JSON service's samples, such as Coinbase's `data.amount`). */
export function field(row: Record<string, unknown>, path: string): unknown {
  if (!path.includes(".")) return row[path];
  let v: unknown = row;
  for (const part of path.split(".")) {
    if (v === null || typeof v !== "object") return undefined;
    v = Array.isArray(v) ? v[Number(part)] : (v as Record<string, unknown>)[part];
  }
  return v;
}

/** Rows → points. With an integer `xDomain`, rows are summed into buckets and gaps are zero-filled. */
export function chartSeries(
  data: readonly Record<string, unknown>[] | null | undefined,
  x: string,
  y: string,
  xDomain?: readonly [number, number],
): ChartPoint[] {
  const rows = Array.isArray(data) ? data : [];
  if (xDomain) {
    const lo = Math.ceil(Math.min(xDomain[0], xDomain[1]));
    const hi = Math.floor(Math.max(xDomain[0], xDomain[1]));
    if (Number.isFinite(lo) && Number.isFinite(hi) && hi - lo < MAX_BUCKETS) {
      const sums = new Map<number, number>();
      for (const row of rows) {
        const k = Number(field(row, x));
        if (!Number.isInteger(k) || k < lo || k > hi) continue;
        sums.set(k, (sums.get(k) ?? 0) + num(field(row, y)));
      }
      const out: ChartPoint[] = [];
      for (let k = lo; k <= hi; k++) out.push({ label: String(k), y: sums.get(k) ?? 0 });
      return out;
    }
  }
  const xs = rows.map((row) => field(row, x));
  const times = timeLabels(xs);
  return rows.map((row, i) => ({
    label: times?.[i] ?? (xs[i] === undefined ? String(i) : toText(xs[i])),
    y: num(field(row, y)),
  }));
}

const NICE = [1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8, 10];

/** A "nice" axis maximum ≥ v (1, 1.2, 1.5, 2, 2.5, 3, 4, 5, 6, 8 × 10ⁿ), so the tallest bar fills most of the plot. */
export function niceMax(v: number): number {
  if (!(v > 0)) return 1;
  const exp = 10 ** Math.floor(Math.log10(v));
  for (const m of NICE) if (Math.round(m * exp * 1e9) / 1e9 >= v) return Math.round(m * exp * 1e9) / 1e9;
  return 10 * exp;
}

/** A "nice" step ≥ v (1, 2, 2.5, 5 × 10ⁿ), for a fitted axis. */
function niceStep(v: number): number {
  const exp = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 2.5, 5, 10]) if (m * exp >= v) return m * exp;
  return 10 * exp;
}

/**
 * The y axis: bars, and readings that reach down near zero, start at 0. A line of readings far from zero (a price, a
 * temperature) is fitted to them on round numbers, so its movement shows instead of a flat line at the top.
 */
export function yRange(kind: "bar" | "line", ys: readonly number[]): [number, number] {
  const max = ys.reduce((m, y) => Math.max(m, y), 0);
  const min = ys.reduce((m, y) => Math.min(m, y), Number.POSITIVE_INFINITY);
  const span = max - min;
  if (kind === "bar" || !(min > 0) || span > max * 0.5) return [0, niceMax(max)];
  const step = niceStep(span > 0 ? span / 4 : max * 0.002);
  const lo = Math.max(0, Math.floor((min - span * 0.1) / step) * step);
  const hi = Math.ceil((max + span * 0.1) / step) * step;
  const round = (v: number) => Math.round(v * 1e9) / 1e9;
  return hi > lo ? [round(lo), round(hi)] : [round(lo), round(lo + step)];
}

const TICK_STEPS = [1, 2, 3, 4, 5, 6, 10, 12, 15, 20, 24, 25, 30, 50, 60, 100];

/** Indices to label on the x axis: about five, on "nice" steps, preferring steps that divide n (24 h → 0/6/12/18). */
export function tickIndices(n: number, max = 6): number[] {
  if (n <= 0) return [];
  if (n <= max) return Array.from({ length: n }, (_, i) => i);
  const target = n / Math.max(1, max - 1);
  const candidates = TICK_STEPS.filter((s) => s >= target);
  const step = candidates.find((s) => n % s === 0 && s <= target * 1.6) ?? candidates[0] ?? Math.ceil(target);
  const out: number[] = [];
  for (let i = 0; i < n; i += step) out.push(i);
  return out;
}

function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [width, setWidth] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    setWidth(el.clientWidth);
    const ro = new ResizeObserver((entries) => {
      const w = Math.round(entries[0]?.contentRect.width ?? 0);
      setWidth((prev) => (prev === w ? prev : w));
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [ref, width] as const;
}

const PLOT_H = 92;
/** The widest a bar gets: a chart with a few days of data shows bars, not one block (r1os, 2026-10-09). */
const MAX_BAR = 44;

interface ChartFrameProps {
  kind: "bar" | "line";
  title: string | undefined;
  points: ChartPoint[];
  "data-enee-id"?: string;
}

function ChartFrame({ kind, title, points, ...rest }: ChartFrameProps) {
  const { format } = useRuntime();
  const [plotRef, width] = useWidth<HTMLDivElement>();
  const [hover, setHover] = useState<number | null>(null);
  const gradientId = useId();

  const maxY = useMemo(() => points.reduce((m, p) => Math.max(m, p.y), 0), [points]);
  const [bottom, top] = useMemo(() => yRange(kind, points.map((p) => p.y)), [kind, points]);
  // Bars sum (reps per hour); a line is a reading over time, so it shows the latest.
  const summary = useMemo(
    () =>
      kind === "line"
        ? `Latest ${formatNumber(points.at(-1)?.y ?? 0, undefined, format)}`
        : `Total ${formatNumber(points.reduce((s, p) => s + p.y, 0), undefined, format)}`,
    [kind, points, format],
  );
  const n = points.length;
  const gap = n > 16 ? 3 : 6;
  const slot = n > 0 ? (width + gap) / n : 0;
  const barW = Math.max(1, Math.min(slot - gap, MAX_BAR));
  // A capped bar sits in the middle of its slot.
  const inset = Math.max(0, (slot - gap - barW) / 2);
  const yOf = (v: number) => PLOT_H - ((v - bottom) / (top - bottom)) * PLOT_H;

  const hovered = hover !== null ? points[hover] : undefined;
  // As many labels as fit (time labels with seconds are wide), and never more than six.
  const longest = points.reduce((m, p) => Math.max(m, p.label.length), 1);
  const ticks = tickIndices(n, Math.min(6, Math.max(2, Math.floor(width / (longest * 7 + 16)))));
  const empty = maxY <= 0;

  const onMove = (e: PointerEvent<HTMLDivElement>) => {
    if (slot <= 0) return;
    const rect = e.currentTarget.getBoundingClientRect();
    const i = Math.floor((e.clientX - rect.left) / slot);
    setHover(i >= 0 && i < n ? i : null);
  };

  return (
    <figure className="enee-chart" data-kind={kind} {...rest}>
      <figcaption className="enee-chart__head">
        <span className="enee-label">{title ?? ""}</span>
        <span className="enee-chart__readout">
          {hovered
            ? `${hovered.label} · ${formatNumber(hovered.y, undefined, format)}`
            : empty
              ? "No data yet"
              : summary}
        </span>
      </figcaption>
      <div
        className="enee-chart__plot"
        ref={plotRef}
        onPointerMove={onMove}
        onPointerLeave={() => setHover(null)}
        style={{ height: PLOT_H }}
      >
        {width > 0 && n > 0 ? (
          <svg width={width} height={PLOT_H} viewBox={`0 0 ${width} ${PLOT_H}`} aria-hidden>
            {!empty ? <line className="enee-chart__grid" x1={0} x2={width} y1={0.5} y2={0.5} /> : null}
            {kind === "line" ? (
              <line className="enee-chart__base" x1={0} x2={width} y1={PLOT_H - 0.5} y2={PLOT_H - 0.5} />
            ) : null}
            {kind === "bar" ? (
              points.map((p, i) => {
                const h = p.y > 0 ? Math.max(3, (p.y / top) * PLOT_H) : 0;
                const x = i * slot + inset;
                if (h === 0) {
                  return <rect key={i} className="enee-chart__stub" x={x} y={PLOT_H - 2} width={barW} height={2} rx={1} />;
                }
                const style = { "--y": `${PLOT_H - h}px`, "--h": `${h}px` } as CSSProperties;
                return (
                  <rect
                    key={i}
                    className="enee-chart__bar"
                    data-hover={hover === i ? "true" : undefined}
                    x={x}
                    y={PLOT_H - h}
                    width={barW}
                    height={h}
                    rx={Math.min(3, barW / 2)}
                    style={style}
                  />
                );
              })
            ) : (
              <LineMarks points={points} slot={slot} yOf={yOf} hover={hover} gradientId={gradientId} />
            )}
          </svg>
        ) : null}
        {!empty ? <span className="enee-chart__max">{formatNumber(top, undefined, format)}</span> : null}
        {!empty && bottom > 0 ? <span className="enee-chart__min">{formatNumber(bottom, undefined, format)}</span> : null}
      </div>
      <div className="enee-chart__axis" aria-hidden>
        {width > 0
          ? ticks.map((i) => {
              const center = i * slot + inset + barW / 2;
              const style: CSSProperties = { left: center };
              return (
                <span key={i} style={style} data-edge={i === 0 ? "start" : undefined}>
                  {points[i]?.label}
                </span>
              );
            })
          : null}
      </div>
    </figure>
  );
}

function LineMarks({
  points,
  slot,
  yOf,
  hover,
  gradientId,
}: {
  points: ChartPoint[];
  slot: number;
  yOf: (v: number) => number;
  hover: number | null;
  gradientId: string;
}) {
  const xs = points.map((_, i) => i * slot + Math.max(1, slot - 3) / 2);
  const line = points.map((p, i) => `${i === 0 ? "M" : "L"}${(xs[i] ?? 0).toFixed(1)},${yOf(p.y).toFixed(1)}`).join("");
  const first = xs[0] ?? 0;
  const last = xs[xs.length - 1] ?? 0;
  const area = `${line}L${last.toFixed(1)},${PLOT_H}L${first.toFixed(1)},${PLOT_H}Z`;
  const fillId = `enee-fill-${gradientId.replace(/[^a-zA-Z0-9_-]/g, "")}`;
  return (
    <>
      <defs>
        <linearGradient id={fillId} x1="0" x2="0" y1="0" y2="1">
          <stop offset="0%" stopColor="var(--enee-accent)" stopOpacity={0.28} />
          <stop offset="100%" stopColor="var(--enee-accent)" stopOpacity={0} />
        </linearGradient>
      </defs>
      <path d={area} fill={`url(#${fillId})`} />
      <path className="enee-chart__line" d={line} />
      {points.map((p, i) =>
        p.y > 0 || hover === i ? (
          <circle
            key={i}
            className="enee-chart__dot"
            data-hover={hover === i ? "true" : undefined}
            cx={xs[i]}
            cy={yOf(p.y)}
            r={hover === i ? 3.5 : 2.25}
          />
        ) : null,
      )}
    </>
  );
}

export function BarChart({ props }: Ctx<"BarChart">) {
  const points = useMemo(
    () => chartSeries(props.data, props.x, props.y, props.xDomain),
    [props.data, props.x, props.y, props.xDomain],
  );
  return <ChartFrame kind="bar" title={props.title} points={points} {...eneeId(props)} />;
}

export interface DonutSlice {
  label: string;
  value: number;
  share: number;
  /** The rows past the first five, summed. */
  other?: boolean;
}

/** How many rows a donut names; the rest are summed as Other. */
const DONUT_SLICES = 5;

/** Rows → donut slices: positive `y` values named by `x`, the biggest first, the rest summed as Other. */
export function donutSlices(
  data: readonly Record<string, unknown>[] | null | undefined,
  x: string,
  y: string,
): DonutSlice[] {
  const items = (data ?? [])
    .map((row) => ({ label: toText(field(row, x)) || "–", value: num(field(row, y)) }))
    .filter((item) => item.value > 0)
    .sort((a, b) => b.value - a.value);
  const shown = items.slice(0, DONUT_SLICES);
  const rest = items.slice(DONUT_SLICES).reduce((sum, item) => sum + item.value, 0);
  const slices: Omit<DonutSlice, "share">[] = rest > 0 ? [...shown, { label: "Other", value: rest, other: true }] : shown;
  const total = slices.reduce((sum, item) => sum + item.value, 0);
  return slices.map((item) => ({ ...item, share: total > 0 ? item.value / total : 0 }));
}

/** Shares of a whole: a ring of the slices (the app's accent, fading), and a legend with values and shares. */
export function DonutChart({ props }: Ctx<"DonutChart">) {
  const { format } = useRuntime();
  const slices = useMemo(() => donutSlices(props.data, props.x, props.y), [props.data, props.x, props.y]);
  const total = slices.reduce((sum, s) => sum + s.value, 0);
  // Each slice starts where the last ended; a hairline gap separates them when there are several.
  const gap = slices.length > 1 ? 0.6 : 0;
  let start = 0;
  return (
    <figure className="enee-chart enee-donut" data-kind="donut" {...eneeId(props)}>
      <figcaption className="enee-chart__head">
        <span className="enee-label">{props.title ?? ""}</span>
        <span className="enee-chart__readout">
          {slices.length > 0 ? `Total ${formatNumber(total, undefined, format)}` : "No data yet"}
        </span>
      </figcaption>
      {slices.length > 0 ? (
        <div className="enee-donut__body">
          <svg className="enee-donut__ring" viewBox="0 0 100 100" role="img" aria-label={props.title ?? "Shares"}>
            <circle className="enee-donut__track" cx={50} cy={50} r={38} pathLength={100} />
            {slices.map((s, i) => {
              const length = Math.max(0, s.share * 100 - gap);
              const offset = start;
              start += s.share * 100;
              return (
                <circle
                  key={i}
                  className="enee-donut__slice"
                  data-slice={s.other ? "other" : i}
                  cx={50}
                  cy={50}
                  r={38}
                  pathLength={100}
                  strokeDasharray={`${length} ${100 - length}`}
                  strokeDashoffset={-offset}
                />
              );
            })}
          </svg>
          <ul className="enee-donut__legend">
            {slices.map((s, i) => (
              <li key={i}>
                <span className="enee-donut__swatch" data-slice={s.other ? "other" : i} />
                <span className="enee-donut__name">{s.label}</span>
                <span className="enee-donut__value">
                  {formatNumber(s.value, undefined, format)} · {Math.round(s.share * 100)}%
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </figure>
  );
}

export interface HeatDay {
  /** Local date, `YYYY-MM-DD`. */
  day: string;
  value: number;
  /** 0 (nothing) to 4 (the busiest day). */
  level: number;
  /** Later than today (the rest of this week). */
  future: boolean;
}

function localDay(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/**
 * Rows → a grid of days: `weeks` columns of seven (Monday first), the last holding today. Values on the same local
 * day add up; each day's level is its value against the busiest day's, in four steps.
 */
export function heatmapDays(
  data: readonly Record<string, unknown>[] | null | undefined,
  date: string,
  value: string,
  weeks: number,
  today: Date,
): HeatDay[][] {
  const sums = new Map<string, number>();
  for (const row of data ?? []) {
    const at = parseInstant(field(row, date));
    if (!at) continue;
    const key = localDay(at);
    sums.set(key, (sums.get(key) ?? 0) + Math.max(0, num(field(row, value))));
  }
  const now = new Date(today.getFullYear(), today.getMonth(), today.getDate());
  const monday = new Date(now);
  monday.setDate(now.getDate() - ((now.getDay() + 6) % 7));
  const first = new Date(monday);
  first.setDate(monday.getDate() - 7 * (weeks - 1));
  const columns: HeatDay[][] = [];
  let max = 0;
  for (let w = 0; w < weeks; w++) {
    const column: HeatDay[] = [];
    for (let d = 0; d < 7; d++) {
      const at = new Date(first.getFullYear(), first.getMonth(), first.getDate() + w * 7 + d);
      const day = localDay(at);
      const v = sums.get(day) ?? 0;
      max = Math.max(max, v);
      column.push({ day, value: v, level: 0, future: at > now });
    }
    columns.push(column);
  }
  for (const column of columns) {
    for (const cell of column) cell.level = cell.value > 0 && max > 0 ? Math.max(1, Math.ceil((cell.value / max) * 4)) : 0;
  }
  return columns;
}

const HEAT_GAP = 3;
const HEAT_MAX_CELL = 18;
const DAY_LABEL = new Intl.DateTimeFormat(undefined, { weekday: "short", month: "short", day: "numeric" });

/** Days as squares, a column per week, shaded by how much happened: a streak or a habit at a glance. */
export function CalendarHeatmap({ props }: Ctx<"CalendarHeatmap">) {
  const { format } = useRuntime();
  const [ref, width] = useWidth<HTMLDivElement>();
  const weeks = props.weeks ?? 12;
  const columns = useMemo(
    () => heatmapDays(props.data, props.date, props.value, weeks, new Date()),
    [props.data, props.date, props.value, weeks],
  );
  const active = columns.flat().filter((c) => c.value > 0).length;
  const cell = Math.max(4, Math.min(HEAT_MAX_CELL, Math.floor((width - HEAT_GAP * (weeks - 1)) / weeks)));
  return (
    <figure className="enee-chart enee-heatmap" data-kind="calendar" {...eneeId(props)}>
      <figcaption className="enee-chart__head">
        <span className="enee-label">{props.title ?? ""}</span>
        <span className="enee-chart__readout">{`${formatNumber(active, 0, format)} ${active === 1 ? "day" : "days"}`}</span>
      </figcaption>
      <div className="enee-heatmap__grid" ref={ref} style={{ gap: HEAT_GAP }}>
        {columns.map((column) => (
          <div className="enee-heatmap__week" key={column[0]?.day} style={{ gap: HEAT_GAP }}>
            {column.map((c) => (
              <span
                key={c.day}
                className="enee-heatmap__day"
                data-level={c.future ? undefined : c.level}
                data-future={c.future || undefined}
                style={{ width: cell, height: cell }}
                title={
                  c.future
                    ? undefined
                    : `${DAY_LABEL.format(parseInstant(c.day) ?? new Date())}: ${formatNumber(c.value, undefined, format)}`
                }
              />
            ))}
          </div>
        ))}
      </div>
    </figure>
  );
}

export function LineChart({ props }: Ctx<"LineChart">) {
  const points = useMemo(
    () => chartSeries(props.data, props.x, props.y, props.xDomain),
    [props.data, props.x, props.y, props.xDomain],
  );
  return <ChartFrame kind="line" title={props.title} points={points} {...eneeId(props)} />;
}
