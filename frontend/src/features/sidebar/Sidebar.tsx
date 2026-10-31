import { useEffect, useRef, useState } from "react";
import { BrandMark } from "../../components/BrandMark";
import { Icon } from "../../components/Icon";
import { Popover } from "../../components/Popover";
import { api } from "../../lib/api";
import type { ThreadView } from "../../lib/types";
import { cn, formatTime, formatTokens } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceStore } from "../../stores/workspace";

type PopoverKind = "trust" | "switch" | null;

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
  const [counts, setCounts] = useState({ usage: 0, audit: 0 });
  const searchInputRef = useRef<HTMLInputElement>(null);

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

  useEffect(() => {
    if (!searchOpen) {
      return;
    }
    const timer = window.setTimeout(() => {
      void searchThreads(query);
    }, 180);
    return () => window.clearTimeout(timer);
  }, [query, searchOpen, searchThreads]);

  useEffect(() => {
    function focusSearch() {
      setSearchOpen(true);
      window.setTimeout(() => searchInputRef.current?.focus(), 30);
    }
    window.addEventListener("cw:focus-thread-search", focusSearch);
    return () => window.removeEventListener("cw:focus-thread-search", focusSearch);
  }, []);

  const list = searchResults ?? threads ?? [];
  const trust = activeWorkspace?.trust ?? "unknown";
  const trustLabel = trust === "trusted" ? "信任" : trust === "restricted" ? "受限模式" : "未选择";
  const trustIcon = trust === "trusted" ? "shield-check" : trust === "restricted" ? "lock" : "alert";

  function closePopover() {
    setPopover({ kind: null, anchor: null });
  }

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

  async function handleSwitchWorkspace(workspaceID: string) {
    await setActiveWorkspace(workspaceID);
    useWorkspaceStore.getState().reset();
    await useWorkspaceStore.getState().loadRoot();
    await useThreadStore.getState().refresh(mode);
    closePopover();
  }

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
          <span>{mode === "work" ? "Work 线程" : "Code 线程"}</span>
          <span className="count-chip">{threads?.length ?? 0}</span>
        </div>
        <div className="thread-head-actions">
          <button
            type="button"
            className="icon-btn sm"
            title="搜索线程"
            aria-label="搜索线程"
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
            title="新建线程 ⌘N"
            aria-label="新建线程"
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
            placeholder="搜索线程标题与内容…"
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
        {list.length === 0 ? (
          <div className="thread-empty">
            {query ? (
              <>
                没有匹配的线程
                <br />
                <span style={{ fontSize: 10, color: "var(--text-4)" }}>试试更短的关键词</span>
              </>
            ) : (
              <>
                还没有线程
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
    </aside>
  );
}

function ThreadItem({
  thread,
  active,
  query,
  onClick,
}: {
  thread: ThreadView;
  active: boolean;
  query: string;
  onClick: () => void;
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

  const hit =
    Boolean(query) && !thread.title.toLowerCase().includes(query.toLowerCase());

  return (
    <button
      type="button"
      className={cn("thread-item", active && "is-active")}
      onClick={onClick}
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
