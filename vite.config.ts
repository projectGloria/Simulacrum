import { defineConfig } from "vite";

// Fixed dev port (must match devUrl in tauri.conf.json). 1430, not the Tauri default 1420,
// so this can run next to other Tauri projects on their default port.
export default defineConfig({
  clearScreen: false,
  server: { port: 1430, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
