// Serves Phosphor's SVG files at `<base>/<weight>/<name>.svg`: from node_modules in dev, emitted as static assets in
// builds. Used by shell/vite.config.ts. r1os: builds emit only the weights asked for (default bold + regular),
// not all four: every file is pushed to the R1 on deploy.

import { readdir, readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";

import type { Plugin } from "vite";

const WEIGHTS = ["regular", "bold", "fill", "duotone"] as const;
type Weight = (typeof WEIGHTS)[number];

/** The directory holding Phosphor's `assets/<weight>/` folders. */
export function phosphorAssetsDir(): string {
  const require = createRequire(import.meta.url);
  return path.join(path.dirname(require.resolve("@phosphor-icons/core")), "..", "assets");
}

/** `drop` + `fill` → `fill/drop-fill.svg`; regular files have no suffix. */
export function phosphorFile(weight: Weight, name: string): string {
  return path.join(weight, weight === "regular" ? `${name}.svg` : `${name}-${weight}.svg`);
}

export function phosphorIcons({
  base = "/icons",
  weights = ["bold", "regular"],
}: { base?: string; weights?: readonly Weight[] } = {}): Plugin {
  const assets = phosphorAssetsDir();
  const route = new RegExp(`^/(${WEIGHTS.join("|")})/([a-z0-9]+(?:-[a-z0-9]+)*)\\.svg$`);
  return {
    name: "r1os-phosphor-icons",
    configureServer(server) {
      server.middlewares.use(base, (req, res, next) => {
        const m = route.exec(req.url ?? "");
        if (!m) return next();
        readFile(path.join(assets, phosphorFile(m[1] as Weight, m[2] as string))).then(
          (svg) => {
            res.setHeader("Content-Type", "image/svg+xml");
            res.setHeader("Cache-Control", "max-age=31536000, immutable");
            res.end(svg);
          },
          () => {
            res.statusCode = 404;
            res.end();
          },
        );
      });
    },
    async generateBundle() {
      for (const weight of weights) {
        for (const file of await readdir(path.join(assets, weight))) {
          const name = weight === "regular" ? file.slice(0, -4) : file.slice(0, -(weight.length + 5));
          this.emitFile({
            type: "asset",
            fileName: `${base.replace(/^\//, "")}/${weight}/${name}.svg`,
            source: await readFile(path.join(assets, weight, file)),
          });
        }
      }
    },
  };
}
