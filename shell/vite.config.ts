import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { phosphorIcons } from "@r1os/icons/vite-plugin";

// `pnpm dev` serves the shell with hot reload and proxies r1d's endpoints to a simulator
// (tools/sim.sh) or to the R1 through `adb forward tcp:7171 tcp:7171`.
const r1d = "http://127.0.0.1:7171";

export default defineConfig({
  // Phosphor SVGs at /icons/<weight>/<name>.svg: bold (apps) and regular, fetched on first use.
  plugins: [react(), phosphorIcons()],
  base: "./",
  server: {
    port: 5171,
    proxy: {
      "/api": r1d,
      "/harness": r1d,
      "/ws": { target: r1d, ws: true },
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    // index.html: the web shell, now only the desktop simulator's UI (on the R1 every screen is native,
    // ADR-0017). app.html: one mini-app, in the native shell's per-app WebView.
    rollupOptions: { input: { index: "index.html", app: "app.html" } },
  },
});
