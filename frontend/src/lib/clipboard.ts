/**
 * 剪贴板工具：统一「复制文本」的入口。
 *
 * 桌面端与 server 模式下，WebView 可能没有浏览器剪贴板权限，
 * 因此优先走标准 API，失败后回落到 Go 侧的系统剪贴板。
 */
import { api } from "./api";

/**
 * 复制文本。
 *
 * 优先使用浏览器剪贴板，失败时回落到 Go 侧系统剪贴板。
 * 空文本直接返回，避免无意义地覆盖用户已有剪贴板内容。
 */
export async function copyText(text: string): Promise<void> {
  if (!text) {
    return;
  }
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    // wails:// 下可能没有剪贴板权限，继续走 Go。
  }
  await api.app.writeClipboard(text);
}
