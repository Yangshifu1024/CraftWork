import { api } from "./api";

/** 复制文本。优先使用浏览器剪贴板，失败时回落到 Go 侧系统剪贴板。 */
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
