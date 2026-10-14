// Phosphor icons (MIT, @phosphor-icons/core 2.1.1; see ../THIRD_PARTY.md) for the shell and the app runtime.
//
// The SVG files are served as static assets at `<base>/<weight>/<name>.svg` (see ../vite-plugin.ts) and fetched on
// first use, so a screen pays only for the icons it shows and the startup bundle carries no icon data. The tagged
// catalog (≈ 450 KB) is a separate lazy import (./catalog.ts) for pickers and search.

export type IconWeight = "regular" | "bold" | "fill" | "duotone";

export const WEIGHTS: readonly IconWeight[] = ["regular", "bold", "fill", "duotone"];

/** Where SVG markup comes from. The default fetches the static assets; tests read the files directly. */
export type IconSource = (name: string, weight: IconWeight) => Promise<string | null>;

const NAME = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

let base = "/icons";
let source: IconSource = async (name, weight) => {
  const res = await fetch(`${base}/${weight}/${name}.svg`);
  return res.ok ? res.text() : null;
};

export function configureIcons(options: { base?: string; source?: IconSource }): void {
  if (options.base !== undefined) base = options.base.replace(/\/$/, "");
  if (options.source) source = options.source;
  cache.clear();
  pending.clear();
}

const cache = new Map<string, string | null>();
const pending = new Map<string, Promise<string | null>>();
const key = (name: string, weight: IconWeight) => `${weight}:${name}`;

/** The icon's SVG markup (`fill="currentColor"`, 256×256 viewBox), or `null` for an unknown name. */
export function loadIcon(name: string, weight: IconWeight = "regular"): Promise<string | null> {
  if (!NAME.test(name)) return Promise.resolve(null);
  const k = key(name, weight);
  if (cache.has(k)) return Promise.resolve(cache.get(k) ?? null);
  let p = pending.get(k);
  if (!p) {
    p = source(name, weight)
      .then((svg) => (svg && svg.trimStart().startsWith("<svg") ? svg : null))
      .catch(() => null)
      .then((svg) => {
        cache.set(k, svg);
        pending.delete(k);
        return svg;
      });
    pending.set(k, p);
  }
  return p;
}

/** The SVG if it has been loaded (`null` if it's known to be missing, `undefined` if not loaded yet). */
export function iconSvgSync(name: string, weight: IconWeight = "regular"): string | null | undefined {
  return cache.get(key(name, weight));
}

export function preloadIcons(names: Iterable<string>, weight: IconWeight = "regular"): Promise<void> {
  return Promise.all([...names].map((n) => loadIcon(n, weight))).then(() => undefined);
}
