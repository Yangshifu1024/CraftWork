/**
 * 左侧边栏。
 *
 * 职责：
 * - 品牌区与 Code / Work 模式切换；切换模式会同步会话列表与工作区视图。
 * - 工作区卡片：显示当前工作区、信任状态，并提供信任切换与工作区切换浮层。
 * - 会话列表：支持标题搜索（带防抖）、标题命中高亮、状态与 token 展示。
 * - 底部入口：用量、审计、命令面板与设置，并显示今日用量与审计条数。
 *
 * 说明：搜索只按标题高亮；“内容命中”标记表示标题没匹配但正文命中了查询词。
 */
import { useEffect, useRef, useState } from "react";
import { BrandMark } from "../../components/BrandMark";
import { Icon } from "../../components/Icon";
import { Popover } from "../../components/Popover";
import { api } from "../../lib/api";
import { copySessionID, deleteSessionViaConfirm, renameSessionViaPrompt } from "../../lib/sessionActions";
import type { ThreadView } from "../../lib/types";
import { cn, formatTime, formatTokens } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceStore } from "../../stores/workspace";

/** 侧栏当前打开的浮层类型；null 表示没有浮层。 */
type PopoverKind = "trust" | "switch" | "session" | null;

/**
 * 左侧边栏组件。
 *
 * props：无，模式、工作区、会话与界面状态都来自全局 store。
 *
 * 关键状态：
 * - `searchOpen` / `query`：搜索框是否展开与当前关键词。
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
  const setActiveWorkspace = useAppStore((state) => state.setActiveWorkspace);
  const pickAndAddWorkspace = useAppStore((state) => state.pickAndAddWorkspace);

  const threads = useThreadStore((state) => state.threads[mode]);
  const activeID = useThreadStore((state) => state.activeID[mode]);
  const searchResults = useThreadStore((state) => state.searchResults);
  const selectThread = useThreadStore((state) => state.select);
  const createThread = useThreadStore((state) => state.create);
  const searchThreads = useThreadStore((state) => state.search);
  const clearSearch = useThreadStore((state) => state.clearSearch);

  const toast = useUiStore((state) => state.toast);
  const askConfirm = useUiStore((state) => state.askConfirm);
  const openPalette = useUiStore((state) => state.openPalette);
  const openSettings = useUiStore((state) => state.openSettings);
  const openData = useUiStore((state) => state.openData);

  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [popover, setPopover] = useState<{ kind: PopoverKind; anchor: HTMLElement | null }>({
    kind: null,
    anchor: null,
  });
  // 会话右键菜单的目标会话与锚点元素；非空时渲染菜单浮层。
  const [menu, setMenu] = useState<{ thread: ThreadView; anchor: HTMLElement } | null>(null);
  const [counts, setCounts] = useState({ usage: 0, audit: 0 });
  const searchInputRef = useRef<HTMLInputElement>(null);

  // 拉取今日用量与最近审计条数；用 alive 标记避免卸载或重复请求后写入旧状态。
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

  // 搜索防抖：输入停止 180 毫秒后才请求，避免每个按键都打一次后端。
  useEffect(() => {
    if (!searchOpen) {
      return;
    }
    const timer = window.setTimeout(() => {
      void searchThreads(query);
    }, 180);
    return () => window.clearTimeout(timer);
  }, [query, searchOpen, searchThreads]);

  // 命令面板或快捷键会派发自定义事件，收到后展开搜索框并聚焦输入。
  useEffect(() => {
    function focusSearch() {
      setSearchOpen(true);
      window.setTimeout(() => searchInputRef.current?.focus(), 30);
    }
    window.addEventListener("cw:focus-thread-search", focusSearch);
    return () => window.removeEventListener("cw:focus-thread-search", focusSearch);
  }, []);

  // 搜索中显示搜索结果；没有搜索时显示当前模式的会话列表。
  const list = searchResults ?? threads ?? [];
  // 信任状态：未选择工作区时按 unknown 处理，徽标与图标都走“未选择”分支。
  const trust = activeWorkspace?.trust ?? "unknown";
  const trustLabel = trust === "trusted" ? "信任" : trust === "restricted" ? "受限模式" : "未选择";
  const trustIcon = trust === "trusted" ? "shield-check" : trust === "restricted" ? "lock" : "alert";

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
   * 工作区变化会影响文件树与会话归属，因此依次重置工作区视图、
   * 重新加载根目录，并刷新当前模式的会话列表。
   */
  async function handleSwitchWorkspace(workspaceID: string) {
    await setActiveWorkspace(workspaceID);
    useWorkspaceStore.getState().reset();
    await useWorkspaceStore.getState().loadRoot();
    await useThreadStore.getState().refresh(mode);
    closePopover();
  }

  /** 通过系统文件夹对话框添加工作区；成功后加载文件树并刷新会话列表。 */
  async function handleAddWorkspace() {
    closePopover();
    try {
      await pickAndAddWorkspace();
      await useWorkspaceStore.getState().loadRoot();
      await useThreadStore.getState().refresh(mode);
      toast({ title: "已添加工作区", tone: "ok" });
    } catch (error) {
      toast({ title: "添加工作区失败", description: String(error), tone: "danger" });
    }
  }

  /** 关闭搜索：清空关键词并让会话 store 丢弃搜索结果，列表恢复为普通会话列表。 */
  function closeSearch() {
    setSearchOpen(false);
    setQuery("");
    clearSearch();
  }

  return (
    <aside className="sidebar">
      <div className="sidebar-head">
        <a className="brand" href="#" aria-label="CraftWork 首页" onClick={(event) => event.preventDefault()}>
          <span className="brand-mark">
            <BrandMark />
          </span>
          <span className="brand-text">CraftWork</span>
        </a>
      </div>

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

      <div className="ws-card">
        <div className="ws-card-top">
          <span className="ws-avatar">
            <Icon name={mode === "work" ? "briefcase" : "folder-open"} />
          </span>
          <div className="ws-meta">
            <div className="ws-title">
              {activeWorkspace?.name ?? "选择工作区"}
              <span
                className={cn("badge", mode === "work" ? "work" : "accent")}
                style={{ height: 15, fontSize: 9 }}
              >
                {mode === "work" ? "Work" : "Code"}
              </span>
            </div>
            <div className="ws-path" title={activeWorkspace?.path}>
              {activeWorkspace?.path ?? "尚未选择工作区"}
            </div>
          </div>
        </div>
        <div className="ws-card-bottom">
          <button
            type="button"
            className={cn("trust-chip", trust)}
            title="点击调整工作区信任"
            onClick={(event) =>
              setPopover({
                kind: "trust",
                anchor: event.currentTarget,
              })
            }
          >
            <Icon name={trustIcon} />
            {trustLabel}
          </button>
          {mode === "code" ? (
            <span className="ws-mini-btn">
              <Icon name="git-branch" />
              Git 工作区
            </span>
          ) : (
            <span className="ws-mini-btn">
              <Icon name="file-text" />
              文档
            </span>
          )}
          <span className="spacer" />
          <button
            type="button"
            className="ws-mini-btn"
            title="切换工作区"
            onClick={(event) => setPopover({ kind: "switch", anchor: event.currentTarget })}
          >
            <Icon name="chevron-down" />
          </button>
        </div>
      </div>

      <div className="thread-head">
        <div className="thread-head-title">
          <span>{mode === "work" ? "Work 会话" : "Code 会话"}</span>
          <span className="count-chip">{threads?.length ?? 0}</span>
        </div>
        <div className="thread-head-actions">
          <button
            type="button"
            className="icon-btn sm"
            title="搜索会话"
            aria-label="搜索会话"
            onClick={() => {
              if (searchOpen) {
                closeSearch();
              } else {
                setSearchOpen(true);
                window.setTimeout(() => searchInputRef.current?.focus(), 30);
              }
            }}
          >
            <Icon name="search" className="ico" />
          </button>
          <button
            type="button"
            className="icon-btn sm"
            title="新建会话 ⌘N"
            aria-label="新建会话"
            onClick={() => void createThread()}
          >
            <Icon name="plus" className="ico" />
          </button>
        </div>
      </div>

      {searchOpen ? (
        <div className="thread-search">
          <Icon name="search" className="ico" />
          <input
            ref={searchInputRef}
            type="text"
            value={query}
            placeholder="搜索会话标题与内容…"
            autoComplete="off"
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                closeSearch();
              }
            }}
          />
          <button type="button" className="icon-btn sm" aria-label="关闭搜索" onClick={closeSearch}>
            <Icon name="x" className="ico" />
          </button>
        </div>
      ) : null}

      <div className="thread-list">
        {/* 列表为空时区分两种情况：搜索无结果，或确实还没有会话。 */}
        {list.length === 0 ? (
          <div className="thread-empty">
            {query ? (
              <>
                没有匹配的会话
                <br />
                <span style={{ fontSize: 10, color: "var(--text-4)" }}>试试更短的关键词</span>
              </>
            ) : (
              <>
                还没有会话
                <br />
                点击右上角 + 新建一个
              </>
            )}
          </div>
        ) : (
          list.map((thread) => (
            <ThreadItem
              key={thread.id}
              thread={thread}
              active={thread.id === activeID}
              query={query}
              onClick={() => void selectThread(thread.id)}
              onContextMenu={(anchor) => setMenu({ thread, anchor })}
            />
          ))
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
      <Popover
        anchor={popover.anchor}
        open={popover.kind === "trust"}
        onClose={closePopover}
        align="left"
      >
        <div className="popover-label">工作区信任</div>
        <button
          type="button"
          className="popover-item"
          onClick={() => void handleTrust("trusted")}
        >
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
        <button
          type="button"
          className="popover-item"
          onClick={() => void handleTrust("restricted")}
        >
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
      <Popover
        anchor={popover.anchor}
        open={popover.kind === "switch"}
        onClose={closePopover}
        align="left"
      >
        <div className="popover-label">最近工作区</div>
        {workspaces.map((workspace) => (
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
 * 会话列表中的一行。
 *
 * @param thread 会话视图数据：标题、更新时间、模型、token 与运行状态。
 * @param active 是否为当前选中会话，用于高亮。
 * @param query 当前搜索关键词，用于标题命中高亮与“内容命中”判断。
 * @param onClick 点击后选中该会话。
 * @param onContextMenu 右键标题时通知侧栏打开该会话的操作菜单，参数是条目元素，用作菜单锚点。
 *
 * 状态徽标：运行中、等待审批、错误分别用不同颜色；其余一律显示“已完成”。
 * 当标题没有命中查询词时，说明这条结果是正文命中，副标题改为“内容命中”。
 */
function ThreadItem({
  thread,
  active,
  query,
  onClick,
  onContextMenu,
}: {
  thread: ThreadView;
  active: boolean;
  query: string;
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

  /** 会话标题是否未命中查询词；未命中但出现在搜索结果里，说明是正文命中。 */
  const hit =
    Boolean(query) && !thread.title.toLowerCase().includes(query.toLowerCase());

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
      <span className="thread-title">{highlight(thread.title, query)}</span>
      <span className="thread-time">{formatTime(thread.updatedAt)}</span>
      <span className="thread-sub">
        <span className="model-name">{thread.modelName || thread.modelId}</span>
        <span className="dot-sep">·</span>
        {hit ? (
          <span style={{ color: "var(--warn)" }}>内容命中</span>
        ) : (
          <span>{formatTokens(thread.tokens)} tokens</span>
        )}
      </span>
      {statusNode}
    </button>
  );
}

/**
 * 高亮标题中第一处命中查询词的片段。
 *
 * 匹配不区分大小写，但保留原文大小写；没有查询词或没有命中时原样返回文本。
 * 返回 React 节点以便用 mark 标签包住命中部分。
 */
function highlight(text: string, query: string) {
  if (!query) {
    return text;
  }
  const index = text.toLowerCase().indexOf(query.toLowerCase());
  if (index < 0) {
    return text;
  }
  return (
    <>
      {text.slice(0, index)}
      <mark>{text.slice(index, index + query.length)}</mark>
      {text.slice(index + query.length)}
    </>
  );
}
