import { describe, expect, it } from "vitest";

import { thumbSrc } from "./list";

describe("thumbSrc", () => {
  it("keeps the app's own image files and inline images, nothing else", () => {
    const hash = "0f".repeat(32);
    expect(thumbSrc(`files/${hash}.png`)).toBe(`files/${hash}.png`);
    expect(thumbSrc(`files/${hash}.thumb.png`)).toBe(`files/${hash}.thumb.png`);
    expect(thumbSrc("data:image/png;base64,iVBORw0KGgo=")).toBe("data:image/png;base64,iVBORw0KGgo=");
    for (const bad of [
      "",
      "https://example.com/a.png",
      "//example.com/a.png",
      `/apps/other/files/${hash}.png`,
      `files/../${hash}.png`,
      `files/${hash.toUpperCase()}.png`,
      "files/abc.png",
      "data:image/svg+xml;base64,PHN2Zz4=",
      "data:text/html;base64,PGI+",
      "javascript:alert(1)",
    ]) {
      expect(thumbSrc(bad)).toBe("");
    }
  });
});
