// 主题偏好与应用的收口：三态偏好（system/light/dark）解析为实际主题类；
// system 跟随系统配色并监听变化。settings.theme 是事实源，
// localStorage 只作启动防闪烁缓存（存偏好本身，兼容旧版本写入的解析值）。
// 同文件收口界面模式（code/work）的主色 class：与主题 class 同机制，
// html 上挂 mode-code / mode-work，styles.css「模式主色命名空间」据此覆写事实主色 brand。
import { getCurrentWindow } from "@tauri-apps/api/window";

export type ThemePref = "system" | "light" | "dark";
export type ResolvedTheme = "theme-craft-light" | "theme-craft-dark";

// 主题底色（与 styles.css 的 --craft-background 两套值一致）：
// 同步到原生窗口背景，窗口 resize 时 WKWebView 重绘滞后露出的填充区域与内容同色，
// 避免最大化/分屏时出现突兀的色块（Electron 时代窗口背景由 Chromium 自动处理）。
const THEME_WINDOW_BACKGROUND: Record<ResolvedTheme, { red: number; green: number; blue: number; alpha: number }> = {
  "theme-craft-light": { red: 0xec, green: 0xec, blue: 0xee, alpha: 255 },
  "theme-craft-dark": { red: 0x2b, green: 0x2b, blue: 0x2b, alpha: 255 },
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
  if (pref === "light") return "theme-craft-light";
  if (pref === "dark") return "theme-craft-dark";
  return darkMql.matches ? "theme-craft-dark" : "theme-craft-light";
}

/** 把偏好应用到文档根类，返回解析结果。 */
export function applyTheme(pref: ThemePref): ResolvedTheme {
  const resolved = resolveTheme(pref);
  document.documentElement.classList.remove("theme-craft-light", "theme-craft-dark");
  document.documentElement.classList.add(resolved);
  syncWindowBackground(resolved);
  return resolved;
}

/** 读启动缓存偏好；兼容旧版本写入的解析值（主题类改名前的 `light`/`dark` 解析值）。
 *  这两条只做历史 localStorage 值的迁移，与当前主题类名无关，改名时不可一并删除。 */
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

// ---------- 界面模式（code / work）主色 ----------

export type UiMode = "code" | "work";

const MODE_KEY = "craftwork-mode";

/** 把模式应用到文档根类：mode-code / mode-work 二选一，驱动全局模式主色。 */
export function applyModeTheme(mode: UiMode): void {
  const root = document.documentElement;
  root.classList.toggle("mode-code", mode === "code");
  root.classList.toggle("mode-work", mode === "work");
}

/** 读启动缓存模式；缺省 code（与 settings.mode 的缺省一致）。 */
export function loadModePref(): UiMode {
  return localStorage.getItem(MODE_KEY) === "work" ? "work" : "code";
}

/** 写启动缓存模式（与 settings.mode 同构）。 */
export function saveModePref(mode: UiMode): void {
  localStorage.setItem(MODE_KEY, mode);
}
