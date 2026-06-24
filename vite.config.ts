import { defineConfig } from "vite";

// The frontend lives in src/; the production build lands in ../dist, which
// tauri.conf.json references as frontendDist.
export default defineConfig({
  root: "src",
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2021",
  },
});
