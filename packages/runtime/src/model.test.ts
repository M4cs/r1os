import { describe, expect, it } from "vitest";

import pushupsJson from "../../../fixtures/apps/pushups.compiled.json";
import { keptStateUpdates } from "./model";
import type { CompiledApp } from "./types";

const pushups = pushupsJson as unknown as CompiledApp;

describe("keptStateUpdates", () => {
  const notes: CompiledApp = {
    ...pushups,
    state: { draft: { text: "" }, tab: "all", tags: [], picked: null },
    persist: ["draft", "tab", "tags", "picked"],
  };

  it("restores the kept keys only", () => {
    expect(keptStateUpdates(notes, { draft: { text: "milk" }, tab: "done", scroll: 40 })).toEqual({
      "/state/draft": { text: "milk" },
      "/state/tab": "done",
    });
    expect(keptStateUpdates({ ...notes, persist: undefined }, { tab: "done" })).toEqual({});
    expect(keptStateUpdates(notes, undefined)).toEqual({});
  });

  it("drops values whose shape no longer matches the initial value", () => {
    expect(keptStateUpdates(notes, { tab: ["done"], tags: "x", draft: "milk" })).toEqual({});
    expect(keptStateUpdates(notes, { tab: null, tags: ["a"], picked: { id: 3 } })).toEqual({
      "/state/tab": null,
      "/state/tags": ["a"],
      "/state/picked": { id: 3 },
    });
  });
});
