// 工具卡的变更 diff 视图（D21）：写文件类工具展开时展示「哪几行被改」。
//
// 设计取态：
// - 只读展示，不带操作（还原 / 应用都不在这里——备份与恢复走数据面板）；
// - 增删只靠行背景、行号与标记的语义色表达，不把 patch 的 +/-/空格协议符当正文显示；
// - 行内容按文件语言着色，与正文代码块同一套 shiki 主题；
// - 渲染有界（MAX_RENDERED_LINES + 容器限高），一次大改动不撑开对话流；
// - 没有 `diff` 字段的旧事件不会走到这里：ToolCard 仍按纯文本输出渲染。
import { useEffect, useMemo, useState } from "react";
import type { DiffHunk, DiffLine, DiffResult } from "../../../shared/types";
import { highlightLines, languageForPath, type HighlightToken } from "../lib/code-highlight";
import { useApp } from "../store/app";

/** 单次展开最多渲染的差异行数：超出部分只报数量（滚动到底也不会再长）。 */
const MAX_RENDERED_LINES = 400;

const KIND_STYLE: Record<DiffLine["kind"], { row: string; gutter: string; marker: string }> = {
  add: { row: "bg-diff-added/15", gutter: "text-diff-added", marker: "+" },
  del: { row: "bg-diff-removed/15", gutter: "text-diff-removed", marker: "−" },
  ctx: { row: "", gutter: "text-foreground-subtlest", marker: " " },
};

/** 渲染所需的扁平行（保留 hunk 边界，用于插入 @@ 头）。 */
type FlatRow = { kind: "hunk"; header: string } | { kind: "line"; line: DiffLine };

function flatten(hunks: DiffHunk[]): FlatRow[] {
  const rows: FlatRow[] = [];
  for (const hunk of hunks) {
    rows.push({ kind: "hunk", header: hunk.header });
    for (const line of hunk.lines) rows.push({ kind: "line", line });
  }
  return rows;
}

/** 把 diff 里所有行正文拼成一段文本供整体 tokenize（跨行语法必须有完整上下文才对，
 *  逐行 highlight 会把模板字符串、块注释之后的整段都染错）。
 *
 *  **只含 `line` 行，hunk 头不参与**——`tokensByRow` 也是按「每行一个 token 行」索引的，
 *  这里若把 hunk 头当成一个空行拼进去，两边口径就差了一行，着色 token 整体错位：
 *  每个位置拿到的颜色与文本都来自上一行，看起来像「上一行没改却被染成删除」。 */
function codeOf(rows: FlatRow[]): string {
  const texts: string[] = [];
  for (const row of rows) {
    if (row.kind === "line") texts.push(row.line.text);
  }
  return texts.join("\n");
}
/** 着色按行对齐：hunk 头行不参与 tokenize，这里用同一段 code 的行号索引回各行 token。
 *  返回 null = 该行不着色（回退纯文本）。索引对不上时也回退纯文本：文本来自 `line.text`，
 *  着色只是配色，宁可无色也不能让 token 里的内容顶替真实行文（那会让人误判改了什么）。 */
function tokensByRow(rows: FlatRow[], tokens: HighlightToken[][] | null): (HighlightToken[] | null)[] {
  const result: (HighlightToken[] | null)[] = [];
  let codeLine = 0;
  for (const row of rows) {
    if (row.kind === "hunk") {
      result.push(null);
      continue;
    }
    const lineTokens = tokens?.[codeLine];
    codeLine += 1;
    // 自检：token 拼接必须等于该行原文。不等说明文本与索引已错位，只回退颜色不回退文本。
    const joined = lineTokens ? lineTokens.map((token) => token.content).join("") : null;
    result.push(joined === row.line.text ? (lineTokens ?? null) : null);
  }
  return result;
}

/** 变更统计行：`+N −M`；零增零删时不显示（无变化不该出现 0 0 这种噪音）。 */
export function DiffStat({ diff }: { diff: DiffResult }): React.JSX.Element | null {
  if (diff.additions === 0 && diff.deletions === 0) return null;
  return (
    <span className="shrink-0 font-mono text-ui-xs">
      <span className="text-diff-added">+{diff.additions}</span>
      <span className="ml-1 text-diff-removed">−{diff.deletions}</span>
    </span>
  );
}

/** diff 视图：hunk 头 + 双行号 + 标记 + 着色正文；容器限高、行内横向滚动。 */
export function ToolDiff({ diff, path }: { diff: DiffResult; path: string }): React.JSX.Element {
  const language = useMemo(() => languageForPath(path), [path]);
  const allRows = useMemo(() => flatten(diff.hunks), [diff.hunks]);
  const rows = useMemo(
    () => (allRows.length > MAX_RENDERED_LINES ? allRows.slice(0, MAX_RENDERED_LINES) : allRows),
    [allRows],
  );
  const code = useMemo(() => codeOf(rows), [rows]);
  const [tokens, setTokens] = useState<HighlightToken[][] | null>(null);
  // 主题取 store 里已解析的结果（lib/theme 的 applyTheme 写回 themeResolved）：
  // 深浅切换即时重新着色，与主题偏好是否「跟随系统」无关，与 markdown 代码块取态一致。
  const dark = useApp((s) => s.themeResolved === "theme-craft-dark");

  useEffect(() => {
    let alive = true;
    setTokens(null);
    if (!language) return;
    void highlightLines(code, language, dark)
      .then((result) => {
        if (alive) setTokens(result);
      })
      // 着色失败只退化为纯文本：diff 本体是信息，颜色是增强。
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [code, dark, language]);

  const rowTokens = useMemo(() => tokensByRow(rows, tokens), [rows, tokens]);
  const omitted = allRows.length - rows.length;

  return (
    <div className="overflow-hidden rounded-lg border border-border/60 bg-surface">
      {diff.truncated && diff.hunks.length === 0 ? (
        <div className="px-3 py-2 text-ui-sm text-foreground-subtle">
          差异过大，仅显示统计：<DiffStat diff={diff} />
        </div>
      ) : (
        <div className="max-h-80 overflow-auto">
          <div className="min-w-full w-max font-mono text-ui-sm leading-relaxed">
            {rows.map((row, index) => {
              if (row.kind === "hunk") {
                return (
                  <div key={`h${index}`} className="select-none px-3 py-0.5 text-ui-xs text-foreground-subtlest">
                    {row.header}
                  </div>
                );
              }
              const line = row.line;
              const style = KIND_STYLE[line.kind];
              return (
                <div key={`l${index}`} className={`flex min-w-full ${style.row}`}>
                  <span className={`w-10 shrink-0 select-none px-1 text-right tabular-nums ${style.gutter}`}>
                    {line.old || ""}
                  </span>
                  <span className={`w-10 shrink-0 select-none px-1 text-right tabular-nums ${style.gutter}`}>
                    {line.new || ""}
                  </span>
                  <span className={`w-4 shrink-0 select-none text-center ${style.gutter}`}>{style.marker}</span>
                  <code className="block flex-1 whitespace-pre pr-3">
                    {/* 空行在 shiki 里是空 token 数组（不是缺行）：`?.` 后接 `??` 会在
                        空数组时误判为「无 token」而回退原文，这里显式区分「没有该行」与「该行为空」。 */}
                    {(rowTokens[index] ?? [{ content: line.text }]).map((token, i) => (
                      <span key={i} style={token.color ? { color: token.color } : undefined}>
                        {token.content}
                      </span>
                    ))}
                  </code>
                </div>
              );
            })}
            {omitted > 0 ? (
              <div className="px-3 py-1 text-ui-xs text-foreground-subtlest">…（还有 {omitted} 行）</div>
            ) : null}
          </div>
        </div>
      )}
    </div>
  );
}
