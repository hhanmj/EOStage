import fs from "node:fs";
import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { configureAtlasCdnForVite, createEoAtlasLocalCdnPlugin } from "./atlasCdnVite";

const renameTauriEntry = {
  name: "rename-tauri-entry-to-index",
  writeBundle() {
    const outputDirectory = path.resolve(import.meta.dirname, "dist-tauri");
    const source = path.join(outputDirectory, "tauri.html");
    const target = path.join(outputDirectory, "index.html");
    if (!fs.existsSync(source)) {
      throw new Error(`Tauri build did not emit its HTML entry: ${source}`);
    }
    if (fs.existsSync(target)) {
      fs.rmSync(target);
    }
    fs.renameSync(source, target);
  },
};

export default defineConfig(({ command, mode }) => {
  const { localPackAvailable, define } = configureAtlasCdnForVite(command, mode);
  return {
    plugins: [
      react(),
      tailwindcss(),
      createEoAtlasLocalCdnPlugin(localPackAvailable),
      renameTauriEntry,
    ],
    base: "./",
    root: import.meta.dirname,
    define,
    resolve: {
      alias: {
        "@": path.resolve(import.meta.dirname, "src"),
      },
    },
    build: {
      outDir: "dist-tauri",
      emptyOutDir: true,
      rollupOptions: {
        input: path.resolve(import.meta.dirname, "tauri.html"),
      },
    },
  };
});
