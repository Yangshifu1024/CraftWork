// 主题偏好与应用的收口：三态偏好（system/light/dark）解析为实际主题类；
// system 跟随系统配色并监听变化。settings.theme 是事实源，
// localStorage 只作启动防闪烁缓存（存偏好本身，兼容旧版本写入的解析值）。
import { getCurrentWindow } from "@tauri-apps/api/window";

export type ThemePref = "system" | "light" | "dark";
export type ResolvedTheme = "theme-zai-light" | "theme-zai-dark";

// 主题底色（与 styles.css 的 --zai-background 两套值一致）：
// 同步到原生窗口背景，窗口 resize 时 WKWebView 重绘滞后露出的填充区域与内容同色，
// 避免最大化/分屏时出现突兀的色块（Electron 时代窗口背景由 Chromium 自动处理）。
const THEME_WINDOW_BACKGROUND: Record<ResolvedTheme, { red: number; green: number; blue: number; alpha: number }> = {
  "theme-zai-light": { red: 0xf8, green: 0xf8, blue: 0xf8, alpha: 255 },
  "theme-zai-dark": { red: 0x16, green: 0x16, blue: 0x16, alpha: 255 },
};

function syncWindowBackground(resolved: ResolvedTheme): void {
  // Tauri 环境专用；失败静默（背景不同步只影响 resize 观感）
  void getCurrentWindow()
    .setBackgroundColor(THEME_WINDOW_BACKGROUND[resolved])
    .catch(() => undefined);
}

const THEME_KEY = "craftwork-theme";
const darkMql = window.matchMedia("(prefers-color-scheme: dark)");

/** 偏好 → 实际主题类；system 按系统当前亮暗解析。 */
export function resolveTheme(pref: ThemePref): ResolvedTheme {
  if (pref === "light") return "theme-zai-light";
  if (pref === "dark") return "theme-zai-dark";
  return darkMql.matches ? "theme-zai-dark" : "theme-zai-light";
}

/** 把偏好应用到文档根类，返回解析结果。 */
export function applyTheme(pref: ThemePref): ResolvedTheme {
  const resolved = resolveTheme(pref);
  document.documentElement.classList.remove("theme-zai-light", "theme-zai-dark");
  document.documentElement.classList.add(resolved);
  syncWindowBackground(resolved);
  return resolved;
}

/** 读启动缓存偏好；兼容旧版本写入的解析值（zai-light/zai-dark）。缺省跟随系统。 */
export function loadThemePref(): ThemePref {
  const saved = localStorage.getItem(THEME_KEY);
  if (saved === "light" || saved === "dark" || saved === "system") return saved;
  if (saved === "zai-light") return "light";
  if (saved === "zai-dark") return "dark";
  return "system";
}

/** 写启动缓存偏好（与 settings.theme 同构）。 */
export function saveThemePref(pref: ThemePref): void {
  localStorage.setItem(THEME_KEY, pref);
}

/** system 偏好下跟随系统亮暗变化并重新应用；返回解绑函数。 */
export function watchSystemTheme(getPref: () => ThemePref, onApply?: (resolved: ResolvedTheme) => void): () => void {
  const onChange = (): void => {
    if (getPref() === "system") onApply?.(applyTheme("system"));
  };
  darkMql.addEventListener("change", onChange);
  return () => darkMql.removeEventListener("change", onChange);
}
