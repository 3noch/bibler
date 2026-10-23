import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";
import solid from "vite-plugin-solid";
import { TEXT_ID } from "./src/textId.ts";

/** Emits sw.js (from src/sw.js) listing this build's files; its version changes whenever any file does. That version
 * is also the client's: it changes only when the client does (file names are content hashes), never for a commit
 * that changes only the server. The page reads its own from `<meta name="build-client">` in index.html; the servers
 * read the one they serve from `client-version.txt` (`api::CLIENT_VERSION_FILE`, so they never look inside HTML) and
 * send it in each sync `ready` and `pong` (the offline-only build fetches the file), so an open app knows when a new
 * client is out. */
function serviceWorker(): Plugin {
  return {
    name: "service-worker",
    apply: "build",
    enforce: "post", // after Vite emits index.html
    generateBundle(_, bundle) {
      const here = (p: string) => fileURLToPath(new URL(p, import.meta.url));
      const files = [...Object.keys(bundle).filter((f) => !f.endsWith(".map") && f !== "index.html"), ...readdirSync(here("public"))].sort();
      const version = createHash("sha256").update(files.join("\n")).update(readFileSync(here("index.html"))).digest("hex").slice(0, 16);
      const source = readFileSync(here("src/sw.js"), "utf8")
        .replace("__VERSION__", version)
        .replace("__SHELL__", JSON.stringify(files))
        .replace("__TEXT_ID__", TEXT_ID);
      this.emitFile({ type: "asset", fileName: "sw.js", source });
      this.emitFile({ type: "asset", fileName: "client-version.txt", source: version + "\n" }); // not in the shell
      const html = bundle["index.html"];
      if (html?.type === "asset") html.source = String(html.source).replace("</head>", `<meta name="build-client" content="${version}">\n</head>`);
    },
  };
}

/** The commit this is built from (shown in the menu): BUILD_COMMIT/BUILD_COMMITTED_AT if set, else git; empty when
 * neither is there. The servers do the same in crates/api/build.rs. */
function commit(): { commit: string; committedAt: string } {
  const git = (...args: string[]) => {
    try {
      return execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
    } catch {
      return "";
    }
  };
  return {
    commit: process.env.BUILD_COMMIT ?? git("rev-parse", "HEAD"),
    committedAt: process.env.BUILD_COMMITTED_AT ?? git("log", "-1", "--format=%cI"),
  };
}

/** Puts the commit in index.html (`<meta name="build-commit">`, `build-committed-at`), not in the bundle: index.html
 * is revalidated on every load anyway and names the bundle that runs, so the page's own stamp is always the right
 * one, and a commit that changes only the server leaves every hashed file (and the service worker's version, which
 * hashes the source index.html) as it was, so browsers keep their cached copies. */
function buildStamp(): Plugin {
  const { commit: c, committedAt } = commit();
  return {
    name: "build-stamp",
    transformIndexHtml: () => [
      { tag: "meta", attrs: { name: "build-commit", content: c }, injectTo: "head" },
      { tag: "meta", attrs: { name: "build-committed-at", content: committedAt }, injectTo: "head" },
    ],
  };
}

// Dev: Vite serves the client with HMR and proxies the server's routes to the Rust dev server.
const backend = process.env.BACKEND ?? "http://127.0.0.1:8080";

export default defineConfig({
  // "./" for static hosting under any subpath (GitHub Pages); routing is in the URL hash, so no server fallback.
  base: process.env.VITE_BASE ?? "/",
  plugins: [solid(), serviceWorker(), buildStamp()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: { "/api": { target: backend, ws: true }, "/text": backend },
  },
  build: { target: "es2022", sourcemap: true },
});
