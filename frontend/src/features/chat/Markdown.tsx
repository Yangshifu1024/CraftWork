import { useMemo, useState, type ReactNode } from "react";
import { MarkdownTextPrimitive } from "@assistant-ui/react-markdown";
import type { Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import hljs from "highlight.js/lib/core";
import go from "highlight.js/lib/languages/go";
import typescript from "highlight.js/lib/languages/typescript";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import markdown from "highlight.js/lib/languages/markdown";
import css from "highlight.js/lib/languages/css";
import xml from "highlight.js/lib/languages/xml";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import java from "highlight.js/lib/languages/java";
import yaml from "highlight.js/lib/languages/yaml";
import bash from "highlight.js/lib/languages/bash";
import sql from "highlight.js/lib/languages/sql";
import diff from "highlight.js/lib/languages/diff";
import plaintext from "highlight.js/lib/languages/plaintext";
import { Icon } from "../../components/Icon";
import { copyText } from "../../lib/clipboard";
import { api } from "../../lib/api";
import { cn } from "../../lib/utils";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceStore } from "../../stores/workspace";

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

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

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
              <span key={index}>{index + 1}</span>
            ))}
          </div>
        ) : null}
        <pre
          className={wrapped ? "is-wrap" : undefined}
          dangerouslySetInnerHTML={{ __html: html }}
        />
      </div>
      {lines.length > 26 ? (
        <button type="button" className="code-more" onClick={() => setExpanded(!expanded)}>
          {expanded ? "收起" : `展开全部 ${lines.length} 行`}
        </button>
      ) : null}
    </div>
  );
}

function MarkdownCode({ className, children }: { className?: string; children?: ReactNode }) {
  const match = /language-([\w-]+)/.exec(className ?? "");
  if (match) {
    return <CodeBlock code={String(children ?? "").replace(/\n$/, "")} language={match[1]} />;
  }
  return <code className={cn("md-code", className)}>{children}</code>;
}

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

const components: Components = {
  pre: (({ children }: any) => <>{children}</>) as Components["pre"],
  code: MarkdownCode as Components["code"],
  a: MarkdownLink as Components["a"],
  img: ({ alt }: any) => <span className="md-image-note">[图片：{alt || "未命名"}]</span>,
};

/**
 * 把 path:line 形式的引用转换成内部链接，链接在 a 渲染器里变成可点击按钮。
 * 代码围栏内的内容不做转换。
 */
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
      return line.replace(
        /(^|[\s(（「“])((?:[\w.-]+\/)*[\w.-]+\.[A-Za-z0-9]{1,8}):(\d{1,5})(?=\b)/g,
        (whole, prefix, path, lineNumber) =>
          `${prefix}[${path}:${lineNumber}](craftwork://${path}:${lineNumber})`,
      );
    })
    .join("\n");
}

/** Markdown 文本部件。 */
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
