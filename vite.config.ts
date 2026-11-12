// 渲染层构建：唯一入口。产物输出 dist，由 tauri.conf.json 的 frontendDist 消费。
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "node:path";

export default defineConfig({
  root: "src/renderer",
  // 打包后经 tauri:// 加载：资源必须用相对路径（与 Electron 版 file:// 同理）
  base: "./",
  clearScreen: false,
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@shared": resolve(__dirname, "src/shared"),
      "@renderer": resolve(__dirname, "src/renderer/src"),
    },
  },
  // Tauri CLI 依赖 devUrl 端口固定
  server: { port: 5173, strictPort: true },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    outDir: resolve(__dirname, "dist"),
    emptyOutDir: true,
    target: "es2021",
    // 桌面应用资源本地加载，无网络传输压力，不设 chunk 体积上限以屏蔽构建警告
    chunkSizeWarningLimit: Infinity,
  },
});
