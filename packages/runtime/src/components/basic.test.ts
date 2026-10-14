import { describe, expect, it } from "vitest";

import { clockText, gaugeRatio } from "./basic";

describe("clockText", () => {
  it("reads m:ss under an hour, h:mm:ss under a day, and days with hours beyond", () => {
    expect(clockText(0)).toBe("0:00");
    expect(clockText(9)).toBe("0:09");
    expect(clockText(25 * 60)).toBe("25:00");
    expect(clockText(59 * 60 + 59.9)).toBe("59:59");
    expect(clockText(3600)).toBe("1:00:00");
    expect(clockText(2 * 3600 + 5 * 60 + 7)).toBe("2:05:07");
    expect(clockText(86_400 * 3 + 3600 * 4 + 59)).toBe("3d 4h");
    expect(clockText(-5)).toBe("0:00");
  });
});

describe("gaugeRatio", () => {
  it("fills value of max, 100 by default, clamped, and reads numbers given as text", () => {
    expect(gaugeRatio(37, undefined)).toBe(0.37);
    expect(gaugeRatio(3, 4)).toBe(0.75);
    expect(gaugeRatio("1,250", 2500)).toBe(0.5);
    expect(gaugeRatio(150, 100)).toBe(1);
    expect(gaugeRatio(-3, 100)).toBe(0);
    expect(gaugeRatio(null, 100)).toBe(0);
    expect(gaugeRatio("n/a", 100)).toBe(0);
    expect(gaugeRatio(5, 0)).toBe(0.05);
  });
});
