import { isDesktop } from "./bus";
import { desktopApi } from "./desktop";
import { mockApi } from "./mock";

/** 当前运行环境的 API 实现：真实宿主（桌面 / server 模式）或纯 UI 预览的 Mock。 */
export const api = isDesktop ? desktopApi : mockApi;
