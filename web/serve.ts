// Serves the built site (`web/dist`) where it's hosted: `bun web/serve.ts`, on $PORT (railway.json at the repo's root).
// Files are served as they are. A path with no file and no extension gets index.html, since the site routes on the
// client (`/docs/apps` has no file of its own); a missing file with an extension is a 404, not a page.
import { stat } from "node:fs/promises";
import { extname, join, resolve, sep } from "node:path";

const root = resolve(import.meta.dir, "dist");
const index = join(root, "index.html");
const always = {
  "X-Content-Type-Options": "nosniff",
  "Referrer-Policy": "strict-origin-when-cross-origin",
  "X-Frame-Options": "DENY",
};

async function isFile(path: string): Promise<boolean> {
  return (await stat(path).catch(() => null))?.isFile() ?? false;
}

const server = Bun.serve({
  port: Number(process.env.PORT ?? 4180),
  async fetch(req) {
    if (req.method !== "GET" && req.method !== "HEAD") return new Response(null, { status: 405, headers: { ...always, Allow: "GET, HEAD" } });
    let path: string;
    try {
      path = decodeURIComponent(new URL(req.url).pathname);
    } catch {
      return new Response("Bad request", { status: 400, headers: always });
    }
    const file = resolve(root, `.${path}`);
    if (file !== root && !file.startsWith(root + sep)) return new Response("Not found", { status: 404, headers: always });
    if (await isFile(file)) {
      // Vite's hashed assets never change; everything else is checked again each time.
      const cache = path.startsWith("/assets/") ? "public, max-age=31536000, immutable" : "no-cache";
      return new Response(Bun.file(file), { headers: { ...always, "Cache-Control": cache } });
    }
    if (extname(path)) return new Response("Not found", { status: 404, headers: always });
    return new Response(Bun.file(index), { headers: { ...always, "Cache-Control": "no-cache", "Content-Type": "text/html; charset=utf-8" } });
  },
});

console.log(`r1os's site on :${server.port}, from ${root}`);
