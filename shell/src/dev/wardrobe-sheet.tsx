// The wardrobe sheet (T-062): every eye style and thing to wear on every form, to check how each sits.
// `pnpm --dir shell dev`, then http://localhost:5171/wardrobe.html. Only the dev server serves it.

import { StrictMode, useState } from "react";
import { createRoot } from "react-dom/client";
import { Character, COLORS, FINISHES, FORMS } from "../ui/Character";
import { EYE_STYLES, ITEMS, SLOTS, WARDROBE } from "../ui/wardrobe";
import characterCss from "../ui/character.css?raw";

/**
 * The sheet's characters as one PNG (a data: URL), `cell` pixels each, `cols` to a row: sharper than a screenshot for
 * checking fine detail. From the console: `await snapPng(320, 4)`.
 */
async function snapPng(cell = 300, cols = 5): Promise<string> {
  const css = `${characterCss}
*{animation:none!important}`;
  const cells = [...document.querySelectorAll<HTMLElement>(".ch")];
  const lab = 22;
  const cv = document.createElement("canvas");
  cv.width = cols * cell;
  cv.height = Math.ceil(cells.length / cols) * (cell + lab);
  const g = cv.getContext("2d")!;
  g.fillStyle = getComputedStyle(document.querySelector("#root > div")!).backgroundColor;
  g.fillRect(0, 0, cv.width, cv.height);
  for (const [i, div] of cells.entries()) {
    const svg = div.querySelector("svg")!.cloneNode(true) as SVGSVGElement;
    svg.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    svg.setAttribute("width", String(cell));
    svg.setAttribute("height", String(cell));
    svg.setAttribute("class", div.className);
    const style = document.createElementNS("http://www.w3.org/2000/svg", "style");
    style.textContent = `${css}
svg{--ch:${div.style.getPropertyValue("--ch")}}`;
    svg.prepend(style);
    const img = new Image();
    img.src = `data:image/svg+xml;charset=utf-8,${encodeURIComponent(new XMLSerializer().serializeToString(svg))}`;
    await img.decode();
    const x = (i % cols) * cell;
    const y = Math.floor(i / cols) * (cell + lab);
    g.drawImage(img, x, y, cell, cell);
    g.fillStyle = "#888";
    g.font = "14px system-ui";
    g.fillText((div.parentElement as HTMLElement).dataset.cell ?? "", x + 8, y + cell + 15);
  }
  return cv.toDataURL("image/png");
}
(window as unknown as { snapPng: typeof snapPng }).snapPng = snapPng;

type Pin = [number, number, number, number];

function Pins({ form, size }: { form: string; size: number }) {
  const pins = WARDROBE.forms[form]!.pins;
  const marks: [string, Pin][] = [];
  for (const [name, p] of Object.entries(pins)) {
    if (typeof p[0] === "number") marks.push([name, p as Pin]);
    else (p as Pin[]).forEach((q, i) => marks.push([`${name}${i}`, q]));
  }
  return (
    <svg viewBox="0 0 200 200" width={size} height={size} style={{ position: "absolute", inset: 0, pointerEvents: "none", overflow: "visible" }}>
      {marks.map(([name, [x, y, r, s]]) => (
        <g key={name} transform={`translate(${x} ${y}) rotate(${r}) scale(${s})`}>
          <path d="M-14 0H14M0 -6V6" stroke="#00e5ff" strokeWidth="0.8" />
          <text x="15" y="2" fontSize="5" fill="#00e5ff">
            {name}
          </text>
        </g>
      ))}
    </svg>
  );
}

function Sheet() {
  const q = new URLSearchParams(location.search);
  const [color, setColor] = useState(q.get("color") ?? "orange");
  const [finish, setFinish] = useState(q.get("finish") ?? "soft");
  const [eyes, setEyes] = useState(q.get("eyes") ?? "classic");
  const [rows, setRows] = useState(q.get("rows") ?? "head");
  const [pins, setPins] = useState(q.has("pins"));
  const [light, setLight] = useState(q.has("light"));
  const [size, setSize] = useState(Number(q.get("size") ?? 150));
  const [only, setOnly] = useState(q.get("only") ?? "");
  const [also, setAlso] = useState(q.get("also") ?? "");
  const list =
    rows === "eyes"
      ? EYE_STYLES.map((e) => ({ id: e.id, label: e.label, eyes: e.id, wear: [] as string[] }))
      : rows === "all"
        ? ITEMS.map((i) => ({ id: i.id, label: `${i.slot}: ${i.label}`, eyes, wear: [i.id] }))
        : ITEMS.filter((i) => i.slot === rows).map((i) => ({ id: i.id, label: i.label, eyes, wear: [i.id] }));
  const shown = list.filter((r) => !only || only.split(",").includes(r.id));
  // Whole outfits: `?fits=form:colour:finish:eyes:item+item;…`.
  const fits = (q.get("fits") ?? "").split(";").filter(Boolean).map((f) => f.split(":"));
  const forms = FORMS.filter((f) => !q.get("forms") || q.get("forms")!.split(",").includes(f.id));
  const extra = also.split(",").filter(Boolean);
  const bg = light ? "#f2f0eb" : "#0e0f12";
  return (
    <div style={{ background: bg, color: light ? "#222" : "#ddd", minHeight: "100vh", padding: 16, font: "13px system-ui" }}>
      {/* Still, so every cell can be compared. */}
      <style>{".ch, .ch * { animation: none !important; }"}</style>
      <div style={{ display: "flex", gap: 10, flexWrap: "wrap", marginBottom: 12, position: "sticky", top: 0, background: bg, padding: "8px 0", zIndex: 2 }}>
        <select value={rows} onChange={(e) => setRows(e.target.value)}>
          <option value="eyes">eyes</option>
          {SLOTS.map((s) => (
            <option key={s.id} value={s.id}>
              {s.id}
            </option>
          ))}
          <option value="all">everything</option>
        </select>
        <select value={color} onChange={(e) => setColor(e.target.value)}>
          {Object.keys(COLORS).map((c) => (
            <option key={c}>{c}</option>
          ))}
        </select>
        <select value={finish} onChange={(e) => setFinish(e.target.value)}>
          {FINISHES.map((f) => (
            <option key={f.id}>{f.id}</option>
          ))}
        </select>
        <select value={eyes} onChange={(e) => setEyes(e.target.value)}>
          {WARDROBE.eyes.map((e) => (
            <option key={e.id}>{e.id}</option>
          ))}
        </select>
        <input placeholder="only: ids" value={only} onChange={(e) => setOnly(e.target.value)} />
        <input placeholder="also wear: ids" value={also} onChange={(e) => setAlso(e.target.value)} />
        <label>
          <input type="checkbox" checked={pins} onChange={(e) => setPins(e.target.checked)} /> pins
        </label>
        <label>
          <input type="checkbox" checked={light} onChange={(e) => setLight(e.target.checked)} /> light
        </label>
        <input type="range" min={60} max={320} value={size} onChange={(e) => setSize(Number(e.target.value))} />
      </div>
      {fits.length ? (
        <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
          {fits.map(([form, c, fin, e, w], i) => (
            <div key={i} data-cell={w}>
              <Character form={form} color={c} finish={fin} eyes={e} wear={w ? w.split("+") : []} state="idle" size={size} />
            </div>
          ))}
        </div>
      ) : q.get("grid") ? (
        // One form, every row's option side by side.
        <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
          {shown.map((r) => (
            <div key={r.id} style={{ textAlign: "center" }}>
              <div style={{ position: "relative", width: size, height: size }}>
                <div data-cell={r.label}>
                  <Character form={q.get("grid")!} color={color} finish={finish} eyes={r.eyes} wear={[...r.wear, ...extra]} state="idle" size={size} />
                </div>
                {pins && <Pins form={q.get("grid")!} size={size} />}
              </div>
              <div style={{ opacity: 0.6, fontSize: 11 }}>{r.label}</div>
            </div>
          ))}
        </div>
      ) : (
      <table style={{ borderCollapse: "collapse" }}>
        <thead>
          <tr>
            <th />
            {forms.map((f) => (
              <th key={f.id} style={{ fontWeight: 500, opacity: 0.6 }}>
                {f.label}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {shown.map((r) => (
            <tr key={r.id} data-row={r.id}>
              <td style={{ paddingRight: 12, whiteSpace: "nowrap", opacity: 0.7 }}>{r.label}</td>
              {forms.map((f) => (
                <td key={f.id} style={{ padding: 4 }}>
                  <div style={{ position: "relative", width: size, height: size }}>
                    <div data-cell={`${r.label} · ${f.label}`}>
                      <Character form={f.id} color={color} finish={finish} eyes={r.eyes} wear={[...r.wear, ...extra]} state="idle" size={size} />
                    </div>
                    {pins && <Pins form={f.id} size={size} />}
                  </div>
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      )}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Sheet />
  </StrictMode>,
);
