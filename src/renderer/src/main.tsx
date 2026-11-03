// 渲染层入口：主题类挂载 + 根组件。
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

const themeKey = "craftwork-theme";
const saved = localStorage.getItem(themeKey) || "zai-dark";
document.documentElement.classList.add(saved === "zai-light" ? "theme-zai-light" : "theme-zai-dark");

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

export { themeKey };
