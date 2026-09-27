import { defineConfig } from "vite";
import { resolve } from "node:path";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const host = process.env.TAURI_DEV_HOST;

// Two pages: the settings window (index.html) and the overlay (overlay.html).
export default defineConfig(async () => ({
  plugins: [svelte()],

  // Tauri expects a fixed port, fail if that port is not available.
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1431 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },

  envPrefix: ["VITE_", "TAURI_ENV_*"],

  build: {
    target: "chrome105", // WebView2
    minify: !process.env.TAURI_ENV_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        overlay: resolve(__dirname, "overlay.html"),
      },
    },
  },
}));
