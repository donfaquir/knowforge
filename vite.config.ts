import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  resolve: {
    // 避免 Vite 将 CodeMirror 拆进多 chunk 时出现多份 @codemirror/*，进而导致 foldService 等 Facet 异常
    dedupe: [
      "@codemirror/state",
      "@codemirror/view",
      "@codemirror/language",
      "@lezer/common",
      "@lezer/highlight",
    ],
  },

  clearScreen: false,
  build: {
    /* Crepe 懒加载后单 chunk 仍含 Milkdown+多数 CodeMirror 语言，约 650kB+；主入口已 <500kB */
    chunkSizeWarningLimit: 750,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes("node_modules")) {
            return;
          }
          if (id.includes("node_modules/react-dom")) {
            return "vendor-react-dom";
          }
          if (id.includes("node_modules/react/")) {
            return "vendor-react";
          }
          /* Milkdown/Crepe/CodeMirror 随 CrepeMarkdownEditor 懒加载 chunk 打包，勿抽成全局 vendor（否则单包 >2MB 仍告警） */
          if (id.includes("node_modules/katex")) {
            return "vendor-katex";
          }
          if (id.includes("node_modules/i18next") || id.includes("node_modules/react-i18next")) {
            return "vendor-i18n";
          }
        },
      },
    },
  },
  server: {
    port: 11186,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 11187,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
}));
