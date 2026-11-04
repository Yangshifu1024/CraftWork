// Markdown 渲染：流式安全的 Markdown 视图。
// 安全策略：不渲染原始 HTML（按文本显示）；代码块高亮；GFM 表格与任务列表由渲染器默认提供。
import { Streamdown } from "streamdown";

export function Markdown({ text, streaming }: { text: string; streaming: boolean }): React.JSX.Element {
  return (
    <div className={`select-text break-words ${streaming ? "stream-fade-in" : ""}`}>
      <Streamdown>{text}</Streamdown>
    </div>
  );
}
