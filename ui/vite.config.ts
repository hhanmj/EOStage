import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { configureAtlasCdnForVite, createEoAtlasLocalCdnPlugin } from "./atlasCdnVite";

// base: "./" so a packaged Tauri WebView can resolve the production assets
// without depending on an external HTTP server.
export default defineConfig(({ command, mode }) => {
  const { localPackAvailable, define } = configureAtlasCdnForVite(command, mode);
  return {
    plugins: [react(), tailwindcss(), createEoAtlasLocalCdnPlugin(localPackAvailable)],
    base: "./",
    define,
    resolve: {
      alias: {
        "@": path.resolve(import.meta.dirname, "src"),
      },
    },
    server: {
      port: 5173,
      strictPort: true,
    },
  };
});
