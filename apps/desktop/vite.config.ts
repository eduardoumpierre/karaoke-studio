import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// O Tauri espera o servidor de desenvolvimento em uma porta fixa (ver tauri.conf.json).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: "127.0.0.1",
    // O Rust compila em src-tauri/target; vigiar essa pasta trava o Vite no Windows.
    watch: { ignored: ["**/src-tauri/**"] },
  },
  test: { environment: "node" },
});
