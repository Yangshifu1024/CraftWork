// 展示格式化工具：与历史 React 版 lib/utils.ts 的 formatTokens 同口径。

/**
 * 取路径最后一段作为展示名（跨平台分隔符兜底）。
 * 项目源文件夹列表（创建 / 编辑对话框）共用：Windows 反斜杠与末尾分隔符都要处理。
 */
export function pathLeaf(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, "");
  const index = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return index >= 0 ? trimmed.slice(index + 1) || trimmed : trimmed;
}

/** token 数紧凑展示：<1000 原样、<10000 一位小数 k、更大四舍五入 k（12.5k→13k、12.4k→12k）。 */
export function formatTokens(value: number): string {
  if (!value || value <= 0) return "0";
  if (value < 1000) return String(value);
  if (value < 10000) return `${(value / 1000).toFixed(1)}k`;
  return `${Math.round(value / 1000)}k`;
}

/**
 * 速度展示：<10 tok/s 保留一位小数（下限 0.1），≥10 取整。
 * 段内停顿（限流、慢段）会让实时速度衰减到 1 以下，整数取整把 0.x 显示成
 * 「0 tok/s」，造成明明在跑却显示 0 的假象；低速本来就该读作「很慢」而非「0」。
 */
export function formatSpeed(value: number): string {
  if (!value || value <= 0) return "0";
  if (value >= 10) return String(Math.round(value));
  return Math.max(0.1, value).toFixed(1);
}

/** 时长展示（毫秒）：<60s 保留一位小数秒（首 token 延迟等），更长按分钟；无记录返回「—」。 */
export function formatDurationMs(ms: number): string {
  if (!ms || ms <= 0) return "—";
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${(ms / 60_000).toFixed(1)}min`;
}

/**
 * 运行计时展示（秒）：分段时钟格式。<1 分钟保留一位小数秒（短时长有推进感；整数输入
 * 不补假精度，直接整数秒——思考折叠行的 1s 粒度计时）；更长进位为「分:秒」，跨小时为
 * 「时:分:秒」——长任务显示「4:05」「1:02:03」，不再出现「754.3 秒」这类需要心算进位的值。
 */
export function formatElapsed(seconds: number): string {
  if (!seconds || seconds <= 0) return "0.0 秒";
  if (seconds < 60) return Number.isInteger(seconds) ? `${seconds} 秒` : `${seconds.toFixed(1)} 秒`;
  const total = Math.floor(seconds);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = String(total % 60).padStart(2, "0");
  if (hours > 0) return `${hours}:${String(minutes).padStart(2, "0")}:${secs}`;
  return `${minutes}:${secs}`;
}

/**
 * 字节数人类可读展示：<1KB 原样整数 B，之后按 1024 进制一位小数（1.5 KB / 12.3 MB / 1.1 GB）。
 * 用于「关于」页的数据库占用这类只在数量级上有意义的场景。
 */
export function formatBytes(bytes: number): string {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let index = 0;
  while (value >= 1024 && index < units.length - 1) {
    value /= 1024;
    index += 1;
  }
  return `${value.toFixed(1)} ${units[index]}`;
}

/**
 * 去除首尾空行（`whitespace-pre-wrap` 原样渲染的场景用）。
 *
 * 模型常在正文末尾多吐几个换行（`\n\n\n\n`）。走 Markdown 渲染时解析器会自动吞掉
 * 尾部空行；但折叠区里的中间解说按纯文本 + `whitespace-pre-wrap` 渲染，尾随换行
 * 会变成真实空行（实测每多一个 `\n` 多出 21px），在时间线上呈现为突兀的空白块。
 * 只裁首尾、不动中间——段内的空行是作者有意的分段。
 */
export function trimBlankLines(text: string): string {
  return text.replace(/^(?:[ \t]*\n)+/, "").replace(/\n(?:[ \t]*\n)*[ \t]*$/, "");
}
