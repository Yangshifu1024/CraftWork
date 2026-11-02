/**
 * 通用工具函数：类名拼接、数值夹取、随机 ID、等待、路径处理与各类格式化。
 *
 * 这些函数保持纯函数、无副作用，前端各层都可以直接引用；不依赖 store 或 API。
 */

/** 把多个类名值拼接成一个字符串，自动过滤 false、null、undefined。 */
export function cn(...values: Array<string | false | null | undefined>): string {
  return values.filter(Boolean).join(" ");
}

/** 把数值限制在 `min` 与 `max` 之间（包含两端）。 */
export function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max);
}

/** 生成带前缀的随机 ID，用于 Mock 数据与前端临时标识；不保证全局唯一。 */
export function uid(prefix = "id"): string {
  return `${prefix}_${Math.random().toString(16).slice(2, 10)}`;
}

/** 等待指定毫秒数，用于 Mock 模拟网络与流式输出的间隔。 */
export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/** 取路径的最后一段作为文件名；路径为空时原样返回。 */
export function basename(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] || path;
}

/** 取路径的父目录；没有父目录（顶层）时返回空字符串。 */
export function dirname(path: string): string {
  const index = path.lastIndexOf("/");
  return index <= 0 ? "" : path.slice(0, index);
}

/**
 * 格式化时间戳。
 *
 * 当天的只显示「时:分」，跨天的显示「月/日 时:分」；时间戳为 0 时返回空字符串。
 */
export function formatTime(timestamp: number): string {
  if (!timestamp) {
    return "";
  }
  const date = new Date(timestamp);
  const now = new Date();
  const sameDay =
    date.getFullYear() === now.getFullYear() &&
    date.getMonth() === now.getMonth() &&
    date.getDate() === now.getDate();
  const time = `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
  if (sameDay) {
    return time;
  }
  return `${String(date.getMonth() + 1).padStart(2, "0")}/${String(date.getDate()).padStart(2, "0")} ${time}`;
}

/**
 * 格式化 token 数量，保持紧凑。
 *
 * 小于 1000 原样显示；小于 10000 显示一位小数的 k（例如 1.8k）；
 * 更大的数值四舍五入到整数 k（例如 128k）。
 */
export function formatTokens(value: number): string {
  if (!value) {
    return "0";
  }
  if (value < 1000) {
    return String(value);
  }
  if (value < 10000) {
    return `${(value / 1000).toFixed(1)}k`;
  }
  return `${Math.round(value / 1000)}k`;
}

/** 把以微美元（百万分之一美元）为单位的费用格式化为美元；小额保留四位小数。 */
export function formatCost(micros: number): string {
  if (!micros) {
    return "$0.00";
  }
  const value = micros / 1_000_000;
  // 单次调用通常远小于 1 美分，固定两位小数会全部显示成 $0.00。
  return value < 0.01 ? `$${value.toFixed(4)}` : `$${value.toFixed(2)}`;
}

/**
 * 格式化字节数。
 *
 * 按 1024 进制逐级换算到 B / KB / MB / GB；字节级别不显示小数，其余显示一位小数。
 */
export function formatBytes(value: number): string {
  if (!value) {
    return "0 B";
  }
  const units = ["B", "KB", "MB", "GB"];
  let size = value;
  let index = 0;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  return `${size.toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

/** 从中间截断过长文本，保留首尾并用省略号连接，常用于显示长路径。 */
export function truncateMiddle(value: string, max = 60): string {
  if (value.length <= max) {
    return value;
  }
  const half = Math.floor((max - 1) / 2);
  return `${value.slice(0, half)}…${value.slice(value.length - half)}`;
}

/**
 * 将文本按代码点切成固定长度的片段，供 Mock 流式输出使用。
 *
 * 用 `Array.from` 而不是 `split("")`，保证表情等由多个 UTF-16 码元组成的字符不会被切坏。
 */
export function chunkText(text: string, size = 4): string[] {
  const characters = Array.from(text);
  const chunks: string[] = [];
  for (let index = 0; index < characters.length; index += size) {
    chunks.push(characters.slice(index, index + size).join(""));
  }
  return chunks;
}

/**
 * 判断一个路径是否是可预览的图片附件。
 *
 * 扩展名白名单与后端 ReadAttachment 的校验保持一致：前端先据此决定
 * 渲染缩略图还是文件 chip，真正能否读取仍由后端校验兜底。
 */
export function isImagePath(path: string): boolean {
  return /\.(png|jpe?g|gif|webp|bmp)$/i.test(path);
}

/**
 * 按模型标识在用户目录里查找模型信息。
 *
 * 新会话保存「提供商/模型」引用；旧会话可能仍保存原始模型 ID，
 * 这里同时匹配 ref 与 id 两种历史格式，未命中返回 undefined。
 * 发送策略与消息渲染都通过它读取能力矩阵，避免两处各写一套匹配规则。
 */
export function matchModelInfo(
  providers: { models: { ref?: string; id: string; caps?: { vision?: boolean; reasoning?: boolean } }[] }[],
  modelID: string | undefined,
): { caps: { vision: boolean; reasoning: boolean } } | undefined {
  if (!modelID) {
    return undefined;
  }
  const model = providers
    .flatMap((provider) => provider.models)
    .find((model) => (model.ref ?? model.id) === modelID || model.id === modelID);
  if (!model) {
    return undefined;
  }
  return { caps: { vision: model.caps?.vision ?? false, reasoning: model.caps?.reasoning ?? false } };
}
