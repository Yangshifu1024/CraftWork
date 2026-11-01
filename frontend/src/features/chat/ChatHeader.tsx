/**
 * 聊天区顶部工具栏。
 *
 * 职责：
 * - 展示当前线程标题、所属模式（Code / Work）与运行状态徽标。
 * - 提供右侧面板开关、重命名、复制、归档、更多操作。
 * - “更多操作”浮层提供归档与彻底删除，删除前必须二次确认。
 *
 * 线程不存在时只渲染一个空的占位头部，保证布局高度稳定。
 */
import { useState } from "react";
import { Icon } from "../../components/Icon";
import { Popover } from "../../components/Popover";
import { cn } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { usePanelStore } from "../../stores/panel";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";

/**
 * 聊天区顶部工具栏组件。
 *
 * props：无，全部数据来自应用、线程、面板与界面 store。
 *
 * 关键状态：
 * - `moreAnchor`：更多操作浮层的锚点元素，非空即表示浮层打开。
 * - `thread`：当前模式下的活动线程；找不到时提前返回空头部。
 *
 * 操作说明：
 * - 重命名通过 `askPrompt` 弹出输入框，确认后立即写回。
 * - 归档只是从列表隐藏，可恢复；删除会连同消息与事件一起删除，需确认。
 */
export function ChatHeader() {
  const mode = useAppStore((state) => state.mode);
  const threads = useThreadStore((state) => state.threads[mode]);
  const activeID = useThreadStore((state) => state.activeID[mode]);
  const rename = useThreadStore((state) => state.rename);
  const duplicate = useThreadStore((state) => state.duplicate);
  const archive = useThreadStore((state) => state.archive);
  const remove = useThreadStore((state) => state.remove);

  const panelOpen = usePanelStore((state) => state.open);
  const togglePanel = usePanelStore((state) => state.toggle);

  const askPrompt = useUiStore((state) => state.askPrompt);
  const askConfirm = useUiStore((state) => state.askConfirm);
  const toast = useUiStore((state) => state.toast);

  const [moreAnchor, setMoreAnchor] = useState<HTMLElement | null>(null);

  const thread = threads.find((item) => item.id === activeID);
  if (!thread) {
    return <div className="chat-head" />;
  }

  // 状态徽标只覆盖需要用户注意的三种状态，其余（空闲、已完成）不额外展示。
  const statusBadge =
    thread.status === "running" ? (
      <span className="badge accent">
        <span className="pulse-dot" style={{ width: 5, height: 5, borderRadius: "50%", background: "currentColor" }} />
        运行中
      </span>
    ) : thread.status === "awaiting" ? (
      <span className="badge warn">
        <Icon name="clock" />
        等待审批
      </span>
    ) : thread.status === "error" ? (
      <span className="badge danger">
        <Icon name="alert" />
        出错
      </span>
    ) : null;

  /** 弹出输入框重命名当前线程；用户取消或输入为空时不做修改。 */
  async function handleRename() {
    const next = await askPrompt({
      title: "重命名线程",
      label: "新标题会立即保存。",
      value: thread?.title ?? "",
      confirmText: "保存",
    });
    if (next && thread) {
      await rename(thread.id, next);
    }
  }

  /** 归档当前线程：从当前列表隐藏，但保留数据，可在归档中恢复。 */
  async function handleArchive() {
    if (!thread) {
      return;
    }
    await archive(thread.id, true);
    toast({ title: "线程已归档", tone: "ok" });
  }

  /**
   * 彻底删除当前线程。
   * 删除不可恢复，必须先通过确认框；确认后才调用线程 store 的 remove，
   * 并提示用户消息、事件与审批记录都已一并删除。
   */
  async function handleDelete() {
    if (!thread) {
      return;
    }
    const confirmed = await askConfirm({
      title: "彻底删除线程？",
      description: "线程的消息、事件与审批记录都会被删除，无法恢复。",
      detail: thread.title,
      confirmText: "删除",
      danger: true,
    });
    if (confirmed) {
      await remove(thread.id);
      toast({ title: "线程已删除", tone: "warn" });
    }
  }

  return (
    <div className="chat-head">
      <div className="chat-head-main">
        <span className="chat-title">{thread.title}</span>
        <span className={cn("badge", thread.mode === "work" ? "work" : "accent")}>
          {thread.mode === "work" ? "Work" : "Code"}
        </span>
        {statusBadge}
      </div>
      <div className="chat-head-actions">
        <button
          type="button"
          className="icon-btn sm"
          title={panelOpen ? "收起右侧面板" : "展开右侧面板"}
          aria-label={panelOpen ? "收起右侧面板" : "展开右侧面板"}
          onClick={togglePanel}
        >
          <Icon name="panel" className="ico" />
        </button>
        <button type="button" className="icon-btn sm" title="重命名线程" aria-label="重命名线程" onClick={() => void handleRename()}>
          <Icon name="edit" className="ico" />
        </button>
        <button
          type="button"
          className="icon-btn sm"
          title="复制线程为新线程"
          aria-label="复制线程"
          onClick={() => void duplicate(thread.id)}
        >
          <Icon name="copy" className="ico" />
        </button>
        <button type="button" className="icon-btn sm" title="归档线程" aria-label="归档线程" onClick={() => void handleArchive()}>
          <Icon name="archive" className="ico" />
        </button>
        <button
          type="button"
          className="icon-btn sm"
          title="更多操作"
          aria-label="更多操作"
          onClick={(event) => setMoreAnchor(event.currentTarget)}
        >
          <Icon name="more" className="ico" />
        </button>
      </div>

      {/* 更多操作浮层：归档可恢复，删除不可恢复，两者都从这里进入。 */}
      <Popover anchor={moreAnchor} open={Boolean(moreAnchor)} onClose={() => setMoreAnchor(null)} align="right">
        <button
          type="button"
          className="popover-item"
          onClick={() => {
            setMoreAnchor(null);
            void handleArchive();
          }}
        >
          <Icon name="archive" />
          <span className="pi-main">
            <div className="pi-title">归档线程</div>
            <div className="pi-desc">从当前列表隐藏，可在归档中恢复</div>
          </span>
        </button>
        <button
          type="button"
          className="popover-item"
          onClick={() => {
            setMoreAnchor(null);
            void handleDelete();
          }}
        >
          <Icon name="trash" />
          <span className="pi-main">
            <div className="pi-title">删除线程</div>
            <div className="pi-desc">彻底删除消息与事件，无法恢复</div>
          </span>
        </button>
        <div className="popover-sep" />
        <div className="popover-note">删除前请确认没有需要保留的审计记录。</div>
      </Popover>
    </div>
  );
}
