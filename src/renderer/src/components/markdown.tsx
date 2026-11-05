// Markdown 渲染：流式安全的 Markdown 视图。
// 安全策略：不渲染原始 HTML（按文本显示）；代码块高亮。
// 字号与外框：.markdown-body 的字号/表格覆盖见 styles.css——streamdown 用 Tailwind 绝对字号，
// 按元素映射到 UI 字号约束（--ui-font-size 体系）；表格 wrapper 是它自带的卡片外框，此处关掉
// 表格工具按钮（controls），外框样式在 CSS 层移除。
import { Streamdown } from "streamdown";

export function Markdown({ text, streaming }: { text: string; streaming: boolean }): React.JSX.Element {
  return (
    <div className={`markdown-body select-text break-words text-ui-base ${streaming ? "stream-fade-in" : ""}`}>
      <Streamdown controls={{ table: false, code: { download: false } }}>{text}</Streamdown>
    </div>
  );
}
