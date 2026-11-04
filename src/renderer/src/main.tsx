// 渲染层入口：主题类挂载（localStorage 缓存快速应用，boot 后以 settings 为准校准）+ 根组件。
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { applyTheme, loadThemePref } from "./lib/theme";
import "./styles.css";

applyTheme(loadThemePref());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
