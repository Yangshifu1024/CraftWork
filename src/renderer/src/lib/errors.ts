// 渲染层共享错误清洗：剥掉 Electron IPC 包装，保留服务端可读中文错误。
// 渲染层 toast / 内联红字统一经此函数取文案（此前各组件各持一份副本，已收敛至此）。

/** 剥掉「Error invoking remote method ...: Error: 」前缀，只留可读文案。 */
export function cleanErrorMessage(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  const marker = "Error:";
  const idx = raw.lastIndexOf(marker);
  return (idx >= 0 ? raw.slice(idx + marker.length) : raw).trim();
}
