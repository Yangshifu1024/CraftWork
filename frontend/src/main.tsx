/**
 * 前端入口：把 React 应用挂载到 index.html 里的 `#root` 节点。
 *
 * 启动顺序：
 * 1. 先引入全局样式 `styles/index.css`，让 CSS 变量、主题与原型样式在首屏渲染前就生效；
 * 2. 用 React 19 的 `createRoot` 创建根节点；
 * 3. 用 `StrictMode` 包裹 `<App />`，开发环境会刻意重复执行一次副作用，用来暴露副作用写得不对的地方。
 *
 * 这里只负责「把界面渲染出来」；拉取后端数据、订阅事件、加载工作区等初始化，
 * 全部在 App.tsx 的启动流程里完成。
 */
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
// 面板与全局上下文（工作区 / 工作模式 / 活动线程）的订阅接线；
// 副作用引入一次即可，详见 stores/panelWiring.ts 的说明。
import "./stores/panelWiring";
import "./styles/index.css";

// 创建 React 根节点并渲染。`as HTMLElement` 是因为 TS 无法确定 index.html 里一定有 #root。
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
