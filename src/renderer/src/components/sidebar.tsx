// 会话侧栏：新建对话按钮 + 会话列表（临时会话置顶、项目分组、相对时间、右键操作）。
import { useMemo, useState } from "react";
import { MessageCirclePlus, Search, FolderOpen, Archive, Pin, Pencil, Trash2, LoaderCircle, CircleDot } from "lucide-react";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";
import { api } from "../lib/api";

function relativeTime(ts: number): string {
  const diff = Date.now() - ts;
  const minutes = Math.floor(diff / 60000);
  if (minutes < 1) return "刚刚";
  if (minutes < 60) return `${minutes} 分钟前`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} 小时前`;
  const days = Math.floor(hours / 24);
  if (days < 30) return `${days} 天前`;
  const d = new Date(ts);
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

interface ThreadRowProps {
  thread: ReturnType<typeof useApp.getState>["threads"]["code"][number];
  active: boolean;
  onSelect(): void;
}

function ThreadRowItem({ thread, active, onSelect }: ThreadRowProps): React.JSX.Element {
  const deleteThread = useApp((s) => s.deleteThread);
  const renameThread = useApp((s) => s.renameThread);
  const openPrompt = useOverlays((s) => s.openPrompt);
  const openConfirm = useOverlays((s) => s.openConfirm);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);

  const statusIcon =
    thread.status === "running" ? (
      <LoaderCircle className="size-3.5 animate-spin text-icon-blue" />
    ) : thread.status === "awaiting" ? (
      <CircleDot className="size-3.5 text-warning" />
    ) : null;

  return (
    <li
      tabIndex={0}
      onClick={onSelect}
      onContextMenu={(e) => {
        e.preventDefault();
        setMenu({ x: e.clientX, y: e.clientY });
      }}
      className={`group/task-item flex cursor-pointer gap-2 rounded-lg pl-2.5 pr-1 py-1 transition-[background-color] ${
        active ? "bg-selected" : "hover:bg-surface-hover"
      }`}
    >
      <div className="relative flex size-4 shrink-0 items-center justify-center">
        {statusIcon ?? (active ? <span className="size-1.5 rounded-full bg-icon-blue" /> : null)}
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex min-w-0 items-center gap-2">
          <span className="truncate text-ui-base text-foreground">{thread.title}</span>
          <span className="mr-0.5 flex shrink-0 items-center gap-1 text-ui-sm text-foreground-subtle">
            {relativeTime(thread.updatedAt)}
          </span>
        </div>
      </div>
      {menu && (
        <>
          <div className="fixed inset-0 z-[59]" onClick={() => setMenu(null)} onContextMenu={(e) => { e.preventDefault(); setMenu(null); }} />
          <div
            className="fixed z-[60] flex min-w-32 flex-col gap-0.5 overflow-hidden rounded-lg border border-popover-border bg-menu p-1 shadow-md"
            style={{ left: menu.x, top: menu.y }}
          >
            <button
              className="flex items-center gap-2 rounded-md px-2 py-1.5 text-ui-base hover:bg-menu-hover"
              onClick={() => {
                setMenu(null);
                openPrompt({
                  title: "重命名会话",
                  message: "新标题会立即保存。",
                  value: thread.title,
                  onOk: (value) => void renameThread(thread.id, value),
                });
              }}
            >
              <Pencil className="size-3.5" /> 重命名
            </button>
            <button
              className="flex items-center gap-2 rounded-md px-2 py-1.5 text-ui-base text-destructive hover:bg-menu-hover"
              onClick={() => {
                setMenu(null);
                openConfirm({
                  title: "删除会话",
                  message: `将彻底删除「${thread.title}」及其全部消息与审批记录，无法恢复。`,
                  danger: true,
                  onOk: () => void deleteThread(thread.id),
                });
              }}
            >
              <Trash2 className="size-3.5" /> 删除
            </button>
          </div>
        </>
      )}
    </li>
  );
}

export function Sidebar(): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const threads = useApp((s) => s.threads[mode]);
  const workspaces = useApp((s) => s.workspaces);
  const activeId = useApp((s) => s.activeThreadId[mode]);
  const sidebarVisible = useApp((s) => s.sidebarVisible);
  const selectThread = useApp((s) => s.selectThread);
  const createThread = useApp((s) => s.createThread);
  const open = useOverlays((s) => s.open);

  const groups = useMemo(() => {
    const filtered = threads;
    // 临时会话（内置目录）置顶，其余按工作区分组
    const conversationWs = workspaces.find((w) => w.purpose === "conversation");
    const temp = conversationWs ? filtered.filter((t) => t.workspaceId === conversationWs.id) : [];
    const byWs = new Map<string, typeof filtered>();
    for (const t of filtered) {
      if (conversationWs && t.workspaceId === conversationWs.id) continue;
      const list = byWs.get(t.workspaceId) ?? [];
      list.push(t);
      byWs.set(t.workspaceId, list);
    }
    const projectGroups = [...byWs.entries()].map(([wsId, list]) => ({
      name: workspaces.find((w) => w.id === wsId)?.name ?? "其他",
      workspaceId: wsId,
      threads: list,
    }));
    return { temp, projectGroups };
  }, [threads, workspaces]);

  return (
    <div
      id="sidebar"
      data-workspace-sidebar-panel="true"
      className={`w-[264px] max-w-[50%] flex-none overflow-hidden duration-200 ease-out transition-[width,opacity] app-region-drag ${
        sidebarVisible ? "opacity-100" : "pointer-events-none w-0 opacity-0"
      }`}
    >
      <aside className="flex h-full min-h-0 select-none flex-col overflow-hidden app-region-no-drag">
        <div className="h-12 shrink-0 app-region-drag" />
        <div className="px-2 pb-2">
          <button
            className="group flex h-8 w-full shrink-0 cursor-pointer items-center gap-2 overflow-hidden rounded-lg pl-2.5 pr-2.5 hover:bg-surface-hover hover:text-foreground"
            onClick={() => void createThread()}
          >
            <MessageCirclePlus className="size-4 shrink-0" />
            <span className="min-w-0 flex-1 truncate text-left text-ui-base">新建对话</span>
            <span className="ml-auto shrink-0 text-ui-xs font-normal text-foreground-subtlest">⌘N</span>
          </button>
        </div>
        <div className="flex items-center justify-between border-t border-border/50 px-3 pb-1 pt-3">
          <span className="text-ui-sm text-foreground-subtlest">会话</span>
          <div className="flex items-center gap-1">
            <button
              className="flex size-6 items-center justify-center rounded-md hover:bg-surface-hover"
              title="搜索会话（⌘K）"
              onClick={() => open("palette")}
            >
              <Search className="size-3.5 text-foreground-subtlest" />
            </button>
          </div>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-2 pb-3">
          <div className="flex flex-col gap-3 pt-1">
            {groups.temp.length > 0 && (
              <section>
                <div className="flex items-center gap-1.5 px-1 pb-1 text-ui-sm text-foreground-subtlest">
                  <Pin className="size-3" /> 临时会话
                </div>
                <ul>
                  {groups.temp.map((t) => (
                    <ThreadRowItem key={t.id} thread={t} active={t.id === activeId} onSelect={() => void selectThread(t.id)} />
                  ))}
                </ul>
              </section>
            )}
            {groups.projectGroups.map((group) => (
              <section key={group.workspaceId}>
                <div className="flex items-center gap-1.5 px-1 pb-1 text-ui-sm text-foreground-subtlest">
                  <FolderOpen className="size-3" /> {group.name}
                </div>
                <ul>
                  {group.threads.map((t) => (
                    <ThreadRowItem key={t.id} thread={t} active={t.id === activeId} onSelect={() => void selectThread(t.id)} />
                  ))}
                </ul>
              </section>
            ))}
            {threads.length === 0 && (
              <div className="px-3 py-8 text-center text-ui-sm text-foreground-subtlest">
                还没有会话，点击上方「新建对话」开始。
              </div>
            )}
          </div>
        </div>
        <div className="border-t border-border/50 px-2 py-1.5">
          <button
            className="flex h-7 w-full items-center gap-2 rounded-lg px-2.5 text-ui-sm text-foreground-subtle hover:bg-surface-hover"
            onClick={() => open("settings")}
          >
            <Archive className="size-3.5" /> 设置与数据
          </button>
        </div>
      </aside>
    </div>
  );
}

// api 引用保留（供后续侧栏内联操作扩展）
void api;
