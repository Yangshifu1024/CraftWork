// Markdown 渲染：流式安全的 Markdown 视图。
// 安全策略：streamdown 经 rehype-raw + rehype-sanitize——良性 HTML 标签按 HTML 渲染，
// 危险内容（script/事件属性/javascript: 等）被 sanitize 剥除；代码块高亮。
// 字号与外框：.markdown-body 的字号/表格覆盖见 styles.css——streamdown 用 Tailwind 绝对字号，
// 按元素映射到 UI 字号约束（--ui-font-size 体系）；表格 wrapper 是它自带的卡片外框，此处关掉
// 表格工具按钮（controls），外框样式在 CSS 层移除。
// 高度全展开：streamdown 默认表格限高 300px、代码块限高 400px（内联 max-height + 滚动），
// 显式置 0 禁用，表格与代码块始终按真实内容高度展开（`conversation.tsx` 的 CONTENT_WIDTH_CLASS 注释）。
// 语法高亮：streamdown 核心不含高亮器，代码块默认退化为单色纯文本；着色由可选的
// @streamdown/code 插件（shiki）提供，双主题 [light, dark] 由 .markdown-body 的 CSS 变量切换，
// 行为约定见 `markdown.tsx` 与 `styles.css` 的代码块样式段。
import { code } from "@streamdown/code";
import { Streamdown } from "streamdown";

/** 流式 Markdown 视图。正文不做流式动画：淡入类挂在容器上时，正文增长/切段重组会整段重播
 *  （重挂载触发 animation both），长淡入还让持续更新的正文始终处于半透明态——正文是内容
 *  本体，保持稳定不闪（正文无动画，运行状态由过程行与轮尾槽位表达）。
 *  安全与字号策略见文件头。 */
export function Markdown({ text }: { text: string }): React.JSX.Element {
  return (
    <div className="markdown-body select-text break-words text-ui-base">
      <Streamdown
        codeBlockMaxHeight={0}
        tableMaxHeight={0}
        controls={{ table: false, code: { download: false } }}
        plugins={{ code }}
      >
        {text}
      </Streamdown>
    </div>
  );
}
