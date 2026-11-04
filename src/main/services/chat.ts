// 对话服务：发送 / 创建并首发 / 引导 / 重新生成 / 取消 / 审批决定。
// 用户消息先落事件表再启动运行（崩溃不丢原文）；审批只校验参数，状态机在 Approvals。
import { EventType } from "../../shared/events";
import { newId, IdPrefix } from "../../shared/ids";
import type { ApprovalScope } from "../../shared/types";
import type { Recorder } from "../core/record";
import type { DB } from "../store/db";
import { createThread, getThread, type ThreadRow } from "../store/threads";
import { snapshot as buildSnapshot } from "../store/events";
import { defaultConversationWorkspaceId, getWorkspace } from "../store/workspaces";
import { toThreadView } from "../store/view";
import { shortTitle } from "../agent/title";
import type { Runner, SteerItem } from "../agent/runner";
import type { Approvals } from "../agent/approvals";
import type { Catalog } from "../model/catalog";
import { normalizeMode } from "./threads";

export class ChatService {
  constructor(
    private readonly db: DB,
    private readonly recorder: Recorder,
    private readonly runner: Runner,
    private readonly approvals: Approvals,
    private readonly catalog: Catalog,
  ) {}

  /** 发送消息：写用户消息事件 → 启动运行；返回 runId。 */
  send(threadId: string, text: string, attachments: string[]): string {
    const thread = getThread(this.db, threadId);
    if (this.runner.isRunning(threadId)) throw new Error("会话正在运行，请先停止");
    this.ensureThreadModel(thread);
    const clean = text.trim();
    this.writeUserMessage(threadId, clean, attachments);
    const { runId, error } = this.runner.run(threadId);
    if (error) throw error;
    return runId;
  }

  /** 创建并首发：模型校验 → 创建 Thread（短标题命名）→ 写用户消息事件 → 启动 Run。 */
  createAndStart(
    mode: string,
    workspaceId: string,
    text: string,
    attachments: string[],
    modelRef: string,
  ): { threadId: string; runId: string } {
    const normalizedMode = normalizeMode(mode);
    const clean = text.trim();
    if (!clean && attachments.length === 0) throw new Error("消息不能为空");
    this.ensureThreadModel({ modelId: modelRef });
    if (workspaceId) {
      try {
        getWorkspace(this.db, workspaceId);
      } catch {
        throw new Error("工作区不存在，请重新选择工作区后再新建会话");
      }
    }
    // 短标题取到即正式命名（titleAuto=false，LLM 命名自然跳过）；取不到回落「新对话」走自动命名
    const short = shortTitle(clean);
    const row: Omit<ThreadRow, "createdAt" | "updatedAt" | "tokens"> = {
      id: newId(IdPrefix.Thread),
      mode: normalizedMode,
      title: short || "新对话",
      titleAuto: short === "",
      modelId: modelRef,
      workspaceId: workspaceId || defaultConversationWorkspaceId(this.db),
      status: "idle",
      archived: false,
    };
    const created = createThread(this.db, row);
    const messageId = this.writeUserMessage(created.id, clean, attachments);
    // 首发后新会话时间线立即显示用户消息（user_message 落库无广播映射，手动 emit 一次）
    this.recorder.emit(EventType.UserMessage, created.id, undefined, { messageId, text: clean, attachments });
    this.publishThreadChanged(created.id);
    const { runId, error } = this.runner.run(created.id);
    if (error) {
      // 创建成功即视为发送成功：线程与用户消息已落库，不回滚；错误经 run.failed 事件展示
      return { threadId: created.id, runId: "" };
    }
    return { threadId: created.id, runId };
  }

  /** 运行中引导：入队（会并入下一次请求）；无在途运行回退普通发送。 */
  steer(threadId: string, text: string, attachments: string[]): string {
    const thread = getThread(this.db, threadId);
    this.ensureThreadModel(thread);
    const clean = text.trim();
    const messageId = this.writeUserMessage(threadId, clean, attachments);
    this.recorder.emit(EventType.UserMessage, threadId, undefined, {
      messageId,
      text: clean,
      attachments,
      via: "steer",
    });
    const item: SteerItem = { messageId, text: clean, attachments };
    const queued = this.runner.trySteer(threadId, item);
    if (queued) return messageId;
    // 无在途运行（含接力刚耗尽的瞬间）：回退普通发送
    const { error } = this.runner.run(threadId);
    if (error) throw error;
    return messageId;
  }

  /** 重新生成：不重复写用户消息，直接重跑（历史中须有用户消息）。 */
  regenerate(threadId: string): string {
    const thread = getThread(this.db, threadId);
    if (this.runner.isRunning(threadId)) throw new Error("会话正在运行，请先停止");
    this.ensureThreadModel(thread);
    const snap = buildSnapshot(this.db, threadId);
    if (!snap.messages.some((m) => m.role === "user")) {
      throw new Error("没有可重新生成的内容");
    }
    const { runId, error } = this.runner.run(threadId);
    if (error) throw error;
    return runId;
  }

  /** 取消：中断运行 + 撤销挂起审批；返回此前是否有在途运行。 */
  cancel(threadId: string): boolean {
    const had = this.runner.cancel(threadId);
    if (had) {
      this.recorder.setThreadStatus(threadId, undefined, "idle");
    }
    return had;
  }

  /** 审批决定：转发给审批状态机。 */
  async decideApproval(approvalId: string, allow: boolean, scope: ApprovalScope): Promise<void> {
    if (!approvalId) throw new Error("审批 ID 不能为空");
    await this.approvals.decide(approvalId, allow, scope);
  }

  private ensureThreadModel(thread: { modelId: string }): void {
    if (!thread.modelId) {
      throw new Error("尚未选择模型，请先在设置中添加提供商并选择模型");
    }
    if (!this.catalog.resolve(thread.modelId)) {
      throw new Error("当前模型不可用，请重新选择模型");
    }
  }

  /** 重读并广播 thread.updated，让侧栏立即出现新会话（已删除则静默）。 */
  private publishThreadChanged(threadId: string): void {
    try {
      const row = getThread(this.db, threadId);
      const view = toThreadView(this.db, row);
      this.recorder.emit(EventType.ThreadUpdated, threadId, undefined, { threadId, thread: view });
    } catch {
      // 已删除：不广播
    }
  }

  /** 写用户消息事件（纯附件消息合法）；返回消息 ID。 */
  private writeUserMessage(threadId: string, text: string, attachments: string[]): string {
    if (!text && attachments.length === 0) throw new Error("消息不能为空");
    const messageId = newId(IdPrefix.Message);
    this.recorder.append({
      threadId,
      messageId,
      kind: "user_message",
      payload: { text, attachments },
    });
    return messageId;
  }
}
