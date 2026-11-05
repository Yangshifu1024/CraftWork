// 渲染层入口：安装桥（window.craftwork 调用面，先于任何 store 动作）→
// 主题类快速应用（localStorage 缓存防闪烁；boot 后以 settings 为准校准）→ 挂载根组件。
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, loadThemePref } from "./lib/theme";
import { installTauriBridge } from "./lib/tauri-bridge";
import "./styles.css";

// 提供 window.craftwork / craftworkPush 调用面（已存在则跳过，幂等安装）。
installTauriBridge();

applyTheme(loadThemePref());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
