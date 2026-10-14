import { readFile } from "node:fs/promises";
import path from "node:path";

import { beforeAll, describe, expect, it } from "vitest";

import { phosphorAssetsDir, phosphorFile } from "../vite-plugin";
import { loadIconCatalog } from "./catalog";
import { configureIcons, iconSvgSync, loadIcon } from "./icons";
import { appTile } from "./tile";

beforeAll(() => {
  const assets = phosphorAssetsDir();
  configureIcons({ source: (name, weight) => readFile(path.join(assets, phosphorFile(weight, name)), "utf8").catch(() => null) });
});

describe("phosphor icons", () => {
  it("has the tagged catalog", async () => {
    const icons = await loadIconCatalog();
    expect(icons.length).toBeGreaterThan(1500);
    expect(icons.find((i) => i.name === "barbell")?.tags).toContain("fitness");
  });

  it("loads one icon per weight on demand and caches it", async () => {
    expect(iconSvgSync("drop", "fill")).toBeUndefined();
    const svg = await loadIcon("drop", "fill");
    expect(svg).toMatch(/^<svg[^>]*viewBox="0 0 256 256"[^>]*fill="currentColor"/);
    expect(iconSvgSync("drop", "fill")).toBe(svg);
    expect(await loadIcon("drop", "duotone")).toContain('opacity="0.2"');
    expect(await loadIcon("drop")).not.toBe(svg);
  });

  it("rejects unknown and malformed names", async () => {
    expect(await loadIcon("no-such-icon", "fill")).toBeNull();
    expect(iconSvgSync("no-such-icon", "fill")).toBeNull();
    expect(await loadIcon("../secret", "fill")).toBeNull();
  });
});

describe("app tiles", () => {
  it("are stable per app and differ between apps", () => {
    expect(appTile("pushups")).toEqual(appTile("pushups"));
    const ids = ["pushups", "expenses", "water", "habits", "notes", "sleep", "coffee", "reading"];
    const looks = new Set(ids.map((id) => JSON.stringify(appTile(id).style)));
    expect(looks.size).toBe(ids.length);
  });

  it("lean toward the app's accent", () => {
    for (const id of ["a", "b", "c", "d"]) {
      const hue = appTile(id, "blue").hue;
      expect(Math.abs(hue - 255)).toBeLessThanOrEqual(34);
    }
  });
});
