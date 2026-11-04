// 会话侧栏：结构与视觉对齐参考侧栏的分层——
// 顶部拖拽留白 → 导航按钮列（新建对话 / 数据 / 设置）→ 列表区（标题行 + 滚动体）→ 底部 footer。
// 任务行：16px 状态槽 + 标题 + 右侧相对时间；hover bg-surface-hover、激活 bg-selected。
import { useMemo, useState } from "react";
import {
  MessageCirclePlus,
  Search,
  Folder,
  FolderOpen,
  Pin,
  Pencil,
  Trash2,
  LoaderCircle,
  Database,
  Settings,
  User,
  CalendarClock,
} from "lucide-react";
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

/** 导航按钮（ghost lg 变体）：h-8 rounded-lg px-2.5 justify-start gap-2。 */
function NavButton({
  icon,
  label,
  active,
  onClick,
  trailing,
}: {
  icon: React.ReactNode;
  label: string;
  active?: boolean;
  onClick(): void;
  trailing?: string;
}): React.JSX.Element {
  return (
    <button
      className={`inline-flex h-8 w-full shrink-0 cursor-pointer select-none items-center justify-start gap-2 whitespace-nowrap rounded-lg px-2.5 text-ui-base transition-colors ${
        active ? "bg-selected text-foreground" : "text-foreground hover:bg-hover"
      }`}
      onClick={onClick}
    >
      <span className="shrink-0 [&_svg]:size-4">{icon}</span>
      <span className="truncate">{label}</span>
      {trailing && <span className="ml-auto shrink-0 text-ui-xs font-normal text-foreground-subtlest">{trailing}</span>}
    </button>
  );
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
      <LoaderCircle className="size-4 animate-spin text-foreground-subtle" />
    ) : thread.status === "awaiting" ? (
      <span className="size-[7px] rounded-full bg-warning" />
    ) : thread.status === "error" ? (
      <span className="h-1.5 w-1.5 rounded-full bg-destructive" />
    ) : null;

  return (
    <li className="py-px">
      <div
        tabIndex={0}
        onClick={onSelect}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ x: e.clientX, y: e.clientY });
        }}
        className={`group/task-item flex cursor-pointer items-center gap-2 rounded-lg py-1 pl-2.5 pr-1 transition-[background-color] ${
          active ? "bg-selected" : "hover:bg-surface-hover"
        }`}
      >
        <div className="relative flex size-4 shrink-0 items-center justify-center">{statusIcon}</div>
        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex min-w-0 items-center gap-2">
            <div className="relative flex h-6 min-w-0 flex-1 flex-wrap items-center gap-1.5">
              <span className="truncate text-ui-base text-foreground">{thread.title}</span>
            </div>
            <span className="mr-0.5 flex shrink-0 items-center gap-1 text-ui-sm text-foreground-subtle">
              {relativeTime(thread.updatedAt)}
            </span>
          </div>
        </div>
      </div>
      {menu && (
        <>
          <div
            className="fixed inset-0 z-[59]"
            onClick={() => setMenu(null)}
            onContextMenu={(e) => {
              e.preventDefault();
              setMenu(null);
            }}
          />
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

/** 项目分组头：h-8、文件夹图标随开合切换、组内任务缩进 pl-2 pt-1。 */
function ProjectGroup({
  name,
  isOpen,
  onToggle,
  children,
  hasUnread,
}: {
  name: string;
  isOpen: boolean;
  onToggle(): void;
  children: React.ReactNode;
  hasUnread?: boolean;
}): React.JSX.Element {
  const [hovered, setHovered] = useState(false);
  return (
    <div>
      <div
        className="group/project-row ml-2 flex h-8 min-w-0 items-center gap-1 rounded-lg pr-1 hover:bg-surface-hover"
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
      >
        <button
          className="flex h-8 min-w-0 flex-1 cursor-pointer justify-start gap-2 rounded-lg pl-2.5 pr-1 text-left"
          onClick={onToggle}
        >
          <span className="flex size-4 shrink-0 items-center justify-center text-foreground-subtle">
            {isOpen ? <FolderOpen className="size-4" /> : <Folder className="size-4" />}
          </span>
          <span className="min-w-0 flex-1 truncate text-ui-base text-foreground-subtle">{name}</span>
          {!isOpen && hasUnread ? <span className="mx-1 size-1.5 shrink-0 rounded-full bg-icon-blue" /> : null}
          <span className="mr-1 flex size-4 shrink-0 items-center justify-center text-foreground-subtlest">
            {hovered ? <Pin className="size-3.5" /> : null}
          </span>
        </button>
      </div>
      {isOpen && <div className="pl-2 pt-1">{children}</div>}
    </div>
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
  const overlay = useOverlays((s) => s.overlay);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const toggleGroup = (key: string): void => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const groups = useMemo(() => {
    const conversationWs = workspaces.find((w) => w.purpose === "conversation");
    const temp = conversationWs ? threads.filter((t) => t.workspaceId === conversationWs.id) : [];
    const byWs = new Map<string, typeof threads>();
    for (const t of threads) {
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
    return { conversationWs, temp, projectGroups };
  }, [threads, workspaces]);

  return (
    <div
      id="sidebar"
      className={`w-[264px] max-w-[50%] flex-none overflow-hidden bg-sidebar duration-200 ease-out transition-[width,opacity] ${
        sidebarVisible ? "opacity-100" : "pointer-events-none w-0 opacity-0"
      }`}
    >
      <aside className="flex h-full min-h-0 select-none flex-col overflow-hidden">
        <div className="h-12 shrink-0 app-region-drag" />
        <div className="relative min-h-0 flex-1 overflow-clip">
          <div className="absolute inset-0 flex min-h-0 flex-col">
            {/* 导航区 */}
            <div className="flex flex-col gap-1 px-2 py-3 app-region-no-drag">
              <div
                className="group inline-flex h-8 w-full shrink-0 cursor-pointer items-center justify-stretch gap-2 overflow-hidden rounded-lg pl-2.5 pr-2.5 hover:bg-surface-hover hover:text-foreground"
                onClick={() => void createThread()}
              >
                <div className="flex min-w-0 flex-1 items-center gap-2 text-ui-base">
                  <MessageCirclePlus className="h-4 w-4 shrink-0" />
                  <span className="truncate">新建对话</span>
                  <span className="ml-auto shrink-0 text-ui-xs font-normal text-foreground-subtlest">⌘N</span>
                </div>
              </div>
              <NavButton icon={<CalendarClock />} label="任务与提醒" onClick={() => open("data")} />
              <NavButton icon={<Database />} label="用量与数据" onClick={() => open("data")} />
            </div>
            {/* 列表区 */}
            <div className="flex min-h-0 flex-1 flex-col border-t border-border/50 pt-3">
              <div className="flex min-w-0 shrink-0 items-center gap-1 px-2 pb-2">
                <span className="min-w-0 flex-1 truncate px-2.5 text-ui-base font-medium text-foreground-subtle">
                  会话
                </span>
                <button
                  className="flex size-6 shrink-0 items-center justify-center rounded-md text-foreground-subtle hover:bg-hover hover:text-foreground"
                  title="搜索（⌘K）"
                  onClick={() => open("palette")}
                >
                  <Search className="size-3.5" />
                </button>
              </div>
              <div className="relative flex min-h-0 flex-1 flex-col overflow-x-hidden overflow-y-auto">
                <div className="flex flex-col gap-3 pb-3">
                  {groups.temp.length > 0 && (
                    <section className="px-2">
                      <h3 className="px-2.5 py-1 text-ui-base font-medium text-foreground-subtlest">临时会话</h3>
                      <ul className="space-y-0.5">
                        {groups.temp.map((t) => (
                          <ThreadRowItem
                            key={t.id}
                            thread={t}
                            active={t.id === activeId}
                            onSelect={() => void selectThread(t.id)}
                          />
                        ))}
                      </ul>
                    </section>
                  )}
                  {groups.projectGroups.map((group) => {
                    const key = group.workspaceId;
                    const isOpen = !collapsed.has(key);
                    return (
                      <ProjectGroup
                        key={key}
                        name={group.name}
                        isOpen={isOpen}
                        onToggle={() => toggleGroup(key)}
                      >
                        <ul className="space-y-0.5">
                          {group.threads.map((t) => (
                            <ThreadRowItem
                              key={t.id}
                              thread={t}
                              active={t.id === activeId}
                              onSelect={() => void selectThread(t.id)}
                            />
                          ))}
                        </ul>
                      </ProjectGroup>
                    );
                  })}
                  {threads.length === 0 && (
                    <div className="px-8.5 py-2 text-ui-base text-foreground-subtlest">还没有会话</div>
                  )}
                </div>
              </div>
            </div>
            {/* 底部 footer：账户位 + 设置齿轮 */}
            <footer className="flex shrink-0 flex-col gap-2.5 px-4 pt-2 pb-4 pr-3">
              <div className="flex min-w-0 gap-2">
                <button
                  className="flex h-8 min-w-0 flex-1 cursor-pointer items-center justify-start gap-2 overflow-hidden rounded-lg hover:bg-hover"
                  onClick={() => open("data")}
                >
                  <span className="flex size-7 shrink-0 items-center justify-center rounded-full bg-surface text-foreground-subtle">
                    <User className="size-4" />
                  </span>
                  <span className="min-w-0 flex-1 truncate text-left text-ui-base font-semibold text-foreground">
                    CraftWork
                  </span>
                </button>
                <div className="flex shrink-0 items-center gap-1.5">
                  <button
                    className={`flex size-8 items-center justify-center rounded-lg text-foreground hover:bg-hover ${
                      overlay === "settings" ? "bg-selected" : ""
                    }`}
                    title="设置（⌘,）"
                    onClick={() => open("settings")}
                  >
                    <Settings className="size-4" />
                  </button>
                </div>
              </div>
            </footer>
          </div>
        </div>
      </aside>
    </div>
  );
}

// api 供后续侧栏内联操作扩展
void api;
