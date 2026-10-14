import { describe, expect, it } from "vitest";

import { toSpec } from "./logic-view";

describe("a script's view", () => {
  it("becomes a flat spec of catalog components", () => {
    const { spec, problems } = toSpec(
      {
        type: "Stack",
        gap: "md",
        children: [
          { type: "Heading", text: "Board" },
          { type: "Row", children: [{ type: "Button", label: "Roll", press: "roll", args: { n: 2 } }] },
          { type: "Wheel", label: "Bet", min: 1, max: 10, bind: "bet" },
          { type: "List", items: [{ id: 1, name: "A" }], title: "name", press: "open", args: { id: "{id}" } },
        ],
      },
      "lv",
    );
    expect(problems).toEqual([]);
    const els = Object.values(spec!.elements);
    const button = els.find((e) => e.type === "Button") as unknown as { on: unknown; props: Record<string, unknown> };
    expect(button.props).toEqual({ label: "Roll" });
    expect(button.on).toEqual({ press: { action: "run", params: { name: "roll", args: { n: 2 } } } });
    const wheel = els.find((e) => e.type === "Wheel")!;
    expect(wheel.props.value).toEqual({ $bindState: "/state/bet" });
    const list = els.find((e) => e.type === "List")!;
    expect(list.props.title).toEqual({ i: "name" });
    expect(list.props.rowActions).toEqual([{ icon: "caret-right", label: "Open", action: "open", args: { id: { i: "id" } } }]);
    expect(spec!.elements[spec!.root]!.children).toHaveLength(4);
  });

  it("leaves out what isn't allowed, and says why", () => {
    const { spec, problems } = toSpec(
      { type: "Stack", children: [{ type: "Game", script: "x" }, { type: "Text" }, { type: "Text", text: "ok" }] },
      "lv",
    );
    expect(problems).toHaveLength(2);
    expect(problems[0]).toContain("Game");
    expect(problems[1]).toContain("Text");
    expect(spec!.elements[spec!.root]!.children).toHaveLength(1);
    expect(toSpec(null, "lv")).toEqual({ spec: null, problems: [] });
  });
});
