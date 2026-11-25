import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Two entry pages: the full-screen gallery overlay and the settings window.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Never watch the Rust build output: Windows locks files there while cargo
    // compiles, which crashes Vite with EBUSY.
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    target: "chrome105", // WebView2 is evergreen Chromium
    minify: "esbuild",
    cssMinify: true,
    rollupOptions: {
      input: {
        main: "index.html",
        settings: "settings.html",
        transition: "transition.html",
      },
    },
  },
});
