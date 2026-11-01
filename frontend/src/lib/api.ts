/**
 * API 入口：根据运行环境挑选后端实现。
 *
 * - 真实宿主（桌面窗口或 server 模式）使用 `desktopApi`，它通过 Wails 绑定直接调用 Go 方法；
 * - `npm run dev` 的纯 UI 预览使用 `mockApi`，全部数据在内存里。
 *
 * 业务代码只从这里导入 `api`，不直接引用具体实现，切换环境时无需改动调用方。
 */
import { isDesktop } from "./bus";
import { desktopApi } from "./desktop";
import { mockApi } from "./mock";

/** 当前运行环境的 API 实现：真实宿主（桌面 / server 模式）或纯 UI 预览的 Mock。 */
export const api = isDesktop ? desktopApi : mockApi;
