/**
 * 消息部件注册表（单表）。
 *
 * 每个 `cw-*` 部件在这里登记一次：渲染组件 + 是否属于「工作过程」。
 * assistant-ui 的 by_name 注册、过程折叠的归属判断、GroupedParts 的
 * 手工兜底渲染都从这一张表派生——新增一种部件只改这张表（外加 Go
 * 事件与 reducer 的 handler），不会再出现第二张平行分类表。
 */
import { MarkdownBlock } from "./Markdown";
import {
  ApprovalCard,
  AttachmentChips,
  CalloutCard,
  DraftCard,
  ErrorCard,
  NoticeCard,
  ReasoningBlock,
  TodoCard,
  ToolCard,
} from "./Parts";

/** 单个部件的登记项：渲染组件与折叠归属。 */
export interface PartEntry {
  /** 渲染组件，接收 `data` prop（转换层与 GroupedParts 兜底两条路径形状一致）。 */
  component: any;
  /** true 表示属于「工作过程」：消息结束后收进「已工作」折叠区。 */
  work: boolean;
}

/**
 * 「最终总结之前的中间正文」：工具间隙的说明文字。
 * MarkdownBlock 是受控组件（直接收文本），这里适配注册表的 data 包装。
 */
function HistoryText({ data }: { data?: any }) {
  return <MarkdownBlock text={String(data?.text ?? "")} />;
}

/** 部件注册表：键名与转换层产出的 `data-cw-<kind>` 去掉前缀后一致。 */
export const partRegistry: Record<string, PartEntry> = {
  "cw-reasoning": { component: ReasoningBlock, work: true },
  "cw-history-text": { component: HistoryText, work: true },
  "cw-tool": { component: ToolCard, work: true },
  "cw-todo": { component: TodoCard, work: true },
  "cw-approval": { component: ApprovalCard, work: true },
  // 错误、草稿、提示等内容型卡片与最终正文一起留在折叠区外，保证显性可见。
  "cw-error": { component: ErrorCard, work: false },
  "cw-draft": { component: DraftCard, work: false },
  "cw-callout": { component: CalloutCard, work: false },
  "cw-notice": { component: NoticeCard, work: false },
  "cw-attachments": { component: AttachmentChips, work: false },
};

/** assistant-ui `data.by_name` 需要的组件映射，由注册表派生。 */
export const partComponentsByName: Record<string, any> = Object.fromEntries(
  Object.entries(partRegistry).map(([name, entry]) => [name, entry.component as any]),
);

/** 判断部件种类（去掉 `cw-` 前缀后的注册表键名）是否属于「工作过程」。 */
export function isWorkKind(name: string): boolean {
  return partRegistry[name]?.work ?? false;
}

/** 按名称取渲染组件；未登记的部件返回 undefined，调用方自行降级。 */
export function partComponent(name: string): any {
  return partRegistry[name]?.component;
}
