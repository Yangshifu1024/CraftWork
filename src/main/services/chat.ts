// 对话服务：发送 / 引导 / 重新生成 / 取消 / 审批决定。
// 用户消息先落事件表再启动运行（崩溃不丢原文）；审批只校验参数，状态机在 Approvals。
import { EventType } from "../../shared/events";
import { newId, IdPrefix } from "../../shared/ids";
import type { ApprovalScope } from "../../shared/types";
import type { Recorder } from "../core/record";
import type { DB } from "../store/db";
import { getThread } from "../store/threads";
import { snapshot as buildSnapshot } from "../store/events";
import type { Runner, SteerItem } from "../agent/runner";
import type { Approvals } from "../agent/approvals";
import type { Catalog } from "../model/catalog";

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
