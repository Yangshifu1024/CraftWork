// 右侧面板：终端（xterm + PTY）/ 文件树 / 文档预览 三个视图。
// 终端是用户终端：不经过 Agent 审批；Agent 的命令执行走策略引擎。
import { useEffect, useRef, useState } from "react";
import { TerminalSquare, FolderOpen, FileText } from "lucide-react";
import { createTerminal } from "../lib/xterm";
import { api, onTerminalData, onTerminalExit } from "../lib/api";
import { useApp } from "../store/app";
import type { FileNode, ThreadView } from "../../../shared/types";

export type PanelTab = "terminal" | "files" | "doc";

/** 当前面板生效的工作区 id：活动会话优先取其工作区；草稿态（活动会话为空）取草稿槽归属
 *  （空串 = 临时槽）。查不到由调用方走各自回落（终端回落内置临时目录，文件树回落首个工作区）。 */
function effectiveWorkspaceId(activeThreadId: string, draftWsId: string, threads: ThreadView[]): string {
  if (activeThreadId) return threads.find((t) => t.id === activeThreadId)?.workspaceId ?? "";
  return draftWsId;
}

export function SidePane(): React.JSX.Element {
  const [tab, setTab] = useState<PanelTab>("terminal");
  const [open, setOpen] = useState(false);

  if (!open) {
    return (
      <div className="flex w-9 shrink-0 flex-col items-center gap-1 border-l border-border/40 py-2">
        <PaneButton label="终端" active={false} onClick={() => { setOpen(true); setTab("terminal"); }}>
          <TerminalSquare className="size-4" />
        </PaneButton>
        <PaneButton label="文件" active={false} onClick={() => { setOpen(true); setTab("files"); }}>
          <FolderOpen className="size-4" />
        </PaneButton>
        <PaneButton label="文档" active={false} onClick={() => { setOpen(true); setTab("doc"); }}>
          <FileText className="size-4" />
        </PaneButton>
      </div>
    );
  }

  return (
    <div className="flex w-[380px] min-w-[280px] shrink-0 flex-col border-l border-border/40 bg-panel">
      <div className="flex h-10 shrink-0 items-center gap-1 border-b border-border/40 px-2">
        {(
          [
            ["terminal", "终端"],
            ["files", "文件"],
            ["doc", "文档"],
          ] as const
        ).map(([id, label]) => (
          <button
            key={id}
            className={`rounded-md px-2.5 py-1 text-ui-sm ${tab === id ? "bg-selected text-foreground" : "text-foreground-subtle hover:bg-surface-hover"}`}
            onClick={() => setTab(id)}
          >
            {label}
          </button>
        ))}
        <div className="flex-1" />
        <button className="rounded-md p-1 text-foreground-subtlest hover:bg-surface-hover" onClick={() => setOpen(false)} title="收起面板">
          ×
        </button>
      </div>
      {tab === "terminal" && <TerminalView />}
      {tab === "files" && <FilesView />}
      {tab === "doc" && <DocView />}
    </div>
  );
}

function PaneButton({ label, active, onClick, children }: { label: string; active: boolean; onClick(): void; children: React.ReactNode }): React.JSX.Element {
  return (
    <button
      title={label}
      className={`flex size-7 items-center justify-center rounded-md text-ui-xs ${active ? "bg-selected text-foreground" : "text-foreground-subtlest hover:bg-surface-hover"}`}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

// ---------- 终端 ----------

let terminalSeq = 0;

function TerminalView(): React.JSX.Element {
  const containerRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<import("../lib/xterm").XtermHandle | null>(null);
  const settings = useApp((s) => s.settings);
  const workspaces = useApp((s) => s.workspaces);
  const mode = useApp((s) => s.mode);
  const activeThreadId = useApp((s) => s.activeThreadId[mode]);
  const draftWsId = useApp((s) => s.draftWsId[mode]);
  const threads = useApp((s) => s.threads[mode]);
  const [exitCode, setExitCode] = useState<number | null>(null);

  // 生效工作区：活动会话的工作区 → 草稿态按草稿槽归属（发送前即可浏览项目）→ 回落内置临时目录。
  const wsId = effectiveWorkspaceId(activeThreadId, draftWsId, threads);
  const activeWorkspace =
    workspaces.find((w) => w.id === wsId) ?? workspaces.find((w) => w.purpose === "conversation");

  useEffect(() => {
    if (!containerRef.current) return;
    const created = createTerminal(containerRef.current, settings.terminalFontSize);
    termRef.current = created;
    const sessionId = `pty_${Date.now()}_${terminalSeq++}`;
    const offData = onTerminalData((id, data) => {
      if (id === sessionId) created.term.write(data);
    });
    const offExit = onTerminalExit((id, code) => {
      if (id === sessionId) setExitCode(code);
    });
    // 惰性创建：等首次布局测量后再 spawn
    const timer = setTimeout(() => {
      created.fit();
      const cwd = activeWorkspace?.path ?? "";
      void api.terminalCreate(sessionId, cwd, created.term.rows, created.term.cols);
    }, 60);
    const resizeObserver = new ResizeObserver(() => {
      created.fit();
      void api.terminalResize(sessionId, created.term.rows, created.term.cols);
    });
    resizeObserver.observe(containerRef.current);
    created.term.onData((data) => void api.terminalWrite(sessionId, data));
    return () => {
      clearTimeout(timer);
      resizeObserver.disconnect();
      offData();
      offExit();
      void api.terminalDispose(sessionId);
      created.term.dispose();
      termRef.current = null;
    };
    // 工作区切换时重起终端（目录冻结语义）
  }, [activeWorkspace?.id, settings.terminalFontSize]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="shrink-0 border-b border-border/30 px-3 py-1 text-ui-xs text-foreground-subtlest">
        {activeWorkspace?.path ?? "（无工作区）"} · 用户终端不经过 Agent 审批
      </div>
      <div ref={containerRef} className="min-h-0 flex-1 p-1" />
      {exitCode !== null && (
        <div className="shrink-0 border-t border-border/30 px-3 py-1 text-ui-xs text-foreground-subtle">
          进程已退出 · 退出码 {exitCode}
        </div>
      )}
    </div>
  );
}

// ---------- 文件树 ----------

function FilesView(): React.JSX.Element {
  const workspaces = useApp((s) => s.workspaces);
  const mode = useApp((s) => s.mode);
  const activeThreadId = useApp((s) => s.activeThreadId[mode]);
  const draftWsId = useApp((s) => s.draftWsId[mode]);
  const threads = useApp((s) => s.threads[mode]);
  const [pickedId, setPickedId] = useState("");
  // 默认选中跟随当前归属（活动会话 / 草稿槽），用户下拉选择后尊重手动选择。
  const wsId = pickedId || effectiveWorkspaceId(activeThreadId, draftWsId, threads);
  const ws = workspaces.find((w) => w.id === wsId) ?? workspaces[0];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <select
        value={ws?.id ?? ""}
        onChange={(e) => setPickedId(e.target.value)}
        className="mx-2 my-1.5 h-7 rounded-md border border-border bg-input px-2 text-ui-xs"
      >
        {workspaces.map((w) => (
          <option key={w.id} value={w.id}>
            {w.name}
          </option>
        ))}
      </select>
      <div className="min-h-0 flex-1 overflow-y-auto px-1 pb-2">
        {ws ? <DirNode workspaceId={ws.id} folderId="" rel="" depth={0} name={ws.name} /> : (
          <div className="px-3 py-8 text-center text-ui-sm text-foreground-subtlest">没有工作区</div>
        )}
      </div>
    </div>
  );
}

function DirNode({ workspaceId, folderId, rel, depth, name }: { workspaceId: string; folderId: string; rel: string; depth: number; name: string }): React.JSX.Element {
  const [open, setOpen] = useState(depth === 0);
  const [nodes, setNodes] = useState<FileNode[] | null>(null);

  useEffect(() => {
    if (open && nodes === null) {
      void api.workspaceListDir(workspaceId, folderId, rel).then(setNodes);
    }
  }, [open, nodes, workspaceId, folderId, rel]);

  return (
    <div>
      <button
        className="flex w-full items-center gap-1 rounded-md px-1.5 py-1 text-left text-ui-sm text-foreground hover:bg-surface-hover"
        style={{ paddingLeft: `${depth * 12 + 6}px` }}
        onClick={() => setOpen(!open)}
      >
        <span className="shrink-0 text-foreground-subtlest">{open ? "▾" : "▸"}</span>
        <span className="truncate">{name}</span>
      </button>
      {open &&
        (nodes ?? []).map((node) =>
          node.type === "dir" ? (
            <DirNode key={node.path} workspaceId={workspaceId} folderId={folderId} rel={node.path} depth={depth + 1} name={node.name} />
          ) : (
            <FileLeaf key={node.path} workspaceId={workspaceId} folderId={folderId} node={node} depth={depth + 1} />
          ),
        )}
    </div>
  );
}

function FileLeaf({ workspaceId, folderId, node, depth }: { workspaceId: string; folderId: string; node: FileNode; depth: number }): React.JSX.Element {
  const [preview, setPreview] = useState<string | null>(null);
  return (
    <div>
      <button
        className="flex w-full items-center gap-1 rounded-md px-1.5 py-1 text-left text-ui-sm text-foreground-subtle hover:bg-surface-hover"
        style={{ paddingLeft: `${depth * 12 + 6}px` }}
        onClick={() => {
          if (preview !== null) {
            setPreview(null);
            return;
          }
          void api.documentPreview(workspaceId, folderId, node.path).then((p) => setPreview(p.text || p.note || "（空）"));
        }}
      >
        <span className="truncate">{node.name}</span>
      </button>
      {preview !== null && (
        <pre className="mx-2 my-1 max-h-60 overflow-auto whitespace-pre-wrap rounded-md bg-surface p-2 font-mono text-ui-xs text-foreground-subtle">
          {preview}
        </pre>
      )}
    </div>
  );
}

// ---------- 文档预览 ----------

function DocView(): React.JSX.Element {
  const [text, setText] = useState("在文件树中点击文件可预览；docx / xlsx / pdf 提取文本层。");
  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-3">
      <pre className="whitespace-pre-wrap break-words font-mono text-ui-xs text-foreground-subtle">{text}</pre>
      <span className="hidden">{setText.name}</span>
    </div>
  );
}
