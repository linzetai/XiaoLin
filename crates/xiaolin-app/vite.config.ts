import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  test: {
    globals: true,
    environment: "node",
    setupFiles: ["./src/test-setup.ts"],
  },
  build: {
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("node_modules")) {
            if (id.includes("@floating-ui")) {
              return "vendor-floating-ui";
            }
            if (id.includes("react-dom") || id.includes("/react/")) {
              return "vendor-react";
            }
            if (id.includes("highlight.js")) {
              return "vendor-highlight";
            }
            if (
              id.includes("react-markdown") ||
              id.includes("rehype-highlight") ||
              id.includes("remark-gfm") ||
              id.includes("unified") ||
              id.includes("remark-") ||
              id.includes("rehype-") ||
              id.includes("hast") ||
              id.includes("mdast") ||
              id.includes("micromark")
            ) {
              return "vendor-markdown";
            }
            if (id.includes("react-virtuoso")) {
              return "vendor-virtuoso";
            }
            if (id.includes("lucide-react")) {
              return "vendor-lucide";
            }
            if (id.includes("zustand")) {
              return "vendor-zustand";
            }
          }
        },
      },
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
    proxy: {
      "/api": {
        target: "http://127.0.0.1:18888",
        changeOrigin: true,
      },
    },
  },
});
