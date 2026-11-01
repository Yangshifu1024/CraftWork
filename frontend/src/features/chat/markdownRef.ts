/**
 * 「路径:行号」文件引用的统一识别规则。
 *
 * 模型在消息里引用源码位置有三种写法：裸文本（foo.go:12）、空目标链接
 * （[foo.go:12]()）和反引号行内代码（`foo.go:12`）。三处分别由
 * Markdown.tsx 的 preprocess / MarkdownLink / MarkdownCode 处理，
 * 判定必须一致，因此路径与行号的匹配规则集中在这里导出。
 */

/**
 * 路径与行号的核心匹配片段。
 *
 * 路径允许多层目录（internal/agent/runner.go）、目录与文件名中的点、
 * 以及 1-8 位扩展名；行号为 1-5 位数字。
 *
 * 刻意不匹配的形态：无扩展名文件（Makefile:3）、带列号（a.go:12:34）、
 * 行号超过 5 位——这些多半是普通代码或误写，宁可不转也不要误伤。
 *
 * preprocess 需要带上「前缀上下文」做行内替换，从这里取核心片段自行拼装；
 * 链接文本与行内代码是整体判定，直接用 matchPathLineRef。
 */
export const pathLinePattern = "((?:[\\w.-]+\\/)*[\\w.-]+\\.[A-Za-z0-9]{1,8}):(\\d{1,5})";

/** 整体匹配：文本必须恰好是一个引用，前后不能有别的字符。 */
const exactPattern = new RegExp(`^${pathLinePattern}$`);

/** 识别结果：路径与行号。行号为 0 时调用方按「不定位到行」处理。 */
export interface PathLineRef {
  path: string;
  line: number;
}

/**
 * 判断一段文本是否整体恰好是一个「路径:行号」引用。
 *
 * 用于链接文本与行内代码的兜底渲染；混合内容（前后还有其他文字）
 * 返回 null，避免把普通代码片段误判成引用。
 */
export function matchPathLineRef(text: string): PathLineRef | null {
  const match = exactPattern.exec(text);
  if (!match) {
    return null;
  }
  return { path: match[1], line: Number(match[2]) };
}
