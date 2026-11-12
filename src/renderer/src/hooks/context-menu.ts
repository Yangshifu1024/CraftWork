// 原生右键菜单抑制：WebView 默认菜单（Reload / Inspect Element）与应用无关，全局禁用；
// 可编辑元素（input / textarea / contenteditable）放行系统菜单（拷贝 / 粘贴 / 拼写检查）。
// 组件级自定义菜单（侧栏会话行 / 分组头）在 React onContextMenu 内自行处理，不受影响。
import { useEffect } from "react";

/** 注册 contextmenu 拦截（组件卸载时解绑），规则见文件头注释。 */
export function useNativeContextMenuGuard(): void {
  useEffect(() => {
    const onCtx = (e: MouseEvent): void => {
      const target = e.target;
      if (target instanceof Element && target.closest("input, textarea, [contenteditable]")) return;
      e.preventDefault();
    };
    window.addEventListener("contextmenu", onCtx);
    return () => window.removeEventListener("contextmenu", onCtx);
  }, []);
}
