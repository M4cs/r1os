import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { phosphorIcons } from "@r1os/icons/vite-plugin";
import { fileURLToPath } from "node:url";

// The companion's window (Tauri loads it from here in dev, from dist/ when built). `@shell` is the web shell's source:
// the agents' characters are drawn by the same component the simulator uses.
export default defineConfig({
  // Bold and regular, and fill for what's chosen (the page you're on, a widget or pin that's on, play).
  plugins: [react(), phosphorIcons({ weights: ["bold", "regular", "fill"] })],
  base: "./",
  resolve: {
    alias: { "@shell": fileURLToPath(new URL("../shell/src", import.meta.url)) },
  },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true, target: "es2022" },
});
