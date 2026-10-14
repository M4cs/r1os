import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { phosphorIcons } from "@r1os/icons/vite-plugin";
import { fileURLToPath } from "node:url";

// r1os's website (docs/kanban/epics/R9 The website.md). `@shell` and `@companion` are the OS's own sources: the agents'
// characters are the shell's, and the R1's screens, look and fonts are the companion's, so the site stays in step.
export default defineConfig({
  plugins: [react(), phosphorIcons({ weights: ["bold", "regular", "fill"] })],
  resolve: {
    alias: {
      "@shell": fileURLToPath(new URL("../shell/src", import.meta.url)),
      "@companion": fileURLToPath(new URL("../companion/src", import.meta.url)),
    },
    dedupe: ["react", "react-dom", "motion"],
  },
  server: { port: 5180, strictPort: true },
  build: { outDir: "dist", emptyOutDir: true, target: "es2022" },
});
