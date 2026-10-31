import { isDesktop } from "./bus";
import { desktopApi } from "./desktop";
import { mockApi } from "./mock";

/** 当前运行环境的 API 实现：Wails 桌面或浏览器 Mock。 */
export const api = isDesktop ? desktopApi : mockApi;
