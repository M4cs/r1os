// The glance screen's clock faces (host/…/shell/Clocks.kt), and the R1's fonts (ui/Theme.kt FAMILIES): the same
// typefaces, copied from the R1 (fonts/r1), at the R1's sizes in dp.

import type { CSSProperties } from "react";
import type { Tokens } from "./paint";

/** The look's fonts, as the R1 has them. */
export const FONTS: { id: string; label: string; css: string }[] = [
  { id: "rubik", label: "rubik", css: "Rubik" },
  { id: "roboto", label: "roboto", css: "'R1 Roboto'" },
  { id: "flex", label: "flex", css: "'R1 Roboto Flex'" },
  { id: "source", label: "source", css: "'R1 Source Sans'" },
  { id: "serif", label: "serif", css: "'R1 Noto Serif'" },
  { id: "mono", label: "mono", css: "'R1 Droid Sans Mono'" },
  { id: "typewriter", label: "typewriter", css: "'R1 Cutive Mono'" },
  { id: "casual", label: "casual", css: "'R1 Coming Soon'" },
  { id: "script", label: "script", css: "'R1 Dancing Script'" },
  { id: "smallcaps", label: "small caps", css: "'R1 Carrois Gothic SC'" },
];
export function fontOf(id?: string): string {
  return FONTS.find((f) => f.id === id)?.css ?? "Rubik";
}

export const CLOCKS = ["bold", "light", "condensed", "stacked", "serif", "mono", "words", "dial"] as const;

/** The R1's 12-hour time, as its clocks write it: `h:mm`, no am or pm. */
export function hm(d: Date): string {
  return `${d.getHours() % 12 || 12}:${String(d.getMinutes()).padStart(2, "0")}`;
}

const NAMES = ["twelve", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven"];
const NUMBERS: Record<number, string> = { 5: "five", 10: "ten", 20: "twenty", 25: "twenty-five" };

/** The words face: to the nearest five minutes ("twenty past eight", "quarter to nine", "noon"). */
export function words(d: Date): string {
  let h = d.getHours();
  let m = Math.round(d.getMinutes() / 5) * 5;
  if (m === 60) {
    m = 0;
    h = (h + 1) % 24;
  }
  const name = (x: number) => (x === 0 ? "midnight" : x === 12 ? "noon" : NAMES[x % 12]);
  const next = (h + 1) % 24;
  if (m === 0) return h === 0 || h === 12 ? name(h) : `${NAMES[h % 12]} o'clock`;
  if (m === 15) return `quarter past ${name(h)}`;
  if (m === 30) return `half past ${name(h)}`;
  if (m === 45) return `quarter to ${name(next)}`;
  if (m <= 25) return `${NUMBERS[m]} past ${name(h)}`;
  return `${NUMBERS[60 - m]} to ${name(next)}`;
}

/** One clock face, in dp. */
export function ClockFace({ face, now, font, t, accent }: { face?: string; now: Date; font: string; t: Tokens; accent: string }) {
  const base: CSSProperties = { color: t.fg, fontVariantNumeric: "tabular-nums", lineHeight: "normal", whiteSpace: "pre" };
  switch (face) {
    case "light":
      return <div style={{ ...base, fontFamily: "'R1 Roboto Flex'", fontVariationSettings: "'wght' 200", fontWeight: 200, fontSize: 78 }}>{hm(now)}</div>;
    case "condensed":
      return <div style={{ ...base, fontFamily: "'R1 Roboto Flex'", fontVariationSettings: "'wdth' 25, 'wght' 720", fontSize: 100, letterSpacing: "-0.01em" }}>{hm(now)}</div>;
    case "stacked": {
      const hh = String(now.getHours() % 12 || 12).padStart(2, "0");
      const mm = String(now.getMinutes()).padStart(2, "0");
      return <div style={{ ...base, fontFamily: font, fontWeight: 700, fontSize: 82, lineHeight: 1.0 }}>{`${hh}\n${mm}`}</div>;
    }
    case "serif":
      return <div style={{ ...base, fontFamily: "'R1 Noto Serif'", fontWeight: 400, fontSize: 70 }}>{hm(now)}</div>;
    case "mono":
      return <div style={{ ...base, fontFamily: "'R1 Droid Sans Mono'", fontWeight: 400, fontSize: 62 }}>{hm(now)}</div>;
    case "words":
      return <div style={{ ...base, fontFamily: font, fontWeight: 700, fontSize: 40, lineHeight: 1.09, whiteSpace: "normal", maxWidth: 320, textTransform: "lowercase" }}>{words(now)}</div>;
    case "dial":
      return <Dial now={now} t={t} accent={accent} />;
    default:
      return <div style={{ ...base, fontFamily: font, fontWeight: 700, fontSize: 64, letterSpacing: "-0.01em" }}>{hm(now)}</div>;
  }
}

/** The dial: twelve marks (the quarters longer), the hour hand in the text's colour, the minutes' in the accent. */
function Dial({ now, t, accent }: { now: Date; t: Tokens; accent: string }) {
  const r = 66;
  const at = (turns: number, len: number): [number, number] => [r + len * Math.sin(turns * 2 * Math.PI), r - len * Math.cos(turns * 2 * Math.PI)];
  const h = ((now.getHours() % 12) + now.getMinutes() / 60) / 12;
  const m = now.getMinutes() / 60;
  return (
    <svg width={132} height={132} viewBox="0 0 132 132" style={{ display: "block" }}>
      {Array.from({ length: 12 }, (_, i) => {
        const big = i % 3 === 0;
        const [x1, y1] = at(i / 12, r - (big ? 14 : 9));
        const [x2, y2] = at(i / 12, r - 3);
        return <line key={i} x1={x1} y1={y1} x2={x2} y2={y2} stroke={big ? t.fg : t.muted} strokeWidth={big ? 4 : 2.5} strokeLinecap="round" />;
      })}
      <line x1={r} y1={r} x2={at(h, 0.5 * r)[0]} y2={at(h, 0.5 * r)[1]} stroke={t.fg} strokeWidth={7} strokeLinecap="round" />
      <line x1={r} y1={r} x2={at(m, 0.78 * r)[0]} y2={at(m, 0.78 * r)[1]} stroke={accent} strokeWidth={4.5} strokeLinecap="round" />
      <circle cx={r} cy={r} r={5} fill={accent} />
    </svg>
  );
}
