import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";
import solid from "vite-plugin-solid";
import { TEXT_ID } from "./src/textId.ts";

/** Emits sw.js (from src/sw.js) listing this build's files; its version changes whenever any file does. */
function serviceWorker(): Plugin {
  return {
    name: "service-worker",
    apply: "build",
    generateBundle(_, bundle) {
      const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));
      const files = [...Object.keys(bundle).filter((f) => !f.endsWith(".map") && f !== "index.html"), ...readdirSync(here("public"))].sort();
      const version = createHash("sha256").update(files.join("\n")).update(readFileSync(here("index.html"))).digest("hex").slice(0, 16);
      const source = readFileSync(here("src/sw.js"), "utf8")
        .replace("__VERSION__", version)
        .replace("__SHELL__", JSON.stringify(files))
        .replace("__TEXT_ID__", TEXT_ID);
      this.emitFile({ type: "asset", fileName: "sw.js", source });
    },
  };
}

// Dev: Vite serves the client with HMR and proxies the server's routes to the Rust dev server.
const backend = process.env.BACKEND ?? "http://127.0.0.1:8080";

export default defineConfig({
  // "./" for static hosting under any subpath (GitHub Pages); routing is in the URL hash, so no server fallback.
  base: process.env.VITE_BASE ?? "/",
  plugins: [solid(), serviceWorker()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: { "/api": { target: backend, ws: true }, "/text": backend },
  },
  build: { target: "es2022", sourcemap: true },
});
