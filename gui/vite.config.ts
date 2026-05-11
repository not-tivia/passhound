import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// https://tauri.app/v1/api/config/
export default defineConfig({
  plugins: [react()],
  // Tauri expects a fixed port, fail if unavailable
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    hmr: {
      protocol: "ws",
      host: "localhost",
      port: 1421,
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
});
