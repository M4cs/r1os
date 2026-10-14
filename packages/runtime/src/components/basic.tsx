import { type RefObject, useEffect, useLayoutEffect, useRef, useState } from "react";

import { useRuntime } from "../context";
import { eneeId } from "../spec";
import { applyFilter, formatNumber, parseInstant, toText } from "../value";
import { Icon } from "./icons";
import type { Ctx } from "./types";

export function Stack({ props, children }: Ctx<"Stack">) {
  return (
    <div className="enee-stack" data-gap={props.gap ?? "md"} {...eneeId(props)}>
      {children}
    </div>
  );
}

/** The smallest a stat's value shrinks to, as a share of its normal size. */
const MIN_FIT = 0.45;

/**
 * Stat values wider than their column shrink to fit: `--enee-fit` (on `ref`) scales their font size. A row's stats shrink
 * together, so their values stay one size (design-systems.md §5.2). Re-fits when a value or the width changes.
 */
function useFitStats(ref: RefObject<HTMLElement | null>, values: string) {
  useLayoutEffect(() => {
    const el = ref.current;
    // A stat inside a row of stats is fitted by its row.
    if (!el || !el.querySelector(values) || el.parentElement?.matches(".enee-row")) return;
    const fit = () => {
      el.style.removeProperty("--enee-fit");
      let ratio = 1;
      for (const v of el.querySelectorAll<HTMLElement>(values)) {
        if (v.scrollWidth > v.clientWidth + 0.5) ratio = Math.min(ratio, v.clientWidth / v.scrollWidth);
      }
      if (ratio < 1) el.style.setProperty("--enee-fit", String(Math.max(MIN_FIT, Math.floor(ratio * 100) / 100)));
    };
    fit();
    const resized = new ResizeObserver(fit);
    resized.observe(el);
    const changed = new MutationObserver(fit);
    changed.observe(el, { subtree: true, childList: true, characterData: true });
    return () => {
      resized.disconnect();
      changed.disconnect();
    };
  }, [ref, values]);
}

export function Row({ props, children }: Ctx<"Row">) {
  const ref = useRef<HTMLDivElement>(null);
  useFitStats(ref, ":scope > .enee-stat > .enee-stat__value");
  return (
    <div
      ref={ref}
      className="enee-row"
      data-gap={props.gap ?? "sm"}
      data-align={props.align ?? "start"}
      data-wrap={props.wrap ? "true" : undefined}
      {...eneeId(props)}
    >
      {children}
    </div>
  );
}

export function Card({ props, children }: Ctx<"Card">) {
  return (
    <section className="enee-card" {...eneeId(props)}>
      {props.title ? <h3 className="enee-label enee-card__title">{props.title}</h3> : null}
      {children}
    </section>
  );
}

export function Heading({ props }: Ctx<"Heading">) {
  const level = props.level ?? 2;
  const Tag = level === 1 ? "h1" : level === 2 ? "h2" : "h3";
  return (
    <Tag className="enee-heading" data-level={level} {...eneeId(props)}>
      {props.text}
    </Tag>
  );
}

export function Text({ props }: Ctx<"Text">) {
  return (
    <p className="enee-text" data-tone={props.tone ?? "default"} {...eneeId(props)}>
      {props.text}
    </p>
  );
}

/** Numbers get grouping ("1,250"); `format` applies a Value IR filter. Missing values show an en dash. */
export function displayValue(value: unknown, format: string | undefined, fmt = {}): string {
  if (value === null || value === undefined || value === "") return "–";
  if (format) return toText(applyFilter(value, format, undefined, fmt)) || "–";
  if (typeof value === "number") return formatNumber(value, undefined, fmt);
  return toText(value);
}

/** Live values (a service sampled every second) change too often to animate. */
const SETTLE_MIN_GAP_MS = 1500;

/** Bumps when the text changes after a quiet spell, which replays the settle animation; rapid changes don't. */
function useSettleKey(text: string): number {
  const [key, setKey] = useState(0);
  const last = useRef(0);
  useEffect(() => {
    const now = Date.now();
    if (now - last.current >= SETTLE_MIN_GAP_MS) setKey((k) => k + 1);
    last.current = now;
  }, [text]);
  return key;
}

export function Stat({ props }: Ctx<"Stat">) {
  const { format } = useRuntime();
  const text = displayValue(props.value, props.format, format);
  const settle = useSettleKey(text);
  const ref = useRef<HTMLDivElement>(null);
  useFitStats(ref, ":scope > .enee-stat__value");
  return (
    <div className="enee-stat" ref={ref} {...eneeId(props)}>
      <div className="enee-label">{props.label}</div>
      {/* Keyed so a change replays a short (compositor-only) settle animation. */}
      <div className="enee-stat__value" key={settle}>
        {text}
      </div>
      {props.caption ? <div className="enee-stat__caption">{props.caption}</div> : null}
    </div>
  );
}

/** A timer's reading: `m:ss`, then `h:mm:ss` from an hour, and `2d 5h` from a day. */
export function clockText(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const pad = (n: number) => String(n).padStart(2, "0");
  if (s >= 86_400) return `${Math.floor(s / 86_400)}d ${Math.floor((s % 86_400) / 3600)}h`;
  if (s >= 3600) return `${Math.floor(s / 3600)}:${pad(Math.floor((s % 3600) / 60))}:${pad(s % 60)}`;
  return `${Math.floor(s / 60)}:${pad(s % 60)}`;
}

/** A stat that ticks: a countdown to `until`, or a stopwatch from `since`. Laid out exactly like a stat. */
export function Timer({ props }: Ctx<"Timer">) {
  const untilMs = parseInstant(props.until)?.getTime() ?? null;
  const sinceMs = untilMs === null ? (parseInstant(props.since)?.getTime() ?? null) : null;
  const [now, setNow] = useState(() => Date.now());
  // Re-render as the reading changes: on the countdown's or the stopwatch's own second boundaries, not the clock's.
  useEffect(() => {
    if (untilMs === null && sinceMs === null) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = () => {
      const t = Date.now();
      setNow(t);
      if (untilMs !== null) {
        const left = untilMs - t;
        if (left > 0) timer = setTimeout(tick, (left % 1000 || 1000) + 2);
      } else if (sinceMs !== null) {
        timer = setTimeout(tick, 1000 - ((t - sinceMs) % 1000) + 2);
      }
    };
    tick();
    return () => clearTimeout(timer);
  }, [untilMs, sinceMs]);
  const done = untilMs !== null && untilMs <= now;
  const text =
    untilMs !== null
      ? done
        ? (props.done ?? "0:00")
        : clockText(Math.ceil((untilMs - now) / 1000))
      : sinceMs !== null
        ? clockText((now - sinceMs) / 1000)
        : "–";
  const ref = useRef<HTMLDivElement>(null);
  useFitStats(ref, ":scope > .enee-stat__value");
  return (
    <div className="enee-stat enee-timer" data-done={done ? "true" : undefined} ref={ref} {...eneeId(props)}>
      <div className="enee-label">{props.label}</div>
      <div className="enee-stat__value" role="timer">
        {text}
      </div>
      {props.caption ? <div className="enee-stat__caption">{props.caption}</div> : null}
    </div>
  );
}

/** A number from a prop that may be text (a value with a filter): `null` when it isn't one. */
function numeric(v: unknown): number | null {
  const n = typeof v === "number" ? v : typeof v === "string" && v.trim() !== "" ? Number(v.replace(/,/g, "")) : NaN;
  return Number.isFinite(n) ? n : null;
}

/** How full a gauge is, 0–1: `value` of `max` (100 when it's missing or not positive). */
export function gaugeRatio(value: unknown, max: unknown): number {
  const v = numeric(value);
  const m = numeric(max);
  const top = m !== null && m > 0 ? m : 100;
  return v === null ? 0 : Math.min(1, Math.max(0, v / top));
}

/** The half circle a gauge draws on: centre (60, 60), radius 50, left to right over the top. */
const GAUGE_ARC = "M 10 60 A 50 50 0 0 1 110 60";

/**
 * A half-circle meter with the value in the middle. It doesn't animate: gauges show live values (a service sampled
 * every second), and the arc's dash isn't a compositor property.
 */
export function Gauge({ props }: Ctx<"Gauge">) {
  const { format } = useRuntime();
  const value = numeric(props.value);
  const ratio = gaugeRatio(props.value, props.max);
  const max = numeric(props.max);
  const digits = max !== null && max > 0 && max <= 10 ? 1 : 0;
  const text = value === null ? "–" : `${formatNumber(value, digits, format)}${props.unit ?? ""}`;
  return (
    <div className="enee-gauge" data-full={ratio >= 1 ? "true" : undefined} {...eneeId(props)}>
      <div className="enee-label">{props.label}</div>
      <div className="enee-gauge__dial">
        <svg viewBox="0 0 120 66" role="meter" aria-valuemin={0} aria-valuemax={max ?? 100} aria-valuenow={value ?? 0}>
          <path className="enee-gauge__track" d={GAUGE_ARC} pathLength={100} />
          {ratio > 0 ? (
            <path className="enee-gauge__fill" d={GAUGE_ARC} pathLength={100} strokeDasharray={`${ratio * 100} 100`} />
          ) : null}
        </svg>
        <div className="enee-gauge__value">{text}</div>
      </div>
      {props.caption ? <div className="enee-stat__caption">{props.caption}</div> : null}
    </div>
  );
}

export function Progress({ props }: Ctx<"Progress">) {
  const value = typeof props.value === "number" ? props.value : 0;
  const max = typeof props.max === "number" && props.max > 0 ? props.max : 0;
  const ratio = max > 0 ? Math.min(1, Math.max(0, value / max)) : 0;
  const done = max > 0 && value >= max;
  return (
    <div className="enee-progress" data-done={done ? "true" : undefined} {...eneeId(props)}>
      {props.label ? (
        <div className="enee-progress__head">
          <span className="enee-label">{props.label}</span>
          <span className="enee-progress__value">
            {formatNumber(value, 0)} / {formatNumber(max, 0)}
          </span>
        </div>
      ) : null}
      <div
        className="enee-progress__track"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={max}
        aria-valuenow={value}
      >
        <div className="enee-progress__fill" style={{ transform: `translateX(${(ratio - 1) * 100}%)` }} />
      </div>
    </div>
  );
}

export function EmptyState({ props }: Ctx<"EmptyState">) {
  return <Empty text={props.text} icon={props.icon} {...eneeId(props)} />;
}

export function Empty({ text, icon, ...rest }: { text: string; icon?: string | undefined; "data-enee-id"?: string }) {
  return (
    <div className="enee-empty" {...rest}>
      <span className="enee-empty__icon">
        <Icon name={icon ?? "inbox"} size={18} />
      </span>
      <span>{text}</span>
    </div>
  );
}
