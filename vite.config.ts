import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri expects a fixed port and fails if it's taken.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1431, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
  build: { target: "safari16", outDir: "dist", emptyOutDir: true },
});
