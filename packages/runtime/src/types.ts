// TypeScript mirror of the compiler ⇄ runtime contract (docs/specs/compiled-app.md, v0).
// Keep in sync with the spec; the Rust compiler (`crates/enee-spec`) is the producer.

/** Filters usable in the Value IR (contract §3). All are null-safe. */
export type FilterName =
  | "time"
  | "date"
  | "datetime"
  | "relative"
  | "duration"
  | "seconds"
  | "number"
  | "percent"
  | "bytes"
  | "round"
  | "upper"
  | "lower"
  | "plural"
  | "default";

/** Read a state path (or `/params/<name>` inside action steps), optionally formatted. */
export interface PathValue {
  p: string;
  /** Filter name. The runtime also accepts an array (a chain), see `value.ts`. */
  f?: FilterName | FilterName[];
  a?: unknown;
}

/** A field of the current row (row templates only). */
export interface ItemValue {
  i: string;
  f?: FilterName | FilterName[];
  a?: unknown;
}

/** Concatenate parts into a string. */
export interface TemplateValue {
  t: Value[];
}

export type Literal = string | number | boolean | null;

/** The Value IR (contract §3): a literal or one of the three forms. */
export type Value = Literal | PathValue | ItemValue | TemplateValue;

export type Row = Record<string, unknown>;

export interface SettingDef {
  type: string;
  default: unknown;
  label?: string;
  min?: number;
  max?: number;
  options?: string[];
}

export interface ColumnDef {
  name: string;
  type: string;
  notNull: boolean;
  primaryKey?: boolean;
  default?: unknown;
  /** `enum`: its options. */
  options?: string[];
  /** `ref`: the table it points to. */
  references?: string;
}

export interface TableDef {
  name: string;
  columns: ColumnDef[];
}

export interface QueryDef {
  name: string;
  sql: string;
  params: Record<string, Value>;
  tables: string[];
  columns: string[];
}

export interface ParamDef {
  type: string;
  required: boolean;
}

export type Step =
  | { sql: string; params?: string[] }
  | { toast: Value }
  | { set: Record<string, Value> }
  | { navigate: string }
  | { notify: { title: Value; body?: Value } }
  | { refresh: string }
  | { open_url: Value }
  | { copy: Value }
  | { copy_parts: { text?: Value; image?: Value; files?: Value } }
  | { launch: Value }
  | { play: { url: Value; title?: Value; subtitle?: Value } }
  | { pause: true }
  | { resume: true }
  | { stop: true }
  | { toggle: true }
  | { run: string };

/** The logic lane (ADR-0041): the app's script, the values it publishes and the actions it answers. */
export interface LogicDef {
  script: string;
  values: string[];
  actions: string[];
}

/** What `r1.fetch` gets back: the response's status and its body, read as asked. */
export interface FetchResult {
  status: number;
  body: unknown;
}

/** What a player step asks of the R1's player (ADR-0040). */
export type MediaOp = "play" | "pause" | "resume" | "stop" | "toggle";

/**
 * What a `copy_parts` step copies (clipboard history, ADR-0022): the first of these that isn't empty, in the order
 * files (full paths, one per line), image (one of the app's own files, `files/<sha256>.png`), text.
 */
export interface CopyParts {
  text?: string;
  image?: string;
  files?: string;
}

export interface ActionDef {
  params: Record<string, ParamDef>;
  confirm?: string;
  steps: Step[];
}

/** A json-render flat spec as emitted by the compiler (`children` may be omitted on leaves). */
export interface ViewSpec {
  root: string;
  elements: Record<string, ViewElement>;
}

export interface ViewElement {
  type: string;
  props?: Record<string, unknown>;
  children?: string[];
  slots?: Record<string, string[]>;
  visible?: unknown;
  on?: Record<string, unknown>;
  repeat?: unknown;
  watch?: Record<string, unknown>;
}

export interface ViewDef {
  name: string;
  title?: string;
  icon?: string;
  spec: ViewSpec;
}

export interface TriggerDef {
  name: string;
  every?: Value;
  when?: string;
  steps: Step[];
  [key: string]: unknown;
}

export interface CheckDef {
  do: { action: string; args?: Record<string, unknown> };
  expect: Record<string, unknown>;
}

export type ThemeMode = "dark" | "light" | "system";

export interface CompiledApp {
  enee: 1;
  app: {
    id: string;
    name: string;
    icon: string;
    archetype: string;
    design: string;
    accent?: string;
    theme: ThemeMode;
    description?: string;
  };
  permissions: { id: string; scope?: unknown; reason: string }[];
  settings: Record<string, SettingDef>;
  tables: TableDef[];
  ddl: string[];
  queries: QueryDef[];
  state: Record<string, unknown>;
  /** State keys kept across restarts (`state._persist`); absent when none. */
  persist?: string[];
  /** Live services; the host produces their values at `/services/<name>` (absent when the app has none). */
  services?: ServiceDef[];
  actions: Record<string, ActionDef>;
  views: ViewDef[];
  /** The app's own code (ADR-0041), when it has some. */
  logic?: LogicDef;
  triggers: TriggerDef[];
  checks: CheckDef[];
  sourcemap: Record<string, { path: string; block: string }>;
}

/** A live service (compiled-app.md §8). */
export interface ServiceDef {
  name: string;
  use: string;
  everyMs?: number;
  keepMs?: number;
  options?: Record<string, unknown>;
  /** `/state/…` and `/settings/…` values its URL binds: the runtime reports them (`Host.serviceVars`). */
  deps?: string[];
}

// ---------------------------------------------------------------------------------------------------------------
// Host interface: what the runtime needs from whoever executes SQL and native side effects.
// Production: the Rust core behind the bridge (enee-apps.md §4). Dev: `dev/sqljs-host.ts`.
// ---------------------------------------------------------------------------------------------------------------

export interface HostInit {
  /** Current setting values (from the app's `settings` row). Missing keys fall back to `SettingDef.default`. */
  settings?: Record<string, unknown>;
  /** Saved values of the app's `persist` state keys, by key. */
  state?: Record<string, unknown>;
}

/** Identifies the compiled step being executed, so a production host can run the *compiled* SQL (enee-apps.md §3). */
export interface StepRef {
  action: string;
  step: number;
}

export interface ExecResult {
  /** Tables whose rows changed. The runtime re-runs every query whose `tables` intersect these. */
  changedTables: string[];
  rowsAffected?: number;
}

export interface Host {
  /** Prepare the app's database (run `ddl`) and return the current settings. Must be idempotent. */
  init(app: CompiledApp): Promise<HostInit>;
  /** Run a read-only query; `params` are already evaluated (named, without the `:` prefix). */
  query(def: QueryDef, params: Record<string, unknown>): Promise<Row[]>;
  /** Run a write; `params` are bound to `:name` placeholders. */
  exec(sql: string, params: Record<string, unknown>, ref?: StepRef): Promise<ExecResult>;
  /** Current setting values; re-read after a write to the `settings` table. */
  settings?(): Promise<Record<string, unknown>>;
  /**
   * Live channel for patches the host originates itself (e.g. `/q/<name>` after a core-side trigger wrote a table,
   * `/services/…`). Paths under `/state` are runtime-owned and ignored. Returns an unsubscribe function.
   */
  subscribe?(push: (updates: Record<string, unknown>) => void): () => void;
  /**
   * The current values of the paths the app's service URLs bind (`ServiceDef.deps`, e.g. `/state/ticker`), sent
   * when they change. The host builds the URLs itself, so values are all it gets.
   */
  serviceVars?(values: Record<string, unknown>): Promise<void>;
  // Side-effect steps. `ref` names the compiled step being run, so a bridged host can check that the app really
  // declares such a step (the frame can't trigger side effects its spec doesn't contain).
  notify?(title: string, body?: string, ref?: StepRef): Promise<void>;
  openUrl?(url: string, ref?: StepRef): Promise<void>;
  copy?(text: string, ref?: StepRef): Promise<void>;
  /** Copies an item back: a `copy_parts` step's parts. */
  copyParts?(parts: CopyParts, ref?: StepRef): Promise<void>;
  /** Starts a desktop app by its Start-menu name (`apps.launch`). */
  launch?(name: string, ref?: StepRef): Promise<void>;
  /**
   * The R1's player (`audio.play`, ADR-0040): `play` a stream (its URL and names), or pause, resume, stop or toggle
   * what this app is playing. What it's doing comes back as `/player` (`subscribe`).
   */
  media?(op: MediaOp, args: { url?: string; title?: string; subtitle?: string }, ref?: StepRef): Promise<void>;
  /**
   * The web, for the app's logic (`r1.fetch`, ADR-0041): the host checks the URL against the app's `net.http`
   * hosts and fetches it, reading the body as JSON, text or a feed.
   */
  fetch?(url: string, as: "json" | "text" | "feed"): Promise<FetchResult>;
  /** Saves the app's `persist` state keys (by key) for its next start. */
  saveState?(values: Record<string, unknown>): Promise<void>;
  /**
   * The host's own writes (a trigger, the command bar, a feed): calls `listener` with the tables they changed, so the
   * app re-runs the queries that read them with its own state and settings. Returns an unsubscribe function.
   */
  onChanged?(listener: (tables: string[]) => void): () => void;
}

// ---------------------------------------------------------------------------------------------------------------
// Runtime-facing types
// ---------------------------------------------------------------------------------------------------------------

export type EneeErrorCode =
  | "E_INIT"
  | "E_SPEC"
  | "E_QUERY"
  | "E_ACTION_UNKNOWN"
  | "E_ARG_MISSING"
  | "E_ARG_TYPE"
  | "E_STEP"
  | "E_STEP_UNSUPPORTED"
  | "E_LOGIC"
  | "E_SQL_PARAM"
  | "E_HOST"
  | "E_RENDER";

/** Errors surfaced to `onError` (and, for user-visible failures, a toast). */
export class EneeError extends Error {
  readonly code: EneeErrorCode;
  readonly action?: string;
  readonly step?: number;
  readonly element?: string;

  constructor(
    code: EneeErrorCode,
    message: string,
    details: { action?: string; step?: number; element?: string; cause?: unknown } = {},
  ) {
    super(message, details.cause === undefined ? undefined : { cause: details.cause });
    this.name = "EneeError";
    this.code = code;
    if (details.action !== undefined) this.action = details.action;
    if (details.step !== undefined) this.step = details.step;
    if (details.element !== undefined) this.element = details.element;
  }
}

export type ToastTone = "default" | "success" | "danger";

export interface ConfirmRequest {
  title: string;
  message?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  variant?: "default" | "danger";
}
