// The Value IR evaluator (docs/specs/compiled-app.md §3). Pure: no React, no json-render.
//
//   { p: "/abs/path", f?, a? }   read a state path (or /params/<name> inside action steps)
//   { i: "field",     f?, a? }   a field of the current row (List row templates, repeat scope)
//   { t: [ "text", Value, … ] }  concatenate parts into a string
//
// Extensions (documented in docs/research/json-render.md, S5 update):
//   - `f` may be an array (a filter chain, as in EneeSpec `{{x | a | b}}`); `a` is then an array of per-filter args.
//   - `{ $state: "/p" }` and `{ $x: Value }` are accepted too, so values copied from json-render props still work.
//   - Plain objects/arrays that are not IR forms are evaluated field by field (action args, `set` maps, …).

import type { FilterName } from "./types";

export interface FormatOptions {
  /** BCP 47 locale; `undefined` = the user's locale. */
  locale?: string;
  /** IANA time zone; `undefined` = the user's local zone. Tests pin it for determinism. */
  timeZone?: string;
  /** Clock for `relative`. */
  now?: () => number;
}

export interface ValueScope {
  /** Resolve an absolute JSON pointer (state paths, `/params/…` inside steps). */
  read: (path: string) => unknown;
  /** The current row, for `{ i }` values. */
  item?: unknown;
  format?: FormatOptions;
}

type Rec = Record<string, unknown>;

const FILTERS = new Set<string>([
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

export function isFilterName(name: unknown): name is FilterName {
  return typeof name === "string" && FILTERS.has(name);
}

function isRecord(v: unknown): v is Rec {
  if (v === null || typeof v !== "object" || Array.isArray(v)) return false;
  const proto = Object.getPrototypeOf(v);
  return proto === Object.prototype || proto === null;
}

function onlyKeys(v: Rec, allowed: readonly string[]): boolean {
  for (const k in v) if (!allowed.includes(k)) return false;
  return true;
}

const P_KEYS = ["p", "f", "a"] as const;
const I_KEYS = ["i", "f", "a"] as const;

/** True for `{p}`, `{i}` and `{t}` objects (exactly those keys). */
export function isValueForm(v: unknown): boolean {
  if (!isRecord(v)) return false;
  if (typeof v.p === "string") return onlyKeys(v, P_KEYS);
  if (typeof v.i === "string") return onlyKeys(v, I_KEYS);
  if (Array.isArray(v.t)) return onlyKeys(v, ["t"]);
  return false;
}

/** Read a JSON pointer (`/a/0/b`) from a plain object. `~1`/`~0` escapes are honoured. */
/** Every path a Value IR reads (`{p}` forms, in templates and nested params too), e.g. a query's `/state/km`. */
export function valuePaths(value: unknown, out: Set<string> = new Set()): Set<string> {
  if (Array.isArray(value)) {
    for (const v of value) valuePaths(v, out);
  } else if (value && typeof value === "object") {
    const v = value as Record<string, unknown>;
    if (typeof v.p === "string") out.add(v.p);
    for (const [k, inner] of Object.entries(v)) if (k !== "p") valuePaths(inner, out);
  }
  return out;
}

export function readPointer(root: unknown, path: string): unknown {
  if (path === "" || path === "/") return root;
  const segments = (path.startsWith("/") ? path.slice(1) : path).split("/");
  let cur: unknown = root;
  for (const raw of segments) {
    if (cur === null || cur === undefined) return undefined;
    const seg = raw.replace(/~1/g, "/").replace(/~0/g, "~");
    if (Array.isArray(cur)) {
      const idx = Number(seg);
      cur = Number.isInteger(idx) ? cur[idx] : undefined;
    } else if (typeof cur === "object") {
      cur = (cur as Rec)[seg];
    } else {
      return undefined;
    }
  }
  return cur;
}

/** Read a row field: `name`, or a dotted path `a.b`, or a pointer `/a/b`. */
function readField(item: unknown, field: string): unknown {
  if (field === "") return item;
  if (field.startsWith("/")) return readPointer(item, field);
  if (!field.includes(".")) return isRecord(item) ? item[field] : undefined;
  return readPointer(item, "/" + field.split(".").join("/"));
}

/** A scope that reads from a plain state object. */
export function stateScope(state: unknown, item?: unknown, format?: FormatOptions): ValueScope {
  const scope: ValueScope = { read: (path) => readPointer(state, path) };
  if (item !== undefined) scope.item = item;
  if (format !== undefined) scope.format = format;
  return scope;
}

/** Evaluate a Value (or a structure containing Values). Never throws. */
export function evalValue(value: unknown, scope: ValueScope): unknown {
  if (value === null || typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map((v) => evalValue(v, scope));
  if (!isRecord(value)) return value;

  if (typeof value.p === "string" && onlyKeys(value, P_KEYS)) {
    return applyFilters(scope.read(value.p), value.f, value.a, scope.format);
  }
  if (typeof value.i === "string" && onlyKeys(value, I_KEYS)) {
    return applyFilters(readField(scope.item, value.i), value.f, value.a, scope.format);
  }
  if (Array.isArray(value.t) && onlyKeys(value, ["t"])) {
    let out = "";
    for (const part of value.t) out += toText(evalValue(part, scope));
    return out;
  }
  if (typeof value.$state === "string" && onlyKeys(value, ["$state"])) return scope.read(value.$state);
  if ("$x" in value && onlyKeys(value, ["$x"])) return evalValue(value.$x, scope);

  const out: Rec = {};
  for (const [k, v] of Object.entries(value)) out[k] = evalValue(v, scope);
  return out;
}

/** String conversion used by `{t}` concatenation: null/undefined → "". */
export function toText(v: unknown): string {
  if (v === null || v === undefined) return "";
  if (typeof v === "string") return v;
  if (typeof v === "number") return Number.isFinite(v) ? String(v) : "";
  if (typeof v === "boolean" || typeof v === "bigint") return String(v);
  if (v instanceof Date) return v.toISOString();
  try {
    return JSON.stringify(v);
  } catch {
    return "";
  }
}

export function applyFilters(
  value: unknown,
  f: unknown,
  a: unknown,
  format: FormatOptions | undefined,
): unknown {
  if (f === undefined || f === null) return value;
  if (Array.isArray(f)) {
    const args = Array.isArray(a) ? a : [];
    return f.reduce<unknown>((acc, name, idx) => applyFilter(acc, name, args[idx], format), value);
  }
  return applyFilter(value, f, a, format);
}

// ---------------------------------------------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------------------------------------------

/** Apply one filter. Null/undefined input yields null (except `default`); unknown filters pass the value through. */
export function applyFilter(value: unknown, name: unknown, arg?: unknown, format: FormatOptions = {}): unknown {
  if (name === "default") return isBlank(value) ? arg ?? null : value;
  if (value === null || value === undefined) return null;
  switch (name) {
    case "time":
      return formatTime(value, format);
    case "date":
      return formatDate(value, format);
    case "datetime":
      return formatDateTime(value, format);
    case "relative":
      return formatRelative(value, format);
    case "duration":
      return mapNumber(value, formatDuration);
    case "seconds":
      return mapNumber(value, formatClock);
    case "number":
      return mapNumber(value, (n) => formatNumber(n, toDigits(arg), format));
    case "percent":
      return mapNumber(value, (n) => formatPercent(n, toDigits(arg), format));
    case "bytes":
      return mapNumber(value, (n) => formatBytes(n, format));
    case "round":
      return mapNumber(value, (n) => roundTo(n, toDigits(arg) ?? 0));
    case "upper":
      return toText(value).toUpperCase();
    case "lower":
      return toText(value).toLowerCase();
    case "plural":
      return mapNumber(value, (n) => pluralize(n, typeof arg === "string" ? arg : "", format));
    default:
      return value;
  }
}

function isBlank(v: unknown): boolean {
  return v === null || v === undefined || v === "" || (typeof v === "number" && Number.isNaN(v));
}

function toDigits(arg: unknown): number | undefined {
  const n = typeof arg === "string" ? Number(arg) : arg;
  if (typeof n !== "number" || !Number.isFinite(n)) return undefined;
  return Math.min(20, Math.max(0, Math.trunc(n)));
}

function toNumber(v: unknown): number | null {
  if (typeof v === "number") return Number.isFinite(v) ? v : null;
  if (typeof v === "string" && v.trim() !== "") {
    const n = Number(v);
    return Number.isFinite(n) ? n : null;
  }
  if (typeof v === "boolean") return v ? 1 : 0;
  return null;
}

/** Apply `fn` to numeric input; non-numeric input passes through unchanged (shown as-is rather than lost). */
function mapNumber<T>(v: unknown, fn: (n: number) => T): T | unknown {
  const n = toNumber(v);
  return n === null ? v : fn(n);
}

export function roundTo(n: number, digits: number): number {
  const m = 10 ** digits;
  return Math.round((n + Number.EPSILON) * m) / m;
}

const numberFormats = new Map<string, Intl.NumberFormat>();
function numberFormat(locale: string | undefined, opts: Intl.NumberFormatOptions): Intl.NumberFormat {
  const key = `${locale ?? ""}|${JSON.stringify(opts)}`;
  let fmt = numberFormats.get(key);
  if (!fmt) {
    fmt = new Intl.NumberFormat(locale, opts);
    numberFormats.set(key, fmt);
  }
  return fmt;
}

export function formatNumber(n: number, digits: number | undefined, format: FormatOptions = {}): string {
  const opts: Intl.NumberFormatOptions =
    digits === undefined ? { maximumFractionDigits: 2 } : { minimumFractionDigits: digits, maximumFractionDigits: digits };
  return numberFormat(format.locale, opts).format(n);
}

/** `percent` treats the input as a ratio (0.42 → "42%"), like `Intl` and json-render's `$format`. */
export function formatPercent(n: number, digits: number | undefined, format: FormatOptions = {}): string {
  const d = digits ?? 0;
  return numberFormat(format.locale, { style: "percent", minimumFractionDigits: d, maximumFractionDigits: d }).format(n);
}

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];
export function formatBytes(n: number, format: FormatOptions = {}): string {
  const sign = n < 0 ? "-" : "";
  let v = Math.abs(n);
  let unit = 0;
  while (v >= 1024 && unit < BYTE_UNITS.length - 1) {
    v /= 1024;
    unit++;
  }
  const digits = unit === 0 || v >= 10 ? 0 : 1;
  return `${sign}${numberFormat(format.locale, { maximumFractionDigits: digits }).format(v)} ${BYTE_UNITS[unit]}`;
}

/** `duration`: seconds → compact human text ("45s", "5m", "1h 30m", "2d 3h"). */
export function formatDuration(totalSeconds: number): string {
  const sign = totalSeconds < 0 ? "-" : "";
  let s = Math.round(Math.abs(totalSeconds));
  if (s === 0) return "0s";
  const d = Math.floor(s / 86400);
  s -= d * 86400;
  const h = Math.floor(s / 3600);
  s -= h * 3600;
  const m = Math.floor(s / 60);
  s -= m * 60;
  const parts: string[] = [];
  if (d) parts.push(`${d}d`);
  if (h) parts.push(`${h}h`);
  if (m && !d) parts.push(`${m}m`);
  if (s && !d && !h) parts.push(`${s}s`);
  return sign + parts.slice(0, 2).join(" ");
}

/** `seconds`: seconds → clock text ("4:05", "1:02:03"), e.g. for timers. */
export function formatClock(totalSeconds: number): string {
  const sign = totalSeconds < 0 ? "-" : "";
  const s = Math.floor(Math.abs(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const pad = (x: number) => String(x).padStart(2, "0");
  return h > 0 ? `${sign}${h}:${pad(m)}:${pad(sec)}` : `${sign}${m}:${pad(sec)}`;
}

/** `plural`: `n` + word, with the English plural ("1 set", "3 sets"); `a` may be "person|people". */
export function pluralize(n: number, word: string, format: FormatOptions = {}): string {
  const [one = "", many] = word.split("|");
  const plural = many ?? englishPlural(one);
  const num = formatNumber(n, undefined, format);
  if (!one) return num;
  return `${num} ${Math.abs(n) === 1 ? one : plural}`;
}

function englishPlural(w: string): string {
  if (!w) return w;
  if (/[^aeiou]y$/i.test(w)) return w.slice(0, -1) + "ies";
  if (/(s|x|z|ch|sh)$/i.test(w)) return w + "es";
  return w + "s";
}

// ----- dates ---------------------------------------------------------------------------------------------------

const DATE_ONLY = /^(\d{4})-(\d{2})-(\d{2})$/;
const TIME_ONLY = /^(\d{1,2}):(\d{2})(?::(\d{2}))?$/;
const SQLITE_UTC = /^(\d{4}-\d{2}-\d{2}) (\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?)$/;

/**
 * Parse what the host stores (contract §6: UTC ISO-8601 text with ms), plus SQLite's `YYYY-MM-DD HH:MM:SS` (UTC),
 * date-only strings (local midnight) and epoch numbers (ms, or seconds when < 1e11).
 */
export function parseInstant(v: unknown): Date | null {
  if (v instanceof Date) return Number.isNaN(v.getTime()) ? null : v;
  if (typeof v === "number") {
    if (!Number.isFinite(v)) return null;
    return new Date(Math.abs(v) < 1e11 ? v * 1000 : v);
  }
  if (typeof v !== "string" || v === "") return null;
  const dateOnly = DATE_ONLY.exec(v);
  if (dateOnly) return new Date(Number(dateOnly[1]), Number(dateOnly[2]) - 1, Number(dateOnly[3]));
  const sqlite = SQLITE_UTC.exec(v);
  const d = new Date(sqlite ? `${sqlite[1]}T${sqlite[2]}Z` : v);
  return Number.isNaN(d.getTime()) ? null : d;
}

const dateFormats = new Map<string, Intl.DateTimeFormat>();
function dateFormat(format: FormatOptions, opts: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = `${format.locale ?? ""}|${format.timeZone ?? ""}|${JSON.stringify(opts)}`;
  let fmt = dateFormats.get(key);
  if (!fmt) {
    fmt = new Intl.DateTimeFormat(format.locale, format.timeZone ? { ...opts, timeZone: format.timeZone } : opts);
    dateFormats.set(key, fmt);
  }
  return fmt;
}

const TIME_OPTS: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };

export function formatTime(v: unknown, format: FormatOptions = {}): unknown {
  if (typeof v === "string") {
    const t = TIME_ONLY.exec(v);
    if (t) {
      // A time of day (setting type `time`): format without any zone conversion.
      const d = new Date(Date.UTC(2000, 0, 1, Number(t[1]), Number(t[2])));
      return dateFormat({ ...format, timeZone: "UTC" }, TIME_OPTS).format(d);
    }
  }
  const d = parseInstant(v);
  return d ? dateFormat(format, TIME_OPTS).format(d) : v;
}

function sameYear(d: Date, format: FormatOptions): boolean {
  const now = new Date(format.now ? format.now() : Date.now());
  const y = dateFormat(format, { year: "numeric" });
  return y.format(d) === y.format(now);
}

export function formatDate(v: unknown, format: FormatOptions = {}): unknown {
  const d = parseInstant(v);
  if (!d) return v;
  const opts: Intl.DateTimeFormatOptions = sameYear(d, format)
    ? { month: "short", day: "numeric" }
    : { month: "short", day: "numeric", year: "numeric" };
  // Date-only strings were parsed as *local* midnight: never shift them through another zone.
  const f = typeof v === "string" && DATE_ONLY.test(v) ? { ...format, timeZone: undefined } : format;
  return dateFormat(f, opts).format(d);
}

export function formatDateTime(v: unknown, format: FormatOptions = {}): unknown {
  const d = parseInstant(v);
  if (!d) return v;
  const opts: Intl.DateTimeFormatOptions = sameYear(d, format)
    ? { month: "short", day: "numeric", ...TIME_OPTS }
    : { month: "short", day: "numeric", year: "numeric", ...TIME_OPTS };
  return dateFormat(format, opts).format(d);
}

const relativeFormats = new Map<string, Intl.RelativeTimeFormat>();
export function formatRelative(v: unknown, format: FormatOptions = {}): unknown {
  const d = parseInstant(v);
  if (!d) return v;
  const now = format.now ? format.now() : Date.now();
  const diffSec = (d.getTime() - now) / 1000;
  const abs = Math.abs(diffSec);
  if (abs < 45) return "just now";
  const key = format.locale ?? "";
  let rtf = relativeFormats.get(key);
  if (!rtf) {
    rtf = new Intl.RelativeTimeFormat(format.locale, { numeric: "auto", style: "short" });
    relativeFormats.set(key, rtf);
  }
  if (abs < 3600) return rtf.format(Math.round(diffSec / 60), "minute");
  if (abs < 86400) return rtf.format(Math.round(diffSec / 3600), "hour");
  if (abs < 604800) return rtf.format(Math.round(diffSec / 86400), "day");
  if (abs < 2592000) return rtf.format(Math.round(diffSec / 604800), "week");
  return formatDate(v, format);
}
