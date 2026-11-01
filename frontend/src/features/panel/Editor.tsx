import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { highlightSelectionMatches, openSearchPanel, searchKeymap } from "@codemirror/search";
import {
  HighlightStyle,
  bracketMatching,
  indentOnInput,
  syntaxHighlighting,
} from "@codemirror/language";
import { tags } from "@lezer/highlight";
import { languages } from "@codemirror/language-data";
import { go } from "@codemirror/lang-go";
import { Icon } from "../../components/Icon";
import { copyText } from "../../lib/clipboard";
import { cn, formatBytes } from "../../lib/utils";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceStore } from "../../stores/workspace";

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
  const onChangeRef = useRef(setDraft);
  onChangeRef.current = setDraft;

  useEffect(() => {
    const host = hostRef.current;
    if (!host || !content || content.binary) {
      return;
    }
    let disposed = false;
    let view: EditorView | null = null;

    void (async () => {
      const language = await loadLanguage(content.path);
      if (disposed) {
        return;
      }
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
      disposed = true;
      view?.destroy();
      viewRef.current = null;
    };
  }, [content?.path, content?.hash, content?.content, content?.binary, editorMode, targetLine]);

  if (!content) {
    return null;
  }

  const crumbs = content.path.split("/");

  return (
    <div className="editor">
      <div className="editor-head">
        <div className="editor-breadcrumb" title={content.path}>
          {crumbs.map((crumb, index) => (
            <span key={`${crumb}-${index}`}>
              {index > 0 ? <span className="sep">/</span> : null}
              <span className={cn("crumb", index === crumbs.length - 1 && "last")}>{crumb}</span>
            </span>
          ))}
        </div>
        <div className="editor-badges">
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
          title={content.binary ? "二进制文件不可编辑" : content.truncated ? "文件超过 2MB，已截断，编辑已禁用" : undefined}
          onClick={() => setEditorMode("edit")}
        >
          <Icon name="edit" className="ico" />
          编辑
        </button>
        <button
          type="button"
          className="tool-btn"
          onClick={() => {
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
            void copyText(content.content);
            toast({ title: "已复制文件内容", tone: "ok" });
          }}
        >
          <Icon name="copy" className="ico" />
          复制
        </button>
        {editorMode === "edit" ? (
          <button type="button" className="tool-btn is-on" onClick={() => void save()}>
            <Icon name="save" className="ico" />
            保存
          </button>
        ) : null}
      </div>

      {content.binary ? (
        <div className="binary-view">
          <Icon name="file" size={24} />
          <div className="binary-facts">
            <div>二进制文件，无法以文本方式预览。</div>
            <div>大小：{formatBytes(content.size)}</div>
            <div>SHA-256：{content.hash}</div>
          </div>
        </div>
      ) : (
        <div className="editor-scroll">
          <div ref={hostRef} className="cm-host" />
        </div>
      )}

      <div className="editor-note">
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
