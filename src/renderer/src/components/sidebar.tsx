// 会话侧栏：结构与视觉对齐参考侧栏的分层——
// 顶部拖拽留白 → 模式切换（Code / Work 两段 pill）→ 导航按钮列（新建对话 / 数据 / 设置）→ 列表区（标题行 + 滚动体）→ 底部 footer。
// 任务行：16px 状态槽 + 标题 + 右侧相对时间；hover bg-surface-hover、激活 bg-selected。
import { useMemo, useState } from "react";
import {
  MessageCirclePlus,
  Search,
  Copy,
  Pencil,
  Trash2,
  LoaderCircle,
  Settings,
  User,
  ChartColumn,
  Folder,
  FolderOpen,
  FolderPlus,
  ChevronDown,
  ChevronRight,
  Plus,
} from "lucide-react";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";
import { useToasts } from "../store/toast";
import { api } from "../lib/api";
import { OverflowTitle } from "./overflow-title";
import { ModeSwitch } from "./mode-switch";

/** 相对时间：刚刚 / N 分钟 / N 小时 / N 天前，超过 30 天回退「M/D」。 */
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

/** 会话行 props。 */
interface ThreadRowProps {
  thread: ReturnType<typeof useApp.getState>["threads"]["code"][number];
  active: boolean;
  onSelect(): void;
}

/** 会话行：状态槽（运行中旋转 / 等待审批黄点 / 出错红点）+ 标题 + 相对时间；右键弹菜单（复制 ID / 重命名 / 删除）。 */
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
              <OverflowTitle text={thread.title} className="min-w-0 flex-1 text-ui-base text-foreground" />
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
            className="fixed z-[60] flex min-w-32 max-w-64 flex-col gap-0.5 overflow-hidden rounded-lg border border-popover-border bg-menu p-1 shadow-md"
            style={{ left: menu.x, top: menu.y }}
          >
            <button
              className="flex items-center gap-2 rounded-md px-2 py-1.5 text-ui-base hover:bg-menu-hover"
              onClick={() => {
                void api.clipboardWrite(thread.id);
                setMenu(null);
              }}
              title={thread.id}
            >
              <Copy className="size-3.5 shrink-0" /> 复制会话 ID
            </button>
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

/** 项目分组头：文件夹图标随开合切换、组内任务缩进 pl-2 pt-1；hover 显示「+」在该项目下新建会话；右键重命名 / 删除。 */
function ProjectGroup({
  name,
  workspaceId,
  threadCount,
  isOpen,
  onToggle,
  onNewDraft,
  children,
  hasUnread,
}: {
  name: string;
  workspaceId: string;
  threadCount: number;
  isOpen: boolean;
  onToggle(): void;
  onNewDraft(): void;
  children: React.ReactNode;
  hasUnread?: boolean;
}): React.JSX.Element {
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const openPrompt = useOverlays((s) => s.openPrompt);
  const openConfirm = useOverlays((s) => s.openConfirm);
  const refreshWorkspaces = useApp((s) => s.refreshWorkspaces);
  const refreshThreads = useApp((s) => s.refreshThreads);
  const pushToast = useToasts((s) => s.push);

  const rename = (value: string): void => {
    api.workspaceRename(workspaceId, value)
      .then(() => refreshWorkspaces())
      .catch((err) => pushToast("danger", "重命名项目失败", err instanceof Error ? err.message : String(err)));
  };

  const remove = (): void => {
    // 级联删除其下会话：两个模式的会话列表都要刷新，避免内存残留已删会话
    api.workspaceRemove(workspaceId)
      .then(() => Promise.all([refreshWorkspaces(), refreshThreads("code"), refreshThreads("work")]))
      .catch((err) => pushToast("danger", "删除项目失败", err instanceof Error ? err.message : String(err)));
  };

  return (
    <div>
      <div className="group/project-row ml-2 flex h-7 min-w-0 items-center gap-1 rounded-lg pr-1 hover:bg-surface-hover">
        <button
          className="flex h-7 min-w-0 flex-1 cursor-pointer items-center justify-start gap-2 rounded-lg pl-2.5 pr-1 text-left"
          onClick={onToggle}
          onContextMenu={(e) => {
            e.preventDefault();
            setMenu({ x: e.clientX, y: e.clientY });
          }}
        >
          {/* 名称起点与「任务列表」标题对齐（pl-2.5 = 18px 左对齐线）；行首槽是文件夹图标，开合切开盖/合盖，折叠靠整行点击 */}
          <span className="flex size-4 shrink-0 items-center justify-center text-foreground-subtle">
            {isOpen ? <FolderOpen className="size-4" /> : <Folder className="size-4" />}
          </span>
          <OverflowTitle text={name} className="min-w-0 text-ui-base text-foreground-subtle" />
          {!isOpen && hasUnread ? <span className="size-1.5 shrink-0 rounded-full bg-icon-blue" /> : null}
        </button>
        {/* 「在该项目下新建会话」：hover 才显示，避免布局跳动；在折叠按钮外，点击不触发开合 */}
        <button
          className="mr-0.5 flex size-6 shrink-0 items-center justify-center rounded-md text-foreground-subtle opacity-0 transition-opacity duration-150 hover:bg-hover hover:text-foreground group-hover/project-row:opacity-100"
          title="在该项目下新建会话"
          onClick={onNewDraft}
        >
          <Plus className="size-3.5" />
        </button>
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
                  title: "重命名项目",
                  message: "新名称会立即保存。",
                  value: name,
                  onOk: rename,
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
                  title: "删除项目",
                  message:
                    threadCount > 0
                      ? `将删除项目「${name}」及其下 ${threadCount} 条会话（含全部消息与审批记录），无法恢复。`
                      : `将删除项目「${name}」，无法恢复。`,
                  danger: true,
                  onOk: remove,
                });
              }}
            >
              <Trash2 className="size-3.5" /> 删除
            </button>
          </div>
        </>
      )}
      {isOpen && <div className="pl-2 pt-1">{children}</div>}
    </div>
  );
}

/** 会话侧栏：新建对话入口 + 分组列表（临时会话置顶、项目分组可折叠、每组超 5 条折叠为「显示更多」）+ 底部 footer。 */
export function Sidebar(): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const threads = useApp((s) => s.threads[mode]);
  const workspaces = useApp((s) => s.workspaces);
  const activeId = useApp((s) => s.activeThreadId[mode]);
  const sidebarVisible = useApp((s) => s.sidebarVisible);
  const selectThread = useApp((s) => s.selectThread);
  const newDraft = useApp((s) => s.newDraft);
  const open = useOverlays((s) => s.open);
  const overlay = useOverlays((s) => s.overlay);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [showAll, setShowAll] = useState<Set<string>>(new Set());

  /** 每组默认可见条数，超出折叠为「显示更多」（对齐 ZCode 任务列表）。 */
  const VISIBLE_LIMIT = 5;

  const toggleGroup = (key: string): void => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const toggleShowAll = (key: string): void => {
    setShowAll((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  /** 新建项目入口：打开「创建项目」对话框（名称/多文件夹/信任确认），创建成功由对话框收尾（刷新列表 + 进入项目草稿态）。 */
  const openCreateProject = useOverlays((s) => s.openCreateProject);

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
    // 以工作区为主序：空项目（无会话）也显示分组，才能从「+」进入该项目草稿槽；
    // 工作区已删的孤儿会话兜底进「其他」。
    const projectGroups = workspaces
      .filter((w) => w.purpose === "project")
      .map((w) => ({ name: w.name, workspaceId: w.id, threads: byWs.get(w.id) ?? [] }));
    for (const [wsId, list] of byWs) {
      if (!workspaces.some((w) => w.id === wsId)) projectGroups.push({ name: "其他", workspaceId: wsId, threads: list });
    }
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
        <div className="h-12 shrink-0" data-tauri-drag-region />
        <div className="relative min-h-0 flex-1 overflow-clip">
          <div className="absolute inset-0 flex min-h-0 flex-col">
            {/* 导航区：模式切换（Code 蓝 / Work 黄）在最顶部，其下为新建对话 */}
            <div className="flex flex-col gap-3 px-2 py-3">
              <ModeSwitch />
              <div
                className="group inline-flex h-8 w-full shrink-0 cursor-pointer items-center justify-stretch gap-2 overflow-hidden rounded-lg pl-2.5 pr-2.5 hover:bg-surface-hover hover:text-foreground"
                onClick={() => newDraft()}
              >
                <div className="flex min-w-0 flex-1 items-center gap-2 text-ui-base">
                  <MessageCirclePlus className="h-4 w-4 shrink-0" />
                  <span className="truncate">新建对话</span>
                  <span className="ml-auto shrink-0 text-ui-xs font-normal text-foreground-subtlest">⌘N</span>
                </div>
              </div>
            </div>
            {/* 列表区 */}
            <div className="flex min-h-0 flex-1 flex-col border-t border-border/50 pt-3">
              <div className="flex min-w-0 shrink-0 items-center gap-1 px-2 pb-2">
                <span className="min-w-0 flex-1 truncate px-2.5 text-ui-base font-medium text-foreground-subtle">
                  任务列表
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
                  {/* 临时会话在顶部，项目分组在后；每组超 5 条折叠为「显示更多」 */}
                  {groups.temp.length > 0 && (
                    <section>
                      {/* 分组头：与项目分组一致可折叠；文字对齐 18px 左线，箭头跟随右侧 */}
                      <div className="group/temp-row ml-2 flex h-7 min-w-0 items-center rounded-lg hover:bg-surface-hover">
                        <button
                          className="flex h-7 min-w-0 flex-1 cursor-pointer items-center gap-1 rounded-lg pl-2.5 pr-1 text-left"
                          onClick={() => toggleGroup(groups.conversationWs?.id ?? "temp")}
                        >
                          <span className="min-w-0 truncate text-ui-base font-medium text-foreground-subtlest">
                            临时会话
                          </span>
                          <span className="flex size-4 shrink-0 items-center justify-center text-foreground-subtlest opacity-0 transition-opacity duration-150 group-hover/temp-row:opacity-100">
                            {collapsed.has(groups.conversationWs?.id ?? "temp") ? (
                              <ChevronRight className="size-3.5" />
                            ) : (
                              <ChevronDown className="size-3.5" />
                            )}
                          </span>
                        </button>
                      </div>
                      {!collapsed.has(groups.conversationWs?.id ?? "temp") && (
                        <>
                          <ul className="ml-2 space-y-0.5 pt-1">
                            {(showAll.has("temp") ? groups.temp : groups.temp.slice(0, VISIBLE_LIMIT)).map((t) => (
                              <ThreadRowItem
                                key={t.id}
                                thread={t}
                                active={t.id === activeId}
                                onSelect={() => void selectThread(t.id)}
                              />
                            ))}
                          </ul>
                          {groups.temp.length > VISIBLE_LIMIT && (
                            <button
                              className="mt-0.5 flex h-7 w-full cursor-pointer items-center rounded-lg pl-8.5 text-left text-ui-sm text-foreground-subtle hover:text-foreground"
                              onClick={() => toggleShowAll("temp")}
                            >
                              {showAll.has("temp") ? "收起" : "显示更多"}
                            </button>
                          )}
                        </>
                      )}
                    </section>
                  )}
                  {/* 「项目」分组：常显（空态也能从分组头新建项目），可折叠头，折叠即收起其下全部项目 */}
                  {
                    <section>
                      <div className="group/proj-sec ml-2 flex h-7 min-w-0 items-center rounded-lg hover:bg-surface-hover">
                        <button
                          className="flex h-7 min-w-0 flex-1 cursor-pointer items-center gap-1 rounded-lg pl-2.5 pr-1 text-left"
                          onClick={() => toggleGroup("projects")}
                        >
                          {/* 名称起点对齐 18px 左线，折叠箭头跟随文字右侧（hover 显示） */}
                          <span className="min-w-0 truncate text-ui-base font-medium text-foreground-subtlest">
                            项目
                          </span>
                          <span className="flex size-4 shrink-0 items-center justify-center text-foreground-subtlest opacity-0 transition-opacity duration-150 group-hover/proj-sec:opacity-100">
                            {collapsed.has("projects") ? (
                              <ChevronRight className="size-3.5" />
                            ) : (
                              <ChevronDown className="size-3.5" />
                            )}
                          </span>
                        </button>
                        {/* 新建项目入口：hover 分组头才显示（从「任务列表」区头移入使用场景附近） */}
                        <button
                          className="flex size-6 shrink-0 items-center justify-center rounded-md text-foreground-subtle opacity-0 transition-opacity duration-150 hover:bg-hover hover:text-foreground group-hover/proj-sec:opacity-100"
                          title="新建项目"
                          onClick={openCreateProject}
                        >
                          <FolderPlus className="size-3.5" />
                        </button>
                      </div>
                      {!collapsed.has("projects") && (
                        <div className="flex flex-col gap-3 pt-1">
                          {groups.projectGroups.length === 0 && (
                            <div className="px-8.5 py-2 text-ui-base text-foreground-subtlest">暂无项目</div>
                          )}
                          {groups.projectGroups.map((group) => {
                    const key = group.workspaceId;
                    const isOpen = !collapsed.has(key);
                    const visible = showAll.has(key) ? group.threads : group.threads.slice(0, VISIBLE_LIMIT);
                    return (
                      <ProjectGroup
                        key={key}
                        name={group.name}
                        workspaceId={key}
                        threadCount={group.threads.length}
                        isOpen={isOpen}
                        onToggle={() => toggleGroup(key)}
                        onNewDraft={() => newDraft(key)}
                      >
                        <ul className="space-y-0.5">
                          {visible.map((t) => (
                            <ThreadRowItem
                              key={t.id}
                              thread={t}
                              active={t.id === activeId}
                              onSelect={() => void selectThread(t.id)}
                            />
                          ))}
                        </ul>
                        {group.threads.length === 0 && (
                          <div className="px-8.5 py-2 text-ui-base text-foreground-subtlest">暂无任务</div>
                        )}
                        {group.threads.length > VISIBLE_LIMIT && (
                          <button
                            className="mt-0.5 flex h-7 w-full cursor-pointer items-center rounded-lg pl-8.5 text-left text-ui-sm text-foreground-subtle hover:text-foreground"
                            onClick={() => toggleShowAll(key)}
                          >
                            {showAll.has(key) ? "收起" : "显示更多"}
                          </button>
                        )}
                      </ProjectGroup>
                          );
                        })}
                        </div>
                      )}
                    </section>
                  }
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
                  onClick={() => open("settings", "usage")}
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
                    title="用量与数据"
                    onClick={() => open("settings", "usage")}
                  >
                    <ChartColumn className="size-4" />
                  </button>
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
