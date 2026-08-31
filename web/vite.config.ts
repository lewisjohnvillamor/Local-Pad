import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Dev mode proxies API calls to a locally running `localpad serve`.
export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:7843",
      "/setup/localpad-ca.crt": "http://127.0.0.1:7843",
      "/ws": { target: "ws://127.0.0.1:7844", ws: true },
    },
  },
});
