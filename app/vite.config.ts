import { defineConfig } from "vite";

// Tauri expects a fixed port; the same dev server also works in a plain
// browser against `pdsim serve` (see README).
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
