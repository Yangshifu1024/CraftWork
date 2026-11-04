// 渲染层构建（Tauri 版）：与 electron.vite.config.ts 的 renderer 段保持同构（alias、base、插件一致）。
// 产物输出 out/web（electron-builder 的 dist/ 由 Electron 版继续占用，互不干扰）；P9 切换时成为唯一渲染层配置。
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
    outDir: resolve(__dirname, "out/web"),
    emptyOutDir: true,
    target: "es2021",
  },
});
