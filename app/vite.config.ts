import { defineConfig } from "vite";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Tauri expects a fixed port. In a plain browser the app defaults to the
// in-browser engine (`/pdsim-web.js`, see `npm run build:web`); `?backend=ws`
// (or `?ws=`) instead talks to `pdsim serve` (see README).
//
// `--mode web` builds the static site: relative URLs (so it works from a
// subpath such as GitHub Pages), no Tauri code, and the Emscripten output
// copied next to the page.
const emscripten = "../target/wasm32-unknown-emscripten/release";

export default defineConfig(({ mode }) => {
  const web = mode === "web";
  return {
    clearScreen: false,
    base: web ? "./" : "/",
    server: { port: 1420, strictPort: true, host: "127.0.0.1" },
    worker: { format: "es" as const },
    build: {
      target: "es2022",
      outDir: "dist",
      emptyOutDir: true,
      rollupOptions: web ? { external: [/^@tauri-apps\//] } : {},
    },
    plugins: web
      ? [
          {
            name: "pdsim-web-assets",
            generateBundle() {
              // The emitted wasm is `pdsim_web.wasm`; the .js looks for that name.
              for (const f of ["pdsim-web.js", "pdsim_web.wasm"]) {
                const source = readFileSync(fileURLToPath(new URL(`${emscripten}/${f}`, import.meta.url)));
                this.emitFile({ type: "asset", fileName: f, source });
              }
            },
          },
        ]
      : [],
  };
});
