/**
 * 左侧边栏。
 *
 * 职责：
 * - 品牌区与 Code / Work 模式切换；切换模式会同步会话列表与工作区视图。
 * - 会话列表：临时会话置顶，项目会话按工作区分组（折叠、分页展示）。
 * - 底部入口：用量、审计、命令面板与设置，并显示今日用量与审计条数。
 */
import { useEffect, useState } from "react";
import { Icon } from "../../components/Icon";
import { Popover } from "../../components/Popover";
import { api } from "../../lib/api";
import { copySessionID, deleteSessionViaConfirm, renameSessionViaPrompt } from "../../lib/sessionActions";
import type { ThreadView } from "../../lib/types";
import { cn, formatTime, formatTokens } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";
import { createTempThread, resyncWorkspaceViews, switchWorkspace } from "../../stores/workspaceFlow";

/** 侧栏当前打开的浮层类型；null 表示没有浮层。 */
type PopoverKind = "trust" | "switch" | "session" | null;

/** 项目分组折叠状态的本地持久化键；只记录折叠了的项目，默认全部展开。 */
const COLLAPSED_PROJECTS_KEY = "cw-sidebar-collapsed-projects";

/** 每个项目分组默认展示的会话条数；「显示更多」每次追加的条数与此相同。 */
const PROJECT_GROUP_PAGE_SIZE = 5;

/** 从本地偏好读取折叠状态；数据损坏时按全部展开处理。 */
function readCollapsedProjects(): Record<string, boolean> {
  try {
    const raw = localStorage.getItem(COLLAPSED_PROJECTS_KEY);
    return raw ? (JSON.parse(raw) as Record<string, boolean>) : {};
  } catch {
    return {};
  }
}

/**
 * 左侧边栏组件。
 *
 * props：无，模式、工作区、会话与界面状态都来自全局 store。
 *
 * 关键状态：
 * - `collapsedProjects`：项目分组的折叠状态，持久化到本地偏好。
 * - `popover`：信任或工作区切换浮层的类型与锚点。
 * - `counts`：底部展示的今日 token 用量与审计条数，随模式或活动会话变化重新拉取。
 *
 * 交互说明：
 * - 切换到受限工作区前必须确认，因为写入与执行都会被严格限制。
 * - 切换工作区后会重置工作区视图、重新加载文件树并刷新会话列表。
 */
export function Sidebar() {
  const mode = useAppStore((state) => state.mode);
  const setMode = useAppStore((state) => state.setMode);
  const activeWorkspace = useAppStore((state) => state.activeWorkspace);
  const workspaces = useAppStore((state) => state.workspaces);
  const pickAndAddWorkspace = useAppStore((state) => state.pickAndAddWorkspace);

  const threads = useThreadStore((state) => state.threads[mode]);
  const activeID = useThreadStore((state) => state.activeID[mode]);
  const selectThread = useThreadStore((state) => state.select);
  const createThread = useThreadStore((state) => state.create);

  const toast = useUiStore((state) => state.toast);
  const askConfirm = useUiStore((state) => state.askConfirm);
  const openPalette = useUiStore((state) => state.openPalette);
  const openSettings = useUiStore((state) => state.openSettings);
  const openData = useUiStore((state) => state.openData);

  const [popover, setPopover] = useState<{ kind: PopoverKind; anchor: HTMLElement | null }>({
    kind: null,
    anchor: null,
  });
  // 会话右键菜单的目标会话与锚点元素；非空时渲染菜单浮层。
  const [menu, setMenu] = useState<{ thread: ThreadView; anchor: HTMLElement } | null>(null);
  const [counts, setCounts] = useState({ usage: 0, audit: 0 });
  // 项目分组的展开/折叠状态：键为工作区 Id，值为 true 表示折叠；写入本地偏好持久化。
  const [collapsedProjects, setCollapsedProjects] = useState<Record<string, boolean>>(readCollapsedProjects);

  useEffect(() => {
    localStorage.setItem(COLLAPSED_PROJECTS_KEY, JSON.stringify(collapsedProjects));
  }, [collapsedProjects]);

  /** 切换某个项目分组的折叠状态。 */
  function toggleProjectGroup(workspaceID: string) {
    setCollapsedProjects((state) => ({ ...state, [workspaceID]: !state[workspaceID] }));
  }

  // 拉取今日用量与最近审计条数；用 alive 标记避免卸载或重复请求后写入旧状态。
  // biome-ignore lint/correctness/useExhaustiveDependencies: mode / activeID 是有意保留的触发依赖，切换模式或会话时刷新计数
  useEffect(() => {
    let alive = true;
    Promise.all([api.audit.usage(), api.audit.list(200)])
      .then(([usage, audit]) => {
        if (!alive) {
          return;
        }
        setCounts({
          usage: usage.today.inputTokens + usage.today.outputTokens,
          audit: audit.length,
        });
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [mode, activeID]);

  // 信任状态：未选择工作区时按 unknown 处理，弹层内容据此切换。
  const trust = activeWorkspace?.trust ?? "unknown";

  // 会话列表按归属分组：内置临时目录的会话置顶成「临时会话」组，
  // 其余按工作区（最近打开顺序）分组；工作区已被删除的会话归入「其他」，
  // 避免数据从列表里凭空消失。
  const source = threads ?? [];
  const projectWorkspaces = workspaces.filter((workspace) => workspace.purpose !== "conversation");
  const conversationIDs = new Set(
    workspaces.filter((workspace) => workspace.purpose === "conversation").map((workspace) => workspace.id),
  );
  const tempThreads = source.filter((thread) => conversationIDs.has(thread.workspaceId));
  // 项目分组：每个项目工作区一组，会话按归属落入对应组；顺序跟随工作区（最近打开优先）。
  const projectGroups = projectWorkspaces.map((workspace) => ({
    workspace,
    items: source.filter((thread) => thread.workspaceId === workspace.id),
  }));
  const projectThreadCount = projectGroups.reduce((sum, group) => sum + group.items.length, 0);
  const orphanThreads = source.filter(
    (thread) =>
      thread.workspaceId !== "" &&
      !conversationIDs.has(thread.workspaceId) &&
      !projectWorkspaces.some((workspace) => workspace.id === thread.workspaceId),
  );
  // 切换浮层只列项目工作区；临时会话通过列表顶部分组的 + 新建。
  const switcherWorkspaces = projectWorkspaces;

  /** 关闭当前浮层并清空锚点。 */
  function closePopover() {
    setPopover({ kind: null, anchor: null });
  }

  /** 关闭会话右键菜单。 */
  function closeSessionMenu() {
    setMenu(null);
  }

  /**
   * 修改当前工作区的信任状态。
   *
   * 切换为受限模式是收紧权限，必须先让用户确认；
   * 切换为信任只放宽限制，审批策略仍然生效，因此不需要确认。
   * 写入成功后刷新工作区列表并提示结果。
   */
  async function handleTrust(next: "trusted" | "restricted") {
    if (!activeWorkspace) {
      return;
    }
    if (next === "restricted") {
      const confirmed = await askConfirm({
        title: "切换为受限模式？",
        description: "所有写入与执行都需要审批，网络访问默认拒绝。已发出的审计记录保留。",
        detail: activeWorkspace.path,
        confirmText: "切换为受限",
        danger: true,
      });
      if (!confirmed) {
        return;
      }
    }
    await api.workspace.setTrust(activeWorkspace.id, next);
    await useAppStore.getState().refreshWorkspaces();
    toast({
      title: next === "trusted" ? "已设为信任工作区" : "已切换为受限模式",
      description: next === "trusted" ? "审批策略仍然生效" : undefined,
      tone: next === "trusted" ? "ok" : "warn",
    });
    closePopover();
  }

  /**
   * 切换活动工作区。
   * 文件树重置、会话列表刷新等联动由 workspaceFlow 统一编排。
   */
  async function handleSwitchWorkspace(workspaceID: string) {
    await switchWorkspace(workspaceID);
    closePopover();
  }

  /** 通过系统文件夹对话框添加工作区；激活已在 store 内完成，这里只重同步依赖视图。 */
  async function handleAddWorkspace() {
    closePopover();
    try {
      await pickAndAddWorkspace();
      await resyncWorkspaceViews();
      toast({ title: "已添加工作区", tone: "ok" });
    } catch (error) {
      toast({ title: "添加工作区失败", description: String(error), tone: "danger" });
    }
  }

  /**
   * 「项目会话」区头的 + ：创建项目。
   * 选择目录后新工作区会自动激活，紧接着在该项目下新建一个会话，
   * 让「添加项目」与「开始干活」一步完成。
   */
  async function handleCreateProject() {
    try {
      await pickAndAddWorkspace();
      await resyncWorkspaceViews();
      await createThread();
      toast({ title: "已创建项目", description: "新会话已就绪", tone: "ok" });
    } catch (error) {
      toast({ title: "创建项目失败", description: String(error), tone: "danger" });
    }
  }

  return (
    <aside className="sidebar">
      {/* 空标题行：仅承担窗口拖拽区与 macOS 红绿灯占位。 */}
      <div className="sidebar-head" />

      {/* 模式切换：用 tab 语义表达两种模式，滑动指示块由 CSS 根据选中项定位。 */}
      <div className="mode-switch" role="tablist" aria-label="工作模式">
        <span className="mode-glider" aria-hidden="true" />
        <button
          type="button"
          className="mode-btn"
          role="tab"
          aria-selected={mode === "code"}
          onClick={() => void setMode("code")}
        >
          <Icon name="code" className="ico" />
          <span>Code</span>
        </button>
        <button
          type="button"
          className="mode-btn"
          role="tab"
          aria-selected={mode === "work"}
          onClick={() => void setMode("work")}
        >
          <Icon name="briefcase" className="ico" />
          <span>Work</span>
        </button>
      </div>

      <div className="thread-list">
        {source.length === 0 ? (
          <div className="thread-empty">
            还没有会话
            <br />
            在「项目会话」右侧 + 创建项目，或新建临时会话
          </div>
        ) : (
          <>
            {/* 临时会话组置顶，组头 + 一键新建并切到内置临时目录。 */}
            <div className="thread-group-head is-temp">
              <Icon name="zap" />
              <span className="thread-group-name">临时会话</span>
              <span className="thread-group-count">{tempThreads.length}</span>
              <button
                type="button"
                className="thread-group-add"
                title="新建临时会话"
                aria-label="新建临时会话"
                onClick={() => void createTempThread()}
              >
                <Icon name="plus" />
              </button>
            </div>
            {tempThreads.map((thread) => (
              <ThreadItem
                key={thread.id}
                thread={thread}
                active={thread.id === activeID}
                onClick={() => void selectThread(thread.id)}
                onContextMenu={(anchor) => setMenu({ thread, anchor })}
              />
            ))}
            {/* 项目会话区：组头提供「创建项目」入口。 */}
            <div className="thread-group-head is-section">
              <span className="thread-group-name">项目会话</span>
              <span className="thread-group-count">{projectThreadCount}</span>
              <span className="spacer" />
              <button
                type="button"
                className="thread-group-add"
                title="创建项目"
                aria-label="创建项目"
                onClick={() => void handleCreateProject()}
              >
                <Icon name="plus" />
              </button>
            </div>
            {/* 每个项目一组：文件夹图标切换折叠，组内分页展示。 */}
            {projectGroups.map(({ workspace, items }) => (
              <ProjectGroup
                key={workspace.id}
                workspace={workspace}
                items={items}
                activeID={activeID}
                collapsed={Boolean(collapsedProjects[workspace.id])}
                onToggle={() => toggleProjectGroup(workspace.id)}
                onSelect={(threadID) => void selectThread(threadID)}
                onContextMenu={(thread, anchor) => setMenu({ thread, anchor })}
              />
            ))}
            {/* 工作区已被移除的会话仍有归属组，避免从列表里凭空消失。 */}
            {orphanThreads.length > 0 ? (
              <>
                <div className="thread-group-head">
                  <Icon name="archive" />
                  <span className="thread-group-name">其他</span>
                  <span className="thread-group-count">{orphanThreads.length}</span>
                </div>
                {orphanThreads.map((thread) => (
                  <ThreadItem
                    key={thread.id}
                    thread={thread}
                    active={thread.id === activeID}
                    onClick={() => void selectThread(thread.id)}
                    onContextMenu={(anchor) => setMenu({ thread, anchor })}
                  />
                ))}
              </>
            ) : null}
          </>
        )}
      </div>

      <div className="sidebar-foot">
        <div className="side-row">
          <button type="button" className="side-link" onClick={() => openData("usage")}>
            <Icon name="zap" className="ico" />
            <span>用量</span>
            <span className="side-value">{formatTokens(counts.usage)}</span>
          </button>
          <button type="button" className="side-link" onClick={() => openData("audit")}>
            <Icon name="shield-check" className="ico" />
            <span>审计</span>
            <span className="side-value">{counts.audit}</span>
          </button>
        </div>
        <div className="side-row">
          <button type="button" className="side-link" title="命令面板 ⌘K" onClick={openPalette}>
            <Icon name="command" className="ico" />
            <span>命令面板</span>
            <span className="kbd">⌘K</span>
          </button>
          <button type="button" className="side-link" title="设置 ⌘," onClick={() => openSettings()}>
            <Icon name="settings" className="ico" />
            <span>设置</span>
            <span className="kbd">⌘,</span>
          </button>
        </div>
      </div>

      {/* 信任浮层：说明两种信任状态的差异，当前状态右侧显示勾选。 */}
      <Popover anchor={popover.anchor} open={popover.kind === "trust"} onClose={closePopover} align="left">
        <div className="popover-label">工作区信任</div>
        <button type="button" className="popover-item" onClick={() => void handleTrust("trusted")}>
          <Icon name="shield-check" />
          <span className="pi-main">
            <div className="pi-title">信任</div>
            <div className="pi-desc">R1 仍逐次审批，R3 / R4 每次确认</div>
          </span>
          {trust === "trusted" ? (
            <span className="pi-trail pi-check">
              <Icon name="check" />
            </span>
          ) : null}
        </button>
        <button type="button" className="popover-item" onClick={() => void handleTrust("restricted")}>
          <Icon name="lock" />
          <span className="pi-main">
            <div className="pi-title">受限模式</div>
            <div className="pi-desc">写入与执行必须审批，网络默认拒绝</div>
          </span>
          {trust === "restricted" ? (
            <span className="pi-trail pi-check">
              <Icon name="check" />
            </span>
          ) : null}
        </button>
        <div className="popover-note">信任状态在重启后保留；可随时撤销。</div>
      </Popover>

      {/* 工作区切换浮层：列出最近工作区，底部提供“添加工作区”。 */}
      <Popover anchor={popover.anchor} open={popover.kind === "switch"} onClose={closePopover} align="left">
        <div className="popover-label">最近工作区</div>
        {switcherWorkspaces.map((workspace) => (
          <button
            key={workspace.id}
            type="button"
            className="popover-item"
            onClick={() => void handleSwitchWorkspace(workspace.id)}
          >
            <Icon name="folder" />
            <span className="pi-main">
              <div className="pi-title">{workspace.name}</div>
              <div className="pi-desc">{workspace.path}</div>
            </span>
            {workspace.id === activeWorkspace?.id ? (
              <span className="pi-trail pi-check">
                <Icon name="check" />
              </span>
            ) : (
              <span className="pi-trail">
                <Icon name="external-link" />
              </span>
            )}
          </button>
        ))}
        <div className="popover-sep" />
        <button type="button" className="popover-item" onClick={() => void handleAddWorkspace()}>
          <Icon name="plus" />
          <span className="pi-main">
            <div className="pi-title">添加工作区…</div>
            <div className="pi-desc">使用系统原生文件夹对话框</div>
          </span>
        </button>
      </Popover>

      {/* 会话右键菜单：复制 Id、重命名与删除；重命名与删除与顶部工具栏共用同一流程。 */}
      {menu ? (
        <Popover anchor={menu.anchor} open onClose={closeSessionMenu} align="left">
          <button
            type="button"
            className="popover-item"
            onClick={() => {
              closeSessionMenu();
              void copySessionID(menu.thread);
            }}
          >
            <Icon name="copy" />
            <span className="pi-main">
              <div className="pi-title">复制会话 Id</div>
              <div className="pi-desc">用于日志排查与问题反馈</div>
            </span>
          </button>
          <button
            type="button"
            className="popover-item"
            onClick={() => {
              closeSessionMenu();
              void renameSessionViaPrompt(menu.thread);
            }}
          >
            <Icon name="edit" />
            <span className="pi-main">
              <div className="pi-title">重命名</div>
              <div className="pi-desc">修改标题，立即保存</div>
            </span>
          </button>
          <button
            type="button"
            className="popover-item danger"
            onClick={() => {
              closeSessionMenu();
              void deleteSessionViaConfirm(menu.thread);
            }}
          >
            <Icon name="trash" />
            <span className="pi-main">
              <div className="pi-title">删除</div>
              <div className="pi-desc">彻底删除消息与事件，无法恢复</div>
            </span>
          </button>
        </Popover>
      ) : null}
    </aside>
  );
}

/**
 * 单个项目的会话分组。
 *
 * 组头左侧的文件夹图标是展开 / 折叠开关，图标随状态切换（展开为
 * folder-open、折叠为 folder）；折叠状态由父组件持久化。组内默认
 * 最多展示 {@link PROJECT_GROUP_PAGE_SIZE} 条，超出时底部显示
 * 「显示更多」，每次点击追加同样条数；分页计数是组件本地状态，
 * 折叠再展开与切换其他项目都不会重置。
 */
function ProjectGroup({
  workspace,
  items,
  activeID,
  collapsed,
  onToggle,
  onSelect,
  onContextMenu,
}: {
  workspace: { id: string; name: string; path: string };
  items: ThreadView[];
  activeID?: string | null;
  collapsed: boolean;
  onToggle: () => void;
  onSelect: (threadID: string) => void;
  onContextMenu: (thread: ThreadView, anchor: HTMLElement) => void;
}) {
  const [limit, setLimit] = useState(PROJECT_GROUP_PAGE_SIZE);
  const open = !collapsed;
  const visible = items.slice(0, limit);
  const hasMore = items.length > limit;

  return (
    <div className="thread-project-group">
      <div className="thread-group-head is-project">
        <button
          type="button"
          className="thread-group-toggle"
          title={open ? "折叠项目会话" : "展开项目会话"}
          aria-expanded={open}
          onClick={onToggle}
        >
          <Icon name={open ? "folder-open" : "folder"} />
        </button>
        <span className="thread-group-name" title={workspace.path}>
          {workspace.name}
        </span>
        <span className="thread-group-count">{items.length}</span>
      </div>
      {open ? (
        <>
          {visible.map((thread) => (
            <ThreadItem
              key={thread.id}
              thread={thread}
              active={thread.id === activeID}
              onClick={() => onSelect(thread.id)}
              onContextMenu={(anchor) => onContextMenu(thread, anchor)}
            />
          ))}
          {hasMore ? (
            <button
              type="button"
              className="thread-show-more"
              onClick={() => setLimit((current) => current + PROJECT_GROUP_PAGE_SIZE)}
            >
              显示更多（还有 {items.length - limit} 条）
            </button>
          ) : null}
        </>
      ) : null}
    </div>
  );
}

/**
 * 会话列表中的一行。
 *
 * @param thread 会话视图数据：标题、更新时间、模型、token 与运行状态。
 * @param active 是否为当前选中会话，用于高亮。
 * @param onClick 点击后选中该会话。
 * @param onContextMenu 右键标题时通知侧栏打开该会话的操作菜单，参数是条目元素，用作菜单锚点。
 *
 * 状态徽标：运行中、等待审批、错误分别用不同颜色；其余一律显示“已完成”。
 */
function ThreadItem({
  thread,
  active,
  onClick,
  onContextMenu,
}: {
  thread: ThreadView;
  active: boolean;
  onClick: () => void;
  onContextMenu: (anchor: HTMLElement) => void;
}) {
  const status = thread.status;
  const statusNode =
    status === "running" ? (
      <span className="thread-status running">
        <span className="st-dot" />
        运行中
      </span>
    ) : status === "awaiting" ? (
      <span className="thread-status awaiting">
        <span className="st-dot" />
        等待审批
      </span>
    ) : status === "error" ? (
      <span className="thread-status error">
        <span className="st-dot" />
        错误
      </span>
    ) : (
      <span className="thread-status idle">已完成</span>
    );

  return (
    <button
      type="button"
      className={cn("thread-item", active && "is-active")}
      onClick={onClick}
      onContextMenu={(event) => {
        // 阻止系统默认菜单，改由应用内会话菜单接管。
        event.preventDefault();
        onContextMenu(event.currentTarget);
      }}
    >
      <span className="thread-title">{thread.title}</span>
      <span className="thread-sub">
        <span className="thread-time">{formatTime(thread.updatedAt)}</span>
        {statusNode}
      </span>
    </button>
  );
}
