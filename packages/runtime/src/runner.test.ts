import { describe, expect, it, vi } from "vitest";

import pushupsJson from "../../../fixtures/apps/pushups.compiled.json";
import { initialModel } from "./model";
import { coerceArg, createRunner, sqlParamNames } from "./runner";
import { createEneeStore } from "./store";
import type { CompiledApp, ConfirmRequest, EneeError, Host, QueryDef } from "./types";

const pushups = pushupsJson as unknown as CompiledApp;

/** In-memory fake of the pushup database: enough SQL "understanding" to exercise the runner. */
function fakeHost() {
  let sets: { id: number; reps: number; at: string }[] = [];
  let nextId = 1;
  const exec = vi.fn(async (sql: string, params: Record<string, unknown>) => {
    if (sql.startsWith("INSERT INTO sets")) {
      sets.push({ id: nextId++, reps: params.reps as number, at: new Date().toISOString() });
      return { changedTables: ["sets"], rowsAffected: 1 };
    }
    if (sql.startsWith("DELETE FROM sets")) {
      const before = sets.length;
      sets = sets.filter((s) => s.id !== params.id);
      return { changedTables: before === sets.length ? [] : ["sets"], rowsAffected: before - sets.length };
    }
    if (sql.startsWith("UPDATE other")) return { changedTables: ["other"] };
    throw new Error(`unexpected SQL: ${sql}`);
  });
  const query = vi.fn(async (def: QueryDef, _params?: Record<string, unknown>) => {
    if (def.name === "today") return [{ total: sets.reduce((s, x) => s + x.reps, 0), n: sets.length }];
    if (def.name === "per_hour") return sets.length ? [{ hour: 14, reps: sets.reduce((s, x) => s + x.reps, 0) }] : [];
    if (def.name === "recent") return [...sets].reverse();
    return [];
  });
  const host: Host = {
    init: async () => ({ settings: { daily_goal: 100, remind_every: 3600 } }),
    query,
    exec,
    notify: vi.fn(async () => {}),
  };
  return { host, exec, query, sets: () => sets };
}

function setup(app: CompiledApp = pushups, answer = true) {
  const fake = fakeHost();
  const store = createEneeStore(initialModel(app));
  const toasts: [string, string | undefined][] = [];
  const confirms: ConfirmRequest[] = [];
  const errors: EneeError[] = [];
  const navigations: string[] = [];
  const runner = createRunner({
    app,
    host: fake.host,
    store,
    onError: (e) => errors.push(e),
    ui: {
      toast: (m, tone) => toasts.push([m, tone]),
      confirm: async (req) => {
        confirms.push(req);
        return answer;
      },
      navigate: (v) => navigations.push(v),
    },
  });
  return { ...fake, store, runner, toasts, confirms, errors, navigations };
}

describe("run", () => {
  it("logs a set: binds :reps, refreshes the queries on `sets`, toasts with /params", async () => {
    const t = setup();
    await t.runner.refresh();
    expect(await t.runner.run("log", { reps: 20 })).toEqual({ ok: true });
    expect(t.exec).toHaveBeenCalledWith("INSERT INTO sets (reps) VALUES (:reps)", { reps: 20 }, { action: "log", step: 0 });
    expect(t.store.get("/q/today/0/total")).toBe(20);
    expect(t.store.get("/q/today/0/n")).toBe(1);
    expect(t.store.get("/q/per_hour")).toEqual([{ hour: 14, reps: 20 }]);
    expect((t.store.get("/q/recent") as unknown[]).length).toBe(1);
    expect(t.toasts).toEqual([["+20 logged", undefined]]);
    expect(t.errors).toEqual([]);
  });

  it("satisfies the fixture's own `checks`", async () => {
    const t = setup();
    await t.runner.refresh();
    for (const check of pushups.checks) {
      expect((await t.runner.run(check.do.action, check.do.args)).ok).toBe(true);
      for (const [path, value] of Object.entries(check.expect)) expect(t.store.get(path)).toBe(value);
    }
  });

  it("coerces args to declared types and drops undeclared ones", async () => {
    const t = setup();
    await t.runner.run("log", { reps: "15", extra: "dropped" });
    expect(t.exec).toHaveBeenLastCalledWith(expect.any(String), { reps: 15 }, expect.anything());
  });

  it("rejects a missing required param before touching the host", async () => {
    const t = setup();
    const outcome = await t.runner.run("log", { reps: null });
    expect(outcome.ok).toBe(false);
    expect(t.exec).not.toHaveBeenCalled();
    expect(t.toasts).toEqual([["Reps is required", "danger"]]);
    expect(t.errors[0]?.code).toBe("E_ARG_MISSING");
    expect(t.errors[0]?.action).toBe("log");
  });

  it("rejects wrongly typed args", async () => {
    const t = setup();
    expect((await t.runner.run("log", { reps: "lots" })).ok).toBe(false);
    expect((await t.runner.run("log", { reps: 1.5 })).ok).toBe(false);
    expect(t.errors.map((e) => e.code)).toEqual(["E_ARG_TYPE", "E_ARG_TYPE"]);
    expect(t.exec).not.toHaveBeenCalled();
  });

  it("asks for confirmation (danger variant for DELETE) and runs on yes", async () => {
    const t = setup();
    await t.runner.run("log", { reps: 10 });
    const id = t.sets()[0]?.id;
    expect(await t.runner.run("remove", { id })).toEqual({ ok: true });
    expect(t.confirms).toEqual([
      { title: "Delete this set?", message: "This can't be undone.", confirmLabel: "Delete", cancelLabel: "Cancel", variant: "danger" },
    ]);
    expect(t.sets()).toEqual([]);
    expect(t.store.get("/q/today/0/total")).toBe(0);
  });

  it("does nothing when the confirmation is cancelled", async () => {
    const t = setup(pushups, false);
    await t.runner.run("log", { reps: 10 });
    expect(await t.runner.run("remove", { id: 1 })).toEqual({ ok: false, cancelled: true });
    expect(t.sets().length).toBe(1);
    expect(t.errors).toEqual([]);
  });

  it("reports unknown actions", async () => {
    const t = setup();
    const outcome = await t.runner.run("nope");
    expect(outcome.ok).toBe(false);
    expect(t.errors[0]?.code).toBe("E_ACTION_UNKNOWN");
  });

  it("does not refresh when nothing changed", async () => {
    const t = setup();
    await t.runner.refresh();
    t.query.mockClear();
    await t.runner.run("remove", { id: 999 });
    expect(t.query).not.toHaveBeenCalled();
  });
});

describe("steps", () => {
  const app: CompiledApp = {
    ...pushups,
    state: { draft: { reps: 15 }, note: "hi", reps: 7 },
    views: [...pushups.views, { name: "stats", spec: { root: "s", elements: { s: { type: "Stack" } } } }],
    actions: {
      fromState: { params: {}, steps: [{ sql: "INSERT INTO sets (reps) VALUES (:reps)", params: ["reps"] }] },
      fromSettings: { params: {}, steps: [{ sql: "INSERT INTO sets (reps) VALUES (:daily_goal)" }] },
      unbound: { params: {}, steps: [{ sql: "INSERT INTO sets (reps) VALUES (:missing)", params: ["missing"] }] },
      setter: {
        params: { n: { type: "int", required: true } },
        steps: [
          { set: { "/state/draft/reps": { p: "/params/n" }, "/state/note": { t: ["n=", { p: "/params/n" }] } } },
          { navigate: "stats" },
          { notify: { title: "Hi", body: { t: [{ p: "/settings/daily_goal" }, " goal"] } } },
        ],
      },
      badSet: { params: {}, steps: [{ set: { "/q/today": null } }] },
      badView: { params: {}, steps: [{ navigate: "nope" }] },
      failing: { params: {}, steps: [{ toast: "before" }, { sql: "BOOM" }, { toast: "after" }] },
      otherTable: { params: {}, steps: [{ sql: "UPDATE other SET x = 1" }] },
      refresher: { params: {}, steps: [{ refresh: "today" }] },
      copyItem: {
        params: { text: { type: "string", required: false }, image: { type: "string", required: false } },
        steps: [{ copy_parts: { text: { p: "/params/text" }, image: { p: "/params/image" } } }],
      },
    },
  };

  it("binds SQL params from args, then /state, then /settings (and infers names from SQL)", async () => {
    const t = setup(app);
    await t.runner.run("fromState");
    await t.runner.run("fromSettings");
    expect(t.exec.mock.calls.map((c) => c[1])).toEqual([{ reps: 7 }, { daily_goal: 100 }]);
    expect((await t.runner.run("unbound")).ok).toBe(false);
    expect(t.errors.at(-1)?.code).toBe("E_SQL_PARAM");
  });

  it("set / navigate / notify evaluate Values with /params", async () => {
    const t = setup(app);
    expect(await t.runner.run("setter", { n: "42" })).toEqual({ ok: true });
    expect(t.store.get("/state/draft/reps")).toBe(42);
    expect(t.store.get("/state/note")).toBe("n=42");
    expect(t.navigations).toEqual(["stats"]);
    // The step ref lets a bridged host check that the app declares this step (S3).
    expect(t.host.notify).toHaveBeenCalledWith("Hi", "100 goal", { action: "setter", step: 2 });
  });

  it("copy_parts evaluates the parts it names, and needs a host that copies them", async () => {
    const t = setup(app);
    expect((await t.runner.run("copyItem", { text: "hi" })).ok).toBe(false);
    expect(t.errors.at(-1)?.code).toBe("E_STEP_UNSUPPORTED");
    const copyParts = vi.fn(async () => {});
    t.host.copyParts = copyParts;
    expect(await t.runner.run("copyItem", { text: "hi" })).toEqual({ ok: true });
    expect(copyParts).toHaveBeenCalledWith({ text: "hi", image: "" }, { action: "copyItem", step: 0 });
  });

  it("refuses writes outside /state and unknown views", async () => {
    const t = setup(app);
    expect((await t.runner.run("badSet")).ok).toBe(false);
    expect((await t.runner.run("badView")).ok).toBe(false);
    expect(t.errors.map((e) => e.code)).toEqual(["E_STEP", "E_STEP"]);
  });

  it("stops at the first failing step and reports which one", async () => {
    const t = setup(app);
    const outcome = await t.runner.run("failing");
    expect(outcome.ok).toBe(false);
    expect(t.toasts.map((x) => x[0])).toEqual(["before", "unexpected SQL: BOOM"]);
    expect(t.errors[0]).toMatchObject({ action: "failing", step: 1, code: "E_HOST" });
  });

  it("only refreshes queries that read a changed table; `refresh` re-runs one query", async () => {
    const t = setup(app);
    await t.runner.run("otherTable");
    expect(t.query).not.toHaveBeenCalled();
    await t.runner.run("refresher");
    expect(t.query.mock.calls.map((c) => (c[0] as QueryDef).name)).toEqual(["today"]);
  });

  it("a host's own write re-runs the queries on its tables with the app's state (a search stays filtered)", async () => {
    const search: QueryDef = {
      name: "search",
      sql: "SELECT * FROM sets WHERE reps >= :min",
      tables: ["sets"],
      params: { min: { p: "/state/min" } },
      columns: ["id", "reps", "at"],
    };
    const t = setup({ ...pushups, state: { min: 10 }, queries: [...pushups.queries, search] });
    t.store.set("/state/min", 25);
    await t.runner.refreshTables(["sets"]);
    const calls = t.query.mock.calls.map((c) => [(c[0] as QueryDef).name, c[1]]);
    expect(calls).toEqual(expect.arrayContaining([["search", { min: 25 }]]));
    expect(calls.map((c) => c[0]).sort()).toEqual(["per_hour", "recent", "search", "today"]);
  });
});

describe("helpers", () => {
  it("coerceArg handles the declared types", () => {
    expect(coerceArg("n", { type: "int", required: true }, "12")).toBe(12);
    expect(coerceArg("n", { type: "real", required: true }, "1.5")).toBe(1.5);
    expect(coerceArg("b", { type: "bool", required: true }, "false")).toBe(false);
    expect(coerceArg("s", { type: "text", required: false }, 5)).toBe("5");
    expect(coerceArg("s", { type: "text", required: false }, "")).toBeNull();
    expect(coerceArg("e", { type: "enum(low|high)", required: true }, "high")).toBe("high");
    expect(() => coerceArg("e", { type: "enum(low|high)", required: true }, "mid")).toThrow(/one of low, high/);
  });

  it("sqlParamNames ignores string literals and casts", () => {
    expect(sqlParamNames("INSERT INTO t (a, b) VALUES (:a, ':not') -- :b")).toEqual(["a"]);
    expect(sqlParamNames("SELECT x::int, :y")).toEqual(["y"]);
  });

  it("the store rejects UI writes outside /state", () => {
    const rejected: string[] = [];
    const store = createEneeStore(initialModel(pushups), { onRejectedWrite: (p) => rejected.push(p) });
    store.set("/q/today", []);
    store.update({ "/settings/daily_goal": 1, "/state/draft/reps": 3 });
    expect(rejected).toEqual(["/q/today", "/settings/daily_goal"]);
    expect(store.get("/state/draft/reps")).toBe(3);
    expect(store.get("/settings/daily_goal")).toBe(100);
  });
});
