/**
 * 聊天消息里的 Markdown 渲染。
 *
 * 职责：
 * - 用 assistant-ui 的 Markdown 文本部件渲染模型回复，支持 GFM 表格、任务列表等。
 * - 代码块自带语法高亮、行号、换行开关、复制与超长折叠。
 * - 按安全策略处理链接与图片：原始 HTML 不渲染、远程图片不加载、
 *   外部链接必须经用户确认后交给系统浏览器打开。
 * - 把正文里 `path:line` 形式的引用转换成可点击的内部链接，
 *   点击后在工作区面板里定位到对应文件与行号。
 *
 * 注意：本文件只负责渲染与交互，不承担消息状态管理。
 */

import { MarkdownTextPrimitive } from "@assistant-ui/react-markdown";
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import go from "highlight.js/lib/languages/go";
import java from "highlight.js/lib/languages/java";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import markdown from "highlight.js/lib/languages/markdown";
import plaintext from "highlight.js/lib/languages/plaintext";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";
import { type ReactNode, useMemo, useState } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { Icon } from "../../components/Icon";
import { api } from "../../lib/api";
import { copyText } from "../../lib/clipboard";
import { cn } from "../../lib/utils";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceStore } from "../../stores/workspace";
import { matchPathLineRef, pathLinePattern } from "./markdownRef";

// 只注册需要的高亮语言，避免把 highlight.js 的全量语言包打进首屏包体。
hljs.registerLanguage("go", go);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("json", json);
hljs.registerLanguage("markdown", markdown);
hljs.registerLanguage("css", css);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("python", python);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("java", java);
hljs.registerLanguage("yaml", yaml);
hljs.registerLanguage("bash", bash);
hljs.registerLanguage("sql", sql);
hljs.registerLanguage("diff", diff);
hljs.registerLanguage("plaintext", plaintext);

/**
 * 常见语言别名到 highlight.js 语言名的映射。
 * 模型常输出 `ts`、`sh`、`yml` 这类短名，直接查表可以让高亮命中。
 */
const aliases: Record<string, string> = {
  ts: "typescript",
  tsx: "typescript",
  js: "javascript",
  jsx: "javascript",
  mjs: "javascript",
  cjs: "javascript",
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  yml: "yaml",
  md: "markdown",
  py: "python",
  rs: "rust",
  html: "xml",
  htm: "xml",
  svg: "xml",
  text: "plaintext",
};

/**
 * 把纯文本转义成可以安全放进 HTML 的字符串。
 * 渲染代码块的兜底路径会直接使用它的结果，防止代码内容被当成标签执行。
 */
function escapeHtml(value: string): string {
  return value.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/**
 * 对代码片段做语法高亮，返回 HTML 字符串。
 *
 * 语言名先经别名表归一化；未注册的语言或高亮过程抛错时，
 * 退回到 HTML 转义后的纯文本，保证内容始终可读且不会注入标签。
 */
function highlight(code: string, language: string): string {
  const resolved = aliases[language] ?? language;
  if (resolved && hljs.getLanguage(resolved)) {
    try {
      return hljs.highlight(code, { language: resolved, ignoreIllegals: true }).value;
    } catch {
      return escapeHtml(code);
    }
  }
  return escapeHtml(code);
}

/**
 * 代码块卡片。
 *
 * @param code 代码原文，不包含结尾换行。
 * @param language 代码围栏上标注的语言；为空时显示为 text。
 *
 * 关键状态：
 * - `wrapped`：是否强制换行，默认关闭以便横向滚动查看长行。
 * - `expanded`：超过 26 行时默认折叠，点击“展开全部”后显示完整内容。
 * - `showLines`：是否显示行号，默认显示。
 *
 * 高亮结果只来自 highlight.js 或本地转义，因此可以安全地写入 innerHTML。
 */
function CodeBlock({ code, language }: { code: string; language: string }) {
  const [wrapped, setWrapped] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [showLines, setShowLines] = useState(true);
  const html = useMemo(() => highlight(code, language), [code, language]);
  const lines = code.split("\n");
  const collapsed = lines.length > 26 && !expanded;

  return (
    <div className={cn("code-block", collapsed && "collapsed", !showLines && "no-lines")}>
      <div className="code-head">
        <span className="code-lang">
          <Icon name="code" className="ico" />
          {language || "text"}
        </span>
        <span className="code-actions">
          <button type="button" className={showLines ? "is-on" : ""} onClick={() => setShowLines(!showLines)}>
            行号
          </button>
          <button type="button" className={wrapped ? "is-on" : ""} onClick={() => setWrapped(!wrapped)}>
            换行
          </button>
          <button type="button" onClick={() => void copyText(code)}>
            <Icon name="copy" className="ico" />
            复制
          </button>
        </span>
      </div>
      <div className="code-body">
        {showLines ? (
          <div className="code-gutter">
            {lines.map((_, index) => (
              // biome-ignore lint/suspicious/noArrayIndexKey: 行号列表是纯静态展示，行数只随内容整体替换
              <span key={index}>{index + 1}</span>
            ))}
          </div>
        ) : null}
        {/* 高亮结果由 highlight.js 生成，或来自本地转义，直接写入可避免二次解析。 */}
        {/* biome-ignore lint/security/noDangerouslySetInnerHtml: 内容只来自 highlight.js 输出与本地转义，不包含原始 HTML */}
        <pre className={wrapped ? "is-wrap" : undefined} dangerouslySetInnerHTML={{ __html: html }} />
      </div>
      {lines.length > 26 ? (
        <button type="button" className="code-more" onClick={() => setExpanded(!expanded)}>
          {expanded ? "收起" : `展开全部 ${lines.length} 行`}
        </button>
      ) : null}
    </div>
  );
}

/**
 * Markdown 行内代码与围栏代码的统一渲染入口。
 *
 * 带 `language-xxx` class 的视为围栏代码，交给 CodeBlock；
 * 其余视为行内代码，只加样式不参与高亮。
 *
 * 行内代码整体恰好是 `path:line` 形式时按文件引用渲染：模型常把引用
 * 包进反引号，preprocess 不会进入 code span，这里兜底让它与裸文本
 * 引用一样可以点击跳转到工作区面板的对应行。
 */
function MarkdownCode({ className, children }: { className?: string; children?: ReactNode }) {
  const match = /language-([\w-]+)/.exec(className ?? "");
  if (match) {
    return <CodeBlock code={String(children ?? "").replace(/\n$/, "")} language={match[1]} />;
  }
  const ref = matchPathLineRef(plainText(children));
  if (ref) {
    return (
      <button
        type="button"
        className="ref-link"
        onClick={() => void useWorkspaceStore.getState().openFileAt(ref.path, ref.line || null)}
      >
        {children}
      </button>
    );
  }
  return <code className={cn("md-code", className)}>{children}</code>;
}

/**
 * 在系统浏览器中打开外部链接。
 *
 * 安全策略：应用内不加载外部网页，必须先弹出确认框，用户同意后
 * 才通过 `AppService.OpenExternal` 交给系统浏览器；打开失败时用 toast 提示。
 */
async function openExternal(href: string): Promise<void> {
  let host = href;
  try {
    host = new URL(href).host || href;
  } catch {
    // 保留原始字符串用于展示。
  }
  const confirmed = await useUiStore.getState().askConfirm({
    title: "用系统浏览器打开链接？",
    description: "应用内不会加载外部网页，链接将交给系统浏览器。",
    detail: `${host}\n${href}`,
    confirmText: "打开链接",
  });
  if (confirmed) {
    try {
      await api.app.openExternal(href);
    } catch (error) {
      useUiStore.getState().toast({
        title: "无法打开链接",
        description: String(error),
        tone: "danger",
      });
    }
  }
}

/**
 * 提取 React 子节点的纯文本，用于识别模型输出的引用样式链接。
 * 嵌套结构（加粗、行内代码包裹的引用）也能取到完整文字。
 */
// biome-ignore lint/suspicious/noExplicitAny: react-markdown 的子节点是递归 React 节点，运行时按类型分支识别
function plainText(node: any): string {
  if (typeof node === "string") {
    return node;
  }
  if (typeof node === "number") {
    return String(node);
  }
  if (Array.isArray(node)) {
    return node.map(plainText).join("");
  }
  if (node?.props?.children !== undefined) {
    return plainText(node.props.children);
  }
  return "";
}

/**
 * Markdown 链接渲染器。
 *
 * 三类链接：
 * - `craftwork://path:line`：由 preprocess 生成的内部引用，渲染成按钮，
 *   点击后让工作区面板打开对应文件并定位到行号。
 * - 没有目标地址、但文本形如 `path:line` 的链接：模型有时把引用输出成
 *   空目标（`[policy.go:176]()`）的 Markdown 链接，preprocess 不会二次
 *   包裹已有链接，这里兜底按内部引用处理，避免弹出“用系统浏览器打开”。
 * - 其他链接：渲染成普通链接，但会阻止默认跳转；页内锚点（# 开头）不处理，
 *   其余一律走 openExternal 的确认流程。
 */
// biome-ignore lint/suspicious/noExplicitAny: react-markdown 链接渲染器的 props 含动态透传属性，按需读取
function MarkdownLink({ href, children, ...rest }: any) {
  if (typeof href === "string" && href.startsWith("craftwork://")) {
    const path = String(rest["data-ref-path"] ?? href.replace("craftwork://", "").replace(/:\d+$/, ""));
    const line = Number(rest["data-ref-line"] ?? 0);
    return (
      <button
        type="button"
        className="ref-link"
        onClick={() => void useWorkspaceStore.getState().openFileAt(path, line || null)}
      >
        {children}
      </button>
    );
  }
  // 空目标（无 href、空字符串或纯 #）且文本是 path:line 形式时按内部引用兜底。
  const ref = !href || href === "#" ? matchPathLineRef(plainText(children)) : null;
  if (ref) {
    return (
      <button
        type="button"
        className="ref-link"
        onClick={() => void useWorkspaceStore.getState().openFileAt(ref.path, ref.line || null)}
      >
        {children}
      </button>
    );
  }
  return (
    <a
      className="md-link"
      href={href}
      onClick={(event) => {
        event.preventDefault();
        if (typeof href === "string" && href.startsWith("#")) {
          return;
        }
        void openExternal(String(href ?? ""));
      }}
    >
      {children}
    </a>
  );
}

/**
 * 交给 react-markdown 的部件覆盖表。
 *
 * - `pre` 直接透传子节点，因为围栏代码已经由 CodeBlock 自己绘制容器。
 * - `code` / `a` 使用本文件的自定义实现。
 * - `img` 不加载图片，只显示“[图片：替代文字]”，避免请求远程地址。
 */
const components: Components = {
  // biome-ignore lint/suspicious/noExplicitAny: pre 渲染器只透传子节点，与 Components 的复杂泛型签名断言适配
  pre: (({ children }: any) => <>{children}</>) as Components["pre"],
  code: MarkdownCode as Components["code"],
  a: MarkdownLink as Components["a"],
  // biome-ignore lint/suspicious/noExplicitAny: 图片按约定不加载，只读 alt 文本
  img: ({ alt }: any) => <span className="md-image-note">[图片：{alt || "未命名"}]</span>,
};

/**
 * 把 path:line 形式的引用转换成内部链接，链接在 a 渲染器里变成可点击按钮。
 * 代码围栏内的内容不做转换。
 *
 * 实现方式：逐行扫描，遇到 ``` 切换围栏状态；围栏外的行用正则匹配
 * “带扩展名的路径:行号”，替换为 `[path:line](craftwork://path:line)`。
 * 核心匹配规则与 markdownRef 同源，保证三种写法判定一致。
 */
const bareRefPattern = new RegExp(`(^|[\\s(（「“])${pathLinePattern}(?=\\b)`, "g");

function preprocess(text: string): string {
  const lines = text.split("\n");
  let inFence = false;
  return lines
    .map((line) => {
      if (/^\s*```/.test(line)) {
        inFence = !inFence;
        return line;
      }
      if (inFence) {
        return line;
      }
      return line.replace(bareRefPattern, (_whole, prefix, path, lineNumber) => {
        return `${prefix}[${path}:${lineNumber}](craftwork://${path}:${lineNumber})`;
      });
    })
    .join("\n");
}

/**
 * Markdown 文本部件。
 *
 * 作为 assistant-ui 的 Text 部件使用：开启 GFM（表格、任务列表、删除线），
 * 关闭流式渐变效果，并在渲染前执行本地引用预处理。
 */
export function MarkdownText() {
  return (
    <MarkdownTextPrimitive
      className="blk md"
      remarkPlugins={[remarkGfm]}
      components={components}
      preprocess={preprocess}
      smooth={false}
    />
  );
}

/**
 * 受控 Markdown 渲染：直接渲染给定的文本，不依赖 assistant-ui 的部件上下文。
 *
 * 用在「工作过程折叠区」里的中间正文（data-cw-history-text）——这些文本
 * 在运行时里是 data 部件，没有内置 Text 部件的上下文可取，因此走 react-markdown
 * 渲染；安全策略（链接确认、图片不加载、代码高亮、本地引用链接）与正文完全一致。
 */
export function MarkdownBlock({ text }: { text: string }) {
  return (
    <div className="blk md">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {preprocess(text)}
      </ReactMarkdown>
    </div>
  );
}
