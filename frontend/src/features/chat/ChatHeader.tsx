/**
 * 聊天区顶部工具栏。
 *
 * 职责：
 * - 展示当前会话标题、所属模式（Code / Work）与运行状态徽标。
 * - 提供右侧面板开关、重命名、复制、归档、更多操作。
 * - “更多操作”浮层提供归档与彻底删除，删除前必须二次确认。
 *
 * 会话不存在时只渲染一个空的占位头部，保证布局高度稳定。
 */
import { useState } from "react";
import { Icon } from "../../components/Icon";
import { Popover } from "../../components/Popover";
import { deleteSessionViaConfirm, renameSessionViaPrompt } from "../../lib/sessionActions";
import { cn } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { usePanelStore } from "../../stores/panel";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";

/**
 * 聊天区顶部工具栏组件。
 *
 * props：无，全部数据来自应用、会话、面板与界面 store。
 *
 * 关键状态：
 * - `moreAnchor`：更多操作浮层的锚点元素，非空即表示浮层打开。
 * - `thread`：当前模式下的活动会话；找不到时提前返回空头部。
 *
 * 操作说明：
 * - 重命名与删除走 `lib/sessionActions` 共享流程，与侧栏右键菜单行为一致。
 * - 归档只是从列表隐藏，可恢复；删除会连同消息与事件一起删除，需确认。
 */
export function ChatHeader() {
  const mode = useAppStore((state) => state.mode);
  const threads = useThreadStore((state) => state.threads[mode]);
  const activeID = useThreadStore((state) => state.activeID[mode]);
  const duplicate = useThreadStore((state) => state.duplicate);
  const archive = useThreadStore((state) => state.archive);

  const panelOpen = usePanelStore((state) => state.open);
  const togglePanel = usePanelStore((state) => state.toggle);

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

  /** 弹出输入框重命名当前会话；流程与侧栏右键菜单共用。 */
  function handleRename() {
    if (!thread) {
      return;
    }
    void renameSessionViaPrompt(thread);
  }

  /** 归档当前会话：从当前列表隐藏，但保留数据，可在归档中恢复。 */
  async function handleArchive() {
    if (!thread) {
      return;
    }
    await archive(thread.id, true);
    toast({ title: "会话已归档", tone: "ok" });
  }

  /**
   * 彻底删除当前会话。
   * 删除不可恢复；确认框与提示由共享流程处理，与侧栏右键菜单保持一致。
   */
  function handleDelete() {
    if (!thread) {
      return;
    }
    void deleteSessionViaConfirm(thread);
  }

  return (
    <div className="chat-head">
      <div className="chat-head-main">
        {/* 手动重命名可能给出很长的标题：超长时省略，悬停显示全文 */}
        <span className="chat-title" title={thread.title}>
          {thread.title}
        </span>
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
          {/* 图标跟随状态：面板展开时显示「收起」方向，收起时显示「展开」方向 */}
          <Icon name={panelOpen ? "panel-close" : "panel-open"} className="ico" />
        </button>
        <button
          type="button"
          className="icon-btn sm"
          title="重命名会话"
          aria-label="重命名会话"
          onClick={() => void handleRename()}
        >
          <Icon name="edit" className="ico" />
        </button>
        <button
          type="button"
          className="icon-btn sm"
          title="复制会话为新会话"
          aria-label="复制会话"
          onClick={() => void duplicate(thread.id)}
        >
          <Icon name="copy" className="ico" />
        </button>
        <button
          type="button"
          className="icon-btn sm"
          title="归档会话"
          aria-label="归档会话"
          onClick={() => void handleArchive()}
        >
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
            <div className="pi-title">归档会话</div>
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
            <div className="pi-title">删除会话</div>
            <div className="pi-desc">彻底删除消息与事件，无法恢复</div>
          </span>
        </button>
        <div className="popover-sep" />
        <div className="popover-note">删除前请确认没有需要保留的审计记录。</div>
      </Popover>
    </div>
  );
}
