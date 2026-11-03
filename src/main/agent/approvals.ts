// 审批状态机：请求 → 卡片 → 用户决定 → 执行/跳过。
// 不允许静默放行：任何需要审批的操作都必须显式等待用户决定；
// 决定一经写入不可更改（数据库层并发保护）；拒绝时一律 once，防止误授永久权限。
import type { ApprovalFact, ApprovalPart } from "../../shared/events";
import type { Recorder } from "../core/record";
import type { DB } from "../store/db";
import { createApproval, getApproval, resolveApproval } from "../store/runs";
import type { Grants } from "./grants";
import { EventType } from "../../shared/events";

export type ApprovalScope = "once" | "session" | "workspace";

export interface ApprovalRequest {
  partId: string;
  threadId: string;
  runId: string;
  messageId: string;
  toolCallId: string;
  tool: string;
  risk: string;
  title: string;
  target: string;
  detail: string;
  facts: ApprovalFact[];
  args: Record<string, unknown>;
  allowPermanent: boolean;
  /** shell_exec 授权前缀（ grants 键用，与展示 target 可不同） */
  grantTarget?: string;
}

export interface ApprovalOutcome {
  allow: boolean;
  status: "approved" | "denied" | "canceled";
  decision: string;
  scope: ApprovalScope;
}

interface PendingEntry {
  resolve: (outcome: ApprovalOutcome) => void;
  permanent: boolean;
}

export class Approvals {
  private pending = new Map<string, PendingEntry>();
  private grantKeys = new Map<string, { tool: string; target: string }>();

  constructor(
    private readonly db: DB,
    private readonly recorder: Recorder,
    private readonly grants: Grants,
  ) {}

  /** 已有授权是否命中（会话/工作区授权缓存）。 */
  allowed(tool: string, target: string): boolean {
    return this.grants.allowed(tool, target);
  }

  /**
   * 发起审批并等待决定：登记等待表 → 写审批行 → 写 approval 事件（卡片载荷）→
   * awaiting 状态 → 等待。取消（signal）按 canceled 收尾并广播 resolved。
   */
  async request(req: ApprovalRequest, signal: AbortSignal): Promise<ApprovalOutcome> {
    const approvalId = createApproval(this.db, {
      threadId: req.threadId,
      runId: req.runId,
      toolCallId: req.toolCallId,
      tool: req.tool,
      risk: req.risk,
    }).id;

    const promise = new Promise<ApprovalOutcome>((resolve) => {
      this.pending.set(approvalId, { resolve, permanent: req.allowPermanent });
    });
    this.grantKeys.set(approvalId, { tool: req.tool, target: req.grantTarget ?? req.target });

    const card: ApprovalPart = {
      partId: req.partId,
      approvalId,
      toolCallId: req.toolCallId,
      tool: req.tool,
      risk: req.risk,
      title: req.title,
      target: req.target,
      detail: req.detail,
      facts: req.facts,
      args: req.args,
      allowPermanent: req.allowPermanent,
      status: "waiting",
    };
    try {
      this.recorder.append({
        threadId: req.threadId,
        messageId: req.messageId,
        runId: req.runId,
        kind: "approval",
        payload: card,
      });
    } catch {
      this.pending.delete(approvalId);
      this.grantKeys.delete(approvalId);
      throw new Error("审批请求写入失败");
    }
    this.recorder.setThreadStatus(req.threadId, req.runId, "awaiting");

    const outcome = await new Promise<ApprovalOutcome>((resolve) => {
      const onAbort = (): void => {
        resolve({ allow: false, status: "canceled", decision: "canceled", scope: "once" });
      };
      if (signal.aborted) {
        onAbort();
        return;
      }
      signal.addEventListener("abort", onAbort, { once: true });
      promise.then((result) => {
        signal.removeEventListener("abort", onAbort);
        resolve(result);
      });
    });

    this.pending.delete(approvalId);
    this.grantKeys.delete(approvalId);

    if (outcome.status === "canceled") {
      try {
        resolveApproval(this.db, approvalId, "canceled", "");
      } catch {
        // 已被处理
      }
      this.recorder.setThreadStatus(req.threadId, req.runId, "running");
      this.recorder.emit(EventType.ApprovalResolved, req.threadId, req.runId, {
        partId: req.partId,
        approvalId,
        status: "canceled",
        decision: "canceled",
        scope: "",
      });
      return outcome;
    }

    this.recorder.setThreadStatus(req.threadId, req.runId, "running");
    this.recorder.emit(EventType.ApprovalResolved, req.threadId, req.runId, {
      partId: req.partId,
      approvalId,
      status: outcome.status,
      decision: outcome.decision,
      scope: outcome.scope,
    });
    return outcome;
  }

  /**
   * 用户决定：状态校验 → scope 归一（拒绝一律 once）→ 并发保护落库 →
   * 授权缓存 → 唤醒等待方（resolved 事件已在 request 侧等待唤醒后广播）。
   */
  async decide(approvalId: string, allow: boolean, scope: string): Promise<void> {
    const approval = getApproval(this.db, approvalId);
    if (approval.decision !== null && approval.decision !== "") {
      throw new Error(`审批已处理：${approval.decision}`);
    }
    let normalized: ApprovalScope;
    if (!allow || (scope !== "session" && scope !== "workspace")) {
      normalized = "once";
    } else {
      normalized = scope;
    }
    const entry = this.pending.get(approvalId);
    if (!entry) throw new Error("审批等待已失效（可能已取消）");
    if (allow && normalized !== "once" && !entry.permanent) {
      throw new Error("该审批不提供永久授权，只能允许一次");
    }

    // 先落库（即使广播失败状态也不回退）
    resolveApproval(this.db, approvalId, allow ? "allow" : "deny", normalized);
    this.pending.delete(approvalId);
    if (allow && normalized !== "once") {
      const key = this.grantKeys.get(approvalId);
      if (key) this.grants.grant(key.tool, key.target, normalized);
    }
    entry.resolve({
      allow,
      status: allow ? "approved" : "denied",
      decision: allow ? "allow" : "deny",
      scope: normalized,
    });
  }
}
