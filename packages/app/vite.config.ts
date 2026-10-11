import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

// Dev: Vite serves the client with HMR and proxies the server's routes to the Rust dev server.
const backend = process.env.BACKEND ?? "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [solid()],
  server: {
    port: 5173,
    strictPort: true,
    proxy: { "/api": { target: backend, ws: true }, "/text": backend },
  },
  build: { target: "es2022", sourcemap: true },
});
