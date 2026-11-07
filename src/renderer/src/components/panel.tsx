// 右侧面板：文件树 / 文档预览 / 计划 三个视图（radix Tabs，非活动视图卸载）。
// 收起时退化为窄图标列，点击图标展开对应视图；文件树按工作区浏览，文件点击做文本级预览；
// 计划视图按路径读取计划正文（planRead）内嵌渲染，不再经系统外部应用打开。
import { useEffect, useState } from "react";
import { Tabs } from "radix-ui";
import { FolderOpen, FileText, ListChecks } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../store/app";
import { useToasts } from "../store/toast";
import { Markdown } from "./markdown";
import type { FileNode, ThreadView, WorkspaceView } from "../../../shared/types";

export type PanelTab = "files" | "doc" | "plan";

/** 当前面板生效的工作区 id：活动会话的工作区 → 草稿态取草稿槽归属（draftWsId 非空时，发送前即可浏览项目）
 *  → 回落内置临时目录（conversation 工作区）。归属查不到（如项目已删）同样回落。 */
function effectiveWorkspaceId(activeThreadId: string, draftWsId: string, threads: ThreadView[], workspaces: WorkspaceView[]): string {
  const direct = activeThreadId
    ? (threads.find((t) => t.id === activeThreadId)?.workspaceId ?? "")
    : draftWsId;
  const found = direct ? workspaces.find((w) => w.id === direct) : undefined;
  return (found ?? workspaces.find((w) => w.purpose === "conversation"))?.id ?? "";
}

/** 右侧面板：收起时是窄图标列，展开后按 Tabs 切换文件树 / 文档预览 / 计划视图。 */
export function SidePane(): React.JSX.Element {
  const [tab, setTab] = useState<PanelTab>("files");
  const [open, setOpen] = useState(false);

  // 计划视图打开意图（计划卡「查看计划」触发）：展开面板并切到计划视图。
  // 意图不清空在这里做——PlanView 此时还未挂载（radix Tabs 非活动视图卸载），
  // 由 PlanView 挂载后按路径加载正文，再调用 clearPlanPanelRequest 消费掉。
  const planRequest = useApp((s) => s.planPanelRequest);
  useEffect(() => {
    if (!planRequest) return;
    setOpen(true);
    setTab("plan");
  }, [planRequest]);

  if (!open) {
    return (
      <div className="flex w-9 shrink-0 flex-col items-center gap-1 border-l border-border/40 py-2">
        <PaneButton label="文件" active={false} onClick={() => { setOpen(true); setTab("files"); }}>
          <FolderOpen className="size-4" />
        </PaneButton>
        <PaneButton label="文档" active={false} onClick={() => { setOpen(true); setTab("doc"); }}>
          <FileText className="size-4" />
        </PaneButton>
        <PaneButton label="计划" active={false} onClick={() => { setOpen(true); setTab("plan"); }}>
          <ListChecks className="size-4" />
        </PaneButton>
      </div>
    );
  }

  return (
    <Tabs.Root
      value={tab}
      onValueChange={(v) => setTab(v as PanelTab)}
      className="flex w-[380px] min-w-[280px] shrink-0 flex-col border-l border-border/40 bg-panel"
    >
      <Tabs.List className="flex h-10 shrink-0 items-center gap-1 border-b border-border/40 px-2">
        {(
          [
            ["files", "文件"],
            ["doc", "文档"],
            ["plan", "计划"],
          ] as const
        ).map(([id, label]) => (
          <Tabs.Trigger
            key={id}
            value={id}
            className="rounded-md px-2.5 py-1 text-ui-sm outline-none data-[state=active]:bg-selected data-[state=active]:text-foreground data-[state=inactive]:text-foreground-subtle hover:bg-surface-hover"
          >
            {label}
          </Tabs.Trigger>
        ))}
        <div className="flex-1" />
        <button className="rounded-md p-1 text-foreground-subtlest hover:bg-surface-hover" onClick={() => setOpen(false)} title="收起面板">
          ×
        </button>
      </Tabs.List>
      {/* 条件卸载语义与迁移前一致：非活动视图卸载 */}
      <Tabs.Content value="files" className="flex min-h-0 flex-1 flex-col outline-none">
        <FilesView />
      </Tabs.Content>
      <Tabs.Content value="doc" className="flex min-h-0 flex-1 flex-col outline-none">
        <DocView />
      </Tabs.Content>
      <Tabs.Content value="plan" className="flex min-h-0 flex-1 flex-col outline-none">
        <PlanView />
      </Tabs.Content>
    </Tabs.Root>
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

// ---------- 文件树 ----------

/** 文件树视图：顶部工作区下拉（默认跟随当前归属，手动选择后尊重手动值）+ 懒加载目录树。 */
function FilesView(): React.JSX.Element {
  const workspaces = useApp((s) => s.workspaces);
  const mode = useApp((s) => s.mode);
  const activeThreadId = useApp((s) => s.activeThreadId[mode]);
  const draftWsId = useApp((s) => s.draftWsId[mode]);
  const threads = useApp((s) => s.threads[mode]);
  const [pickedId, setPickedId] = useState("");
  // 默认选中跟随当前归属（活动会话工作区 / 草稿槽归属 / 内置临时目录），用户下拉选择后尊重手动选择。
  const wsId = pickedId || effectiveWorkspaceId(activeThreadId, draftWsId, threads, workspaces);
  const ws = workspaces.find((w) => w.id === wsId);

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

/** 目录节点：懒加载——首次展开（nodes === null）才请求子项；根节点默认展开。 */
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

/** 文件叶子：点击做文本级预览（docx / xlsx / pdf 提取文本层），再次点击收起。 */
function FileLeaf({ workspaceId, folderId, node, depth }: { workspaceId: string; folderId: string; node: FileNode; depth: number }): React.JSX.Element {
  const [preview, setPreview] = useState<string | null>(null);
  const pushToast = useToasts((s) => s.push);
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
          void api
            .documentPreview(workspaceId, folderId, node.path)
            .then((p) => setPreview(p.text || p.note || "（空）"))
            .catch((err) => pushToast("danger", "文档预览失败", String(err)));
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

/** 文档预览视图：当前为静态引导文案（预览入口在文件树，此处留作后续扩展）。 */
function DocView(): React.JSX.Element {
  const [text, setText] = useState("在文件树中点击文件可预览；docx / xlsx / pdf 提取文本层。");
  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-3">
      <pre className="whitespace-pre-wrap break-words font-mono text-ui-xs text-foreground-subtle">{text}</pre>
      <span className="hidden">{setText.name}</span>
    </div>
  );
}

// ---------- 计划 ----------

/** 计划视图：按路径读取计划正文（planRead，白名单校验在后端）并内嵌 Markdown 渲染。
 *  路径来源为计划卡「查看计划」的意图请求：挂载后按请求路径加载，加载完清空意图
 *  （重复点击同一路径时 token 自增，effect 依赖请求对象整体，仍会重跑再取一次）。
 *  无请求时给出引导文案。 */
function PlanView(): React.JSX.Element {
  const planRequest = useApp((s) => s.planPanelRequest);
  const clearPlanPanelRequest = useApp((s) => s.clearPlanPanelRequest);
  const pushToast = useToasts((s) => s.push);
  const [text, setText] = useState("");

  useEffect(() => {
    if (!planRequest) return;
    let cancelled = false;
    void api
      .planRead(planRequest.path)
      .then((content) => {
        if (!cancelled) setText(content);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setText("");
          pushToast("danger", "读取计划失败", String(err));
        }
      })
      .finally(() => {
        if (!cancelled) clearPlanPanelRequest();
      });
    return () => {
      cancelled = true;
    };
  }, [planRequest, clearPlanPanelRequest, pushToast]);

  return (
    <div className="min-h-0 flex-1 overflow-y-auto p-3">
      {text ? (
        <Markdown text={text} />
      ) : (
        <div className="flex h-full items-center justify-center px-6 text-center text-ui-sm text-foreground-subtlest">
          暂无计划：计划模式生成的计划会在这里展示
        </div>
      )}
    </div>
  );
}
