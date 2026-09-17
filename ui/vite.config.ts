import { defineConfig } from "vite";

// Tauri expects a fixed dev-server port (matches tauri.conf.json's devUrl)
// and a relative base so the built asset paths work when loaded from the
// webview's custom protocol rather than an http:// origin.
export default defineConfig({
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    outDir: "dist",
    minify: "esbuild",
    sourcemap: true,
  },
});
