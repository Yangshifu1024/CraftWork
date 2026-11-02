/**
 * 代码编辑器面板。
 *
 * 负责：用 CodeMirror 6 展示当前打开的文件，在只读与编辑之间切换，
 * 把编辑内容写回草稿，并提供搜索、复制与保存入口。
 *
 * 真正的写盘与备份由 workspace store 的 save 完成，本组件只负责编辑体验。
 */

import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { go } from "@codemirror/lang-go";
import { bracketMatching, HighlightStyle, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { highlightSelectionMatches, openSearchPanel, searchKeymap } from "@codemirror/search";
import { EditorState } from "@codemirror/state";
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { tags } from "@lezer/highlight";
import { useEffect, useRef } from "react";
import { Icon } from "../../components/Icon";
import { copyText } from "../../lib/clipboard";
import { cn, formatBytes } from "../../lib/utils";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceStore } from "../../stores/workspace";

/** 代码高亮的配色定义，颜色尽量贴近原型使用的深色主题。 */
const highlightStyle = HighlightStyle.define([
  { tag: tags.keyword, color: "#c792ea" },
  { tag: [tags.string, tags.special(tags.string)], color: "#a5d6a7" },
  { tag: [tags.number, tags.bool, tags.null], color: "#f2b77c" },
  { tag: [tags.comment, tags.lineComment, tags.blockComment], color: "var(--text-4)", fontStyle: "italic" },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], color: "#82aaff" },
  { tag: [tags.typeName, tags.className, tags.namespace], color: "#7dd3c7" },
  { tag: [tags.operator, tags.punctuation], color: "var(--text-2)" },
  { tag: [tags.propertyName, tags.attributeName], color: "#e8ebf1" },
]);

/** CodeMirror 的外观主题：字体、行高、选择区与搜索面板都复用原型的 CSS 变量。 */
const editorTheme = EditorView.theme({
  "&": {
    height: "100%",
    backgroundColor: "transparent",
    color: "var(--text)",
    fontSize: "11.5px",
  },
  ".cm-scroller": {
    fontFamily: "var(--font-mono)",
    lineHeight: "1.62",
    overflow: "auto",
  },
  ".cm-content": { padding: "9px 0", caretColor: "var(--accent)" },
  ".cm-gutters": {
    backgroundColor: "transparent",
    color: "var(--text-4)",
    border: "none",
    paddingRight: "4px",
  },
  ".cm-activeLine": { backgroundColor: "transparent" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--text-2)" },
  "&.cm-focused": { outline: "none" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--accent-soft) !important",
  },
  ".cm-cursor": { borderLeftColor: "var(--accent)" },
  ".cm-searchMatch": { backgroundColor: "var(--warn-soft)", outline: "1px solid var(--warn)" },
  ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "var(--accent-soft)" },
  ".cm-panels": { backgroundColor: "var(--bg-elevated)", color: "var(--text)" },
  ".cm-panels input, .cm-panels button": { fontSize: "11px" },
});

/**
 * 按文件扩展名加载 CodeMirror 的语言支持。
 *
 * Go 单独使用官方语言包；其他语言从 language-data 里按扩展名查找并动态加载。
 * 没有匹配或加载失败时返回 null，此时编辑器按纯文本显示。
 */
async function loadLanguage(path: string) {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  if (extension === "go") {
    return go();
  }
  const description = languages.find((item) => item.extensions.includes(extension));
  if (!description) {
    return null;
  }
  try {
    return await description.load();
  } catch {
    return null;
  }
}

/**
 * 编辑器面板组件。
 *
 * props：无。数据都来自 workspace store：
 * - content：文件内容、语言、大小、哈希，以及是否二进制 / 截断；
 * - editorMode：read 表示只读，edit 表示可编辑；
 * - targetLine：从差异或搜索跳转过来时需要定位的行号。
 *
 * 本组件持有 CodeMirror 视图与宿主节点的引用，卸载时销毁视图释放资源。
 */
export function Editor() {
  const content = useWorkspaceStore((state) => state.content);
  const editorMode = useWorkspaceStore((state) => state.editorMode);
  const setEditorMode = useWorkspaceStore((state) => state.setEditorMode);
  const setDraft = useWorkspaceStore((state) => state.setDraft);
  const save = useWorkspaceStore((state) => state.save);
  const closeFile = useWorkspaceStore((state) => state.closeFile);
  const targetLine = useWorkspaceStore((state) => state.targetLine);
  const toast = useUiStore((state) => state.toast);

  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  // 用 ref 保存最新的 setDraft，重建编辑器时不必把它放进依赖数组。
  const onChangeRef = useRef(setDraft);
  onChangeRef.current = setDraft;

  // 创建 CodeMirror 视图。
  // 文件路径、内容、哈希、编辑模式或目标行变化时整体重建：
  // 这些变化来自切换文件或保存后的回读，重建比增量同步更稳妥。
  useEffect(() => {
    const host = hostRef.current;
    // 没有内容或内容是二进制时不创建编辑器，界面会显示二进制信息。
    if (!host || !content || content.binary) {
      return;
    }
    let disposed = false;
    let view: EditorView | null = null;

    void (async () => {
      const language = await loadLanguage(content.path);
      // 语言包是异步加载的，等待期间组件可能已卸载，用 disposed 阻止继续创建视图。
      if (disposed) {
        return;
      }
      // 基础扩展：行号、活动行高亮、历史记录、括号匹配、搜索快捷键，
      // 以及由编辑模式决定的只读开关。
      const extensions = [
        lineNumbers(),
        highlightActiveLineGutter(),
        highlightActiveLine(),
        highlightSpecialChars(),
        history(),
        drawSelection(),
        indentOnInput(),
        bracketMatching(),
        highlightSelectionMatches(),
        keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap, indentWithTab]),
        editorTheme,
        syntaxHighlighting(highlightStyle),
        EditorView.updateListener.of((update) => {
          // 文档内容变化时写回草稿，由 store 记录未保存状态。
          if (update.docChanged) {
            onChangeRef.current(update.state.doc.toString());
          }
        }),
        EditorView.editable.of(editorMode === "edit"),
        EditorState.readOnly.of(editorMode !== "edit"),
      ];
      if (language) {
        extensions.push(language);
      }

      view = new EditorView({
        state: EditorState.create({ doc: content.content, extensions }),
        parent: host,
      });
      viewRef.current = view;

      if (targetLine && targetLine > 0) {
        // 需要跳转行时把光标移到该行，并滚动到视口中间；
        // 行号会先夹在文档的有效范围内，避免越界。
        const lineNumber = Math.min(Math.max(1, targetLine), view.state.doc.lines);
        const line = view.state.doc.line(lineNumber);
        view.dispatch({
          selection: { anchor: line.from },
          effects: EditorView.scrollIntoView(line.from, { y: "center" }),
        });
        view.focus();
      }
    })();

    return () => {
      // 卸载或重建前销毁旧视图，避免同一宿主节点上残留多个编辑器。
      disposed = true;
      view?.destroy();
      viewRef.current = null;
    };
    // content 以对象引用整体比较：打开文件与保存回读都会生成新对象，正好触发重建。
  }, [content, editorMode, targetLine]);

  if (!content) {
    return null;
  }

  // 面包屑：按路径分隔符拆分展示。
  const crumbs = content.path.split("/");

  return (
    <div className="editor">
      <div className="editor-head">
        {/* 面包屑显示完整路径，最后一段高亮为当前文件名 */}
        <div className="editor-breadcrumb" title={content.path}>
          {crumbs.map((crumb, index) => (
            // biome-ignore lint/suspicious/noArrayIndexKey: 路径段可能重复（如 a/b/a），拼接序号保证列表内唯一
            <span key={`${crumb}-${index}`}>
              {index > 0 ? <span className="sep">/</span> : null}
              <span className={cn("crumb", index === crumbs.length - 1 && "last")}>{crumb}</span>
            </span>
          ))}
        </div>
        <div className="editor-badges">
          {/* 状态徽标：编辑中 / 只读，以及文件语言 */}
          <span className={cn("badge", editorMode === "edit" ? "warn" : "neutral")}>
            {editorMode === "edit" ? "编辑中" : "只读"}
          </span>
          <span className="badge neutral">{content.language}</span>
        </div>
        <div className="editor-actions">
          <button type="button" className="icon-btn sm" title="关闭文件" aria-label="关闭文件" onClick={closeFile}>
            <Icon name="x" className="ico" />
          </button>
        </div>
      </div>

      <div className="editor-toolbar">
        {/* 只读 / 编辑切换；编辑按钮在二进制或超限截断的文件上禁用 */}
        <button
          type="button"
          className={cn("tool-btn", editorMode === "read" && "is-on")}
          onClick={() => setEditorMode("read")}
        >
          <Icon name="eye" className="ico" />
          只读
        </button>
        <button
          type="button"
          className={cn("tool-btn", editorMode === "edit" && "is-on")}
          disabled={content.binary || content.truncated}
          title={
            content.binary ? "二进制文件不可编辑" : content.truncated ? "文件超过 2MB，已截断，编辑已禁用" : undefined
          }
          onClick={() => setEditorMode("edit")}
        >
          <Icon name="edit" className="ico" />
          编辑
        </button>
        <button
          type="button"
          className="tool-btn"
          onClick={() => {
            // 打开 CodeMirror 自带的搜索面板
            const view = viewRef.current;
            if (view) {
              openSearchPanel(view);
            }
          }}
        >
          <Icon name="search" className="ico" />
          搜索
        </button>
        <span className="spacer" />
        <button
          type="button"
          className="tool-btn"
          onClick={() => {
            // 复制的是已加载到内存的文本内容（超限文件可能已被截断）
            void copyText(content.content);
            toast({ title: "已复制文件内容", tone: "ok" });
          }}
        >
          <Icon name="copy" className="ico" />
          复制
        </button>
        {editorMode === "edit" ? (
          /* 只在编辑态显示保存；保存会先备份原文件，并校验外部修改 */
          <button type="button" className="tool-btn is-on" onClick={() => void save()}>
            <Icon name="save" className="ico" />
            保存
          </button>
        ) : null}
      </div>

      {content.binary ? (
        /* 二进制文件不做文本渲染，只展示大小与哈希 */
        <div className="binary-view">
          <Icon name="file" size={24} />
          <div className="binary-facts">
            <div>二进制文件，无法以文本方式预览。</div>
            <div>大小：{formatBytes(content.size)}</div>
            <div>SHA-256：{content.hash}</div>
          </div>
        </div>
      ) : (
        /* CodeMirror 的挂载节点，编辑器实例在上面的 effect 中创建 */
        <div className="editor-scroll">
          <div ref={hostRef} className="cm-host" />
        </div>
      )}

      <div className="editor-note">
        {/* 底部说明按文件状态切换文案：二进制、截断、编辑态或只读 */}
        <Icon name="info" className="ico" />
        <span>
          {content.binary
            ? "二进制文件不可编辑。"
            : content.truncated
              ? "文件超过 2MB，已截断，编辑已禁用；请使用其他方式处理。"
              : editorMode === "edit"
                ? "保存前会备份原文件，外部修改会阻止覆盖。"
                : "只读预览；切换到编辑态后可修改。"}
        </span>
        <span className="spacer" />
        <span>
          {content.lines} 行 · {formatBytes(content.size)}
          {content.truncated ? " · 已截断" : ""}
        </span>
      </div>
    </div>
  );
}
