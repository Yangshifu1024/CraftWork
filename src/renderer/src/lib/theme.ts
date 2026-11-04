// 主题偏好与应用的收口：三态偏好（system/light/dark）解析为实际主题类；
// system 跟随系统配色并监听变化。settings.theme 是事实源，
// localStorage 只作启动防闪烁缓存（存偏好本身，兼容旧版本写入的解析值）。
export type ThemePref = "system" | "light" | "dark";
export type ResolvedTheme = "theme-zai-light" | "theme-zai-dark";

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
