import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const host = process.env.TAURI_DEV_HOST;
const port = Number(process.env.VITE_PORT) || 1420;

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
            if (id.includes("@tauri-apps+api")) {
              return "vendor-tauri-api";
            }
            if (
              id.includes("@codemirror+lang-javascript") ||
              id.includes("@codemirror+lang-css") ||
              id.includes("@codemirror+lang-html") ||
              id.includes("@codemirror+lang-json") ||
              id.includes("@codemirror+lang-xml") ||
              id.includes("@lezer+javascript") ||
              id.includes("@lezer+css") ||
              id.includes("@lezer+html") ||
              id.includes("@lezer+json") ||
              id.includes("@lezer+xml")
            ) {
              return "vendor-codemirror-web";
            }
            if (
              id.includes("@codemirror+lang-markdown") ||
              id.includes("@codemirror+lang-python") ||
              id.includes("@codemirror+lang-php") ||
              id.includes("@codemirror+lang-sql") ||
              id.includes("@codemirror+lang-yaml") ||
              id.includes("@lezer+markdown") ||
              id.includes("@lezer+python") ||
              id.includes("@lezer+php") ||
              id.includes("@lezer+yaml")
            ) {
              return "vendor-codemirror-docs";
            }
            if (
              id.includes("@codemirror+lang-cpp") ||
              id.includes("@codemirror+lang-go") ||
              id.includes("@codemirror+lang-java") ||
              id.includes("@codemirror+lang-rust") ||
              id.includes("@lezer+cpp") ||
              id.includes("@lezer+go") ||
              id.includes("@lezer+java") ||
              id.includes("@lezer+rust")
            ) {
              return "vendor-codemirror-systems";
            }
            if (id.includes("@codemirror+view") || id.includes("@codemirror/view")) {
              return "vendor-codemirror-view";
            }
            if (id.includes("@codemirror+state") || id.includes("@codemirror/state")) {
              return "vendor-codemirror-state";
            }
            if (id.includes("@lezer") || id.includes("@lezer+")) {
              return "vendor-lezer";
            }
            if (id.includes("@codemirror+lang")) {
              return "vendor-codemirror-lang";
            }
            if (id.includes("@codemirror") || id.includes("/codemirror@")) {
              return "vendor-codemirror-core";
            }
            if (id.includes("@floating-ui")) {
              return "vendor-floating-ui";
            }
            if (id.includes("react-dom")) {
              return "vendor-react-dom";
            }
            if (id.includes("/react/") || id.includes("/react@")) {
              return "vendor-react";
            }
            if (id.includes("highlight.js/lib/languages")) {
              return "highlight-langs";
            }
            if (id.includes("highlight.js")) {
              return "vendor-highlight";
            }
            if (id.includes("lowlight")) {
              return "vendor-highlight";
            }
            if (
              id.includes("react-markdown") ||
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

          if (
            id.includes("/src/components/file-viewer/CodeViewer") ||
            id.includes("/src/components/file-viewer/cm-languages")
          ) {
            return "feature-code-viewer";
          }
          if (id.includes("/src/components/message-stream/")) {
            return "feature-message-stream";
          }
          if (id.includes("/src/components/browser/")) {
            return "feature-browser";
          }
          if (id.includes("/src/components/settings/")) {
            return "feature-settings";
          }
          if (id.includes("/src/components/file-viewer/")) {
            return "feature-file-viewer";
          }
          if (
            id.includes("/src/components/layout/TitleBar") ||
            id.includes("/src/components/layout/UpdateBanner") ||
            id.includes("/src/components/layout/ClawIcon") ||
            id.includes("/src/components/shell/AppHeader") ||
            id.includes("/src/components/common/") ||
            id.includes("/src/components/notification/")
          ) {
            return "app-shell";
          }
          if (
            id.includes("/src/components/shell/InteractiveTerminal") ||
            id.includes("/src/components/shell/TerminalPanel") ||
            id.includes("/src/components/shell/TerminalTabContent")
          ) {
            return "feature-terminal";
          }
          if (
            id.includes("/src/components/shell/SearchPanel") ||
            id.includes("/src/components/shell/ReviewTabContent")
          ) {
            return "feature-review-search";
          }
          if (
            id.includes("/src/components/shell/FilesTabContent") ||
            id.includes("/src/components/shell/WorkspacePanel")
          ) {
            return "feature-workspace-panel";
          }
          if (
            id.includes("/src/components/shell/GoalPanel") ||
            id.includes("/src/components/shell/CoordinatorPanel") ||
            id.includes("/src/components/shell/WelcomeView") ||
            id.includes("/src/components/shell/AppSidebar")
          ) {
            return "app-shell-panels";
          }
          if (id.includes("/src/lib/stores/")) {
            return "state-stores";
          }
          if (id.includes("/src/lib/")) {
            return "app-lib";
          }
        },
      },
    },
  },
  clearScreen: false,
  server: {
    port,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: port + 1 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
    proxy: {
      "/api": {
        target: "http://127.0.0.1:18888",
        changeOrigin: true,
      },
    },
  },
});
