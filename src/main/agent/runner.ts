// Agent 主循环：一次运行的完整状态机——
// 启动/取消、建立模型流（带整体超时与失败转移）、消费流片段、工具执行、审批门控、
// 历史重建、上下文压缩、自动命名、用量与事件落库。
// 同一会话同时只允许一次运行：新消息总是接管（先取消旧运行再覆盖登记）。
import { EventType } from "../../shared/events";
import type { RunFinishedData, RunStartedData, TextDeltaData, ToolCallPart } from "../../shared/events";
import { newId, IdPrefix } from "../../shared/ids";
import type { Recorder } from "../core/record";
import type { DB } from "../store/db";
import { getThread, addThreadTokens } from "../store/threads";
import { createRun, finishRun } from "../store/runs";
import { snapshot as buildSnapshot } from "../store/events";
import { getWorkspace, type WorkspaceWithFolders } from "../store/workspaces";
import { addAudit, addUsage } from "../store/records";
import { Engine, reasonFor } from "../policy/engine";
import type { Registry, ToolResult } from "../tools/registry";
import type { ModelFactory } from "../model/factory";
import type { Chunk, Client, Message, ModelInfo, ToolCall, ToolSpec, Usage } from "../model/types";
import { ModelError, codeOf, retryable, isAbort } from "../model/errors";
import { buildHistory } from "./history";
import { collectEnvContext } from "./envcontext";
import { buildPrompt } from "./prompt";
import { runCompression } from "./context";
import { autoTitle } from "./title";
import { loadAttachments } from "./attachments";
import type { Approvals } from "./approvals";

/** 单次模型请求（含流式接收全程）的整体超时。 */
const MODEL_REQUEST_TIMEOUT_MS = 3 * 60 * 1000;
const MAX_STEER_QUEUE = 10;

export interface RunEnv {
  workspacePath: string;
  roots: string[];
  restricted: boolean;
  purpose: string;
}

export interface SteerItem {
  messageId: string;
  text: string;
  attachments: string[];
}

export interface RunnerDeps {
  db: DB;
  records: Recorder;
  registry: Registry;
  engine: Engine;
  approvals: Approvals;
  models: ModelFactory;
  attachmentDir: string;
  dataDir: string;
}

interface Session {
  client: Client;
  modelId: string; // 提供商侧模型 ID
  modelRef: string;
  info: ModelInfo;
}

/** 费用估算（微美元）：token × 美元/百万 token；Anthropic 的输入计数不含缓存 token。 */
export function estimateCostMicros(usage: Usage, info: ModelInfo, providerKind: string): number {
  const cost = info.cost;
  if (!cost) return 0;
  const cacheWrite = Math.min(usage.cacheWriteTokens, usage.cacheTokens);
  const cacheRead = usage.cacheTokens - cacheWrite;
  let uncached = usage.inputTokens - usage.cacheTokens;
  if (providerKind === "anthropic") uncached = usage.inputTokens;
  const inputPrice = cost.inputPerMTok;
  const readPrice = cost.cacheReadPerMTok || inputPrice;
  const writePrice = cost.cacheWritePerMTok || inputPrice;
  const total =
    uncached * inputPrice + cacheRead * readPrice + cacheWrite * writePrice + usage.outputTokens * cost.outputPerMTok;
  return Math.max(0, Math.round(total));
}

/** 带整体超时的流消费：单次请求超过上限按超时错误终止。 */
async function* withStreamTimeout(
  stream: AsyncIterable<Chunk>,
  timeoutMs: number,
): AsyncGenerator<Chunk, void, unknown> {
  const iterator = stream[Symbol.asyncIterator]();
  for (;;) {
    let timer: NodeJS.Timeout | null = null;
    const timeoutPromise = new Promise<"__timeout__">((resolve) => {
      timer = setTimeout(() => resolve("__timeout__"), timeoutMs);
    });
    try {
      const result = (await Promise.race([iterator.next(), timeoutPromise])) as
        | IteratorResult<Chunk>
        | "__timeout__";
      if (result === "__timeout__") {
        throw new ModelError("timeout", "", "模型响应超时（超过 3 分钟未完成）");
      }
      if (result.done) return;
      yield result.value;
    } finally {
      if (timer) clearTimeout(timer);
    }
  }
}

export class Runner {
  private steers = new Map<string, SteerItem[]>();
  private cancels = new Map<string, AbortController>();
  private notifier: ((threadId: string, title: string, message: string) => void) | null = null;
  private maxIterationsResolver: (() => number) | null = null;
  private titlePending = new Set<string>();

  constructor(private readonly deps: RunnerDeps) {}

  setNotifier(fn: (threadId: string, title: string, message: string) => void): void {
    this.notifier = fn;
  }

  setMaxIterationsResolver(fn: () => number): void {
    this.maxIterationsResolver = fn;
  }

  isRunning(threadId: string): boolean {
    return this.cancels.has(threadId);
  }

  /** 启动一次运行：写 run 行 + 异步执行。 */
  run(threadId: string): { runId: string; error?: Error } {
    let thread;
    try {
      thread = getThread(this.deps.db, threadId);
    } catch (err) {
      return { runId: "", error: err instanceof Error ? err : new Error(String(err)) };
    }
    const run = createRun(this.deps.db, threadId, thread.modelId);
    this.deps.records.setThreadStatus(threadId, run.id, "running");
    // 新消息接管：取消旧运行
    this.cancels.get(threadId)?.abort();
    const controller = new AbortController();
    this.cancels.set(threadId, controller);
    void this.execute(controller, threadId, run.id, run.messageId, thread.mode, thread.modelId).catch(() => {
      // execute 内部已处理错误路径；此处兜底避免未处理拒绝
    });
    return { runId: run.id };
  }

  cancel(threadId: string): boolean {
    const controller = this.cancels.get(threadId);
    if (!controller) return false;
    controller.abort();
    return true;
  }

  /** 运行中引导：无在途运行返回 false（调用方回退普通发送）；队列满抛错。 */
  trySteer(threadId: string, item: SteerItem): boolean {
    if (!this.cancels.has(threadId)) return false;
    const queue = this.steers.get(threadId) ?? [];
    if (queue.length >= MAX_STEER_QUEUE) {
      throw new Error(`排队引导已达上限（${MAX_STEER_QUEUE} 条），请等当前步骤完成后再发`);
    }
    queue.push(item);
    this.steers.set(threadId, queue);
    return true;
  }

  private takeSteers(threadId: string): SteerItem[] {
    const queue = this.steers.get(threadId);
    if (!queue || queue.length === 0) return [];
    this.steers.set(threadId, []);
    return queue;
  }

  private hasSteers(threadId: string): boolean {
    return (this.steers.get(threadId)?.length ?? 0) > 0;
  }

  // ---------- 主循环 ----------

  private async execute(
    controller: AbortController,
    threadId: string,
    firstRunId: string,
    firstMessageId: string,
    mode: string,
    modelId: string,
  ): Promise<void> {
    let runId = firstRunId;
    let messageId = firstMessageId;
    let titled = false;
    try {
      for (;;) {
        const ok = await this.executeRun(controller, threadId, runId, messageId, mode, modelId, titled);
        if (!ok) return;
        titled = true;
        if (!this.hasSteers(threadId)) return;
        // 接力运行：排队引导并入新 run（历史重建时自动带入）
        const next = createRun(this.deps.db, threadId, modelId);
        runId = next.id;
        messageId = next.messageId;
        this.deps.records.setThreadStatus(threadId, runId, "running");
      }
    } finally {
      // 收尾临界区：丢弃未投递的排队引导（它们已写入事件表，下次运行的历史仍可见）
      this.steers.delete(threadId);
      if (this.cancels.get(threadId) === controller) this.cancels.delete(threadId);
    }
  }

  /** 单轮运行：返回 true 表示正常完成（可接力），false 表示已写过结束事件。 */
  private async executeRun(
    controller: AbortController,
    threadId: string,
    runId: string,
    messageId: string,
    mode: string,
    modelId: string,
    titled: boolean,
  ): Promise<boolean> {
    const { db, records, registry } = this.deps;
    const startedAt = Date.now();

    const started: RunStartedData = { messageId, modelId };
    records.emit(EventType.RunStarted, threadId, runId, started);

    // 运行环境是值不是全局态：开始时一次性解析，沿调用图下发
    const thread = getThread(db, threadId);
    let env: RunEnv = { workspacePath: "", roots: [], restricted: false, purpose: "" };
    if (thread.workspaceId) {
      let ws: WorkspaceWithFolders;
      try {
        ws = getWorkspace(db, thread.workspaceId);
      } catch {
        this.fail(threadId, runId, messageId, "workspace_missing", "会话绑定的工作区不存在或已被移除，请在侧栏重新选择工作区后重试", false, startedAt);
        return false;
      }
      env = {
        workspacePath: ws.path,
        roots: ws.folders.map((f) => f.path),
        restricted: ws.trust !== "trusted",
        purpose: ws.purpose,
      };
    }

    // 历史重建
    let history: Message[];
    let snap;
    try {
      snap = buildSnapshot(db, threadId);
      const info = this.deps.models.resolve(modelId);
      const caps = info?.caps ?? { stream: true, tools: true, vision: false, reasoning: false, structured: false };
      history = buildHistory(db, snap, env.workspacePath, this.deps.attachmentDir, caps).messages;
    } catch (err) {
      this.fail(threadId, runId, messageId, "history_failed", err instanceof Error ? err.message : String(err), true, startedAt);
      return false;
    }

    const envContext = env.workspacePath ? await collectEnvContext(this.deps.dataDir, env.workspacePath) : null;
    const promptResult = buildPrompt({
      mode,
      workspace: env.workspacePath,
      additionalFolders: env.roots.slice(1),
      workspacePurpose: env.purpose,
      restricted: env.restricted,
      env: envContext,
    });

    const specs: ToolSpec[] = registry.specsForMode(mode);
    const limit = this.maxIterationsResolver ? this.maxIterationsResolver() : 300;
    let compressionChecked = false;

    for (let iteration = 0; limit <= 0 || iteration < limit; iteration++) {
      if (controller.signal.aborted) {
        this.canceled(threadId, runId, startedAt);
        return false;
      }

      // 引导投递点：每个迭代开始（按消息 ID 去重，接力历史可能已包含）
      for (const item of this.takeSteers(threadId)) {
        if (history.some((m) => m.id === item.messageId)) continue;
        const { attachments } = loadAttachments(env.workspacePath, this.deps.attachmentDir, item.attachments);
        history.push({
          role: "user",
          content: item.text,
          toolCalls: [],
          toolCallId: "",
          attachments,
          id: item.messageId,
        });
      }

      // 建立模型会话（含失败转移）；压缩只在首轮做一次
      const session = this.openSessionWithFailover(modelId, threadId, runId, messageId);
      if (!session) return false; // 已写过结束事件
      if (!compressionChecked) {
        compressionChecked = true;
        history = await runCompression(db, records, snap, history, threadId, runId, session.client, session.info.id, session.info.contextK);
      }

      // 自动命名（首轮并行，防并发重复）
      if (iteration === 0 && !titled && thread.titleAuto && !this.titlePending.has(threadId)) {
        this.titlePending.add(threadId);
        const firstUser = history.find((m) => m.role === "user");
        void autoTitle({
          db,
          client: session.client,
          modelId: session.info.id,
          threadId,
          firstText: firstUser?.content ?? "",
          firstAttachments: firstUser?.attachments ?? [],
        })
          .catch(() => undefined)
          .finally(() => this.titlePending.delete(threadId));
      }

      const callStart = Date.now();
      let text = "";
      let reasoning = "";
      let textPartId: string | null = null;
      let reasoningPartId: string | null = null;
      const toolCalls: ToolCall[] = [];
      let usage: Usage | null = null;
      let streamError: unknown = null;

      try {
        const stream = withStreamTimeout(
          session.client.stream({
            model: session.info.id,
            mode,
            system: promptResult.blocks,
            messages: history,
            tools: specs,
            maxTokens: session.info.maxTokens > 0 ? session.info.maxTokens : 0,
          }),
          MODEL_REQUEST_TIMEOUT_MS,
        );
        for await (const chunk of stream) {
          if (controller.signal.aborted) break;
          if (chunk.err) {
            streamError = chunk.err;
            break;
          }
          if (chunk.reasoning) {
            reasoningPartId ??= newId(IdPrefix.Part);
            records.emit(EventType.ReasoningDelta, threadId, runId, { partId: reasoningPartId, text: chunk.reasoning } satisfies TextDeltaData);
            reasoning += chunk.reasoning;
          }
          if (chunk.text) {
            textPartId ??= newId(IdPrefix.Part);
            records.emit(EventType.TextDelta, threadId, runId, { partId: textPartId, text: chunk.text } satisfies TextDeltaData);
            text += chunk.text;
          }
          if (chunk.toolCall) toolCalls.push(chunk.toolCall);
          if (chunk.usage) usage = chunk.usage;
        }
      } catch (err) {
        streamError = err;
      }

      if (controller.signal.aborted) {
        this.canceled(threadId, runId, startedAt);
        return false;
      }
      if (streamError !== null) {
        if (isAbort(streamError) || (streamError instanceof ModelError && streamError.code === "canceled")) {
          this.canceled(threadId, runId, startedAt);
          return false;
        }
        if (streamError instanceof ModelError && streamError.code === "timeout") {
          this.fail(threadId, runId, messageId, "model_timeout", `模型响应超时：${streamError.message}`, retryable(streamError), startedAt);
          return false;
        }
        const code = codeOf(streamError);
        const message = streamError instanceof Error ? streamError.message : String(streamError);
        this.fail(threadId, runId, messageId, code === "unknown" ? "model_stream_error" : code, message, retryable(streamError), startedAt);
        return false;
      }

      // 正文/推理部件落库（事件表存完整部件而非增量）
      if (reasoning) {
        records.appendOrWarn({
          threadId,
          runId,
          messageId,
          kind: "reasoning",
          payload: { partId: reasoningPartId ?? newId(IdPrefix.Part), text: reasoning },
        });
      }
      if (text) {
        records.appendOrWarn({
          threadId,
          runId,
          messageId,
          kind: "text",
          payload: { partId: textPartId ?? newId(IdPrefix.Part), text },
        });
      }

      if (usage) {
        this.recordUsage(threadId, runId, session.info, usage, Date.now() - callStart, false);
      }

      // 没有工具调用 = 最终回答
      if (toolCalls.length === 0) {
        this.complete(threadId, runId, startedAt);
        return true;
      }

      history.push({ role: "assistant", content: text, toolCalls, toolCallId: "", attachments: [] });
      for (const call of toolCalls) {
        const { result, aborted } = await this.runTool(controller, threadId, runId, messageId, call, env, mode);
        if (aborted) {
          this.canceled(threadId, runId, startedAt);
          return false;
        }
        history.push({ role: "tool", content: result.output, toolCalls: [], toolCallId: call.id, attachments: [] });
      }
    }

    this.fail(threadId, runId, messageId, "max_iterations", `达到模型调用上限（${limit} 轮），已停止本次运行`, false, startedAt);
    return false;
  }

  // ---------- 会话建立与失败转移 ----------

  private openSession(modelRef: string): Session | null {
    try {
      const info = this.deps.models.resolve(modelRef);
      if (!info) return null;
      const client = this.deps.models.client(modelRef);
      return { client, modelId: info.id, modelRef, info };
    } catch {
      return null;
    }
  }

  /**
   * 建流候选链：[主模型, 备用模型]。
   * 仅在「尚未进入流」的建立阶段转移（client 构造失败/模型缺失），并写可见 notice。
   * 一旦流建立成功，后续错误不再转移（避免上下文混乱）。
   */
  private openSessionWithFailover(modelRef: string, threadId: string, runId: string, messageId: string): Session | null {
    const fallback = this.deps.models.fallbackModel(modelRef);
    const candidates = fallback && fallback !== modelRef ? [modelRef, fallback] : [modelRef];
    const failures: string[] = [];
    for (const candidate of candidates) {
      const session = this.openSession(candidate);
      if (session) {
        if (failures.length > 0) {
          this.deps.records.appendOrWarn({
            threadId,
            runId,
            messageId,
            kind: "notice",
            payload: {
              partId: newId(IdPrefix.Part),
              text: `主模型暂时不可用（${failures.join("；")}），已切换到 ${candidate}。`,
            },
          });
        }
        return session;
      }
      failures.push(candidate);
    }
    const error = new ModelError("provider_unavailable", modelRef, `无法建立模型连接（${failures.join("；")}），请检查模型配置`);
    this.fail(threadId, runId, messageId, error.code, error.message, false, Date.now());
    return null;
  }

  // ---------- 工具执行 ----------

  /** 单工具调用全流程：评估 → tool.started → 拦截判定/审批 → 执行 → 落库 → 审计。 */
  private async runTool(
    controller: AbortController,
    threadId: string,
    runId: string,
    messageId: string,
    call: ToolCall,
    env: RunEnv,
    mode: string,
  ): Promise<{ result: ToolResult; aborted: boolean }> {
    const { db, records, registry, engine, approvals } = this.deps;
    const partId = newId(IdPrefix.Part);
    const assessment = registry.assess(env.roots.length > 0 ? env.roots : ["."], call.name, call.args ?? {});
    const spec = registry.specFor(call.name);
    const summary = spec?.summaryArg ? String((call.args ?? {})[spec.summaryArg] ?? "") : (spec?.summaryText ?? call.name);
    const title = spec?.title || call.name;

    records.emit(EventType.ToolStarted, threadId, runId, {
      partId,
      toolCallId: call.id,
      name: call.name,
      args: call.args ?? {},
      risk: assessment.risk,
      status: "running",
      summary,
      title,
    } satisfies ToolCallPart);

    let denied = "";
    let scope = "auto";

    if (!assessment.exists) denied = `未知工具：${call.name}`;
    else if (assessment.prepared?.err) denied = assessment.prepared.err.message;
    else if (!registry.allows(mode, call.name)) denied = "当前模式不支持该工具";
    else {
      const action = engine.action(env.restricted, assessment.risk);
      if (action === "deny") denied = "当前审批档位拒绝该操作";
      else if (action === "ask") {
        const target = assessment.target;
        const grantTarget = call.name === "shell_exec" ? `prefix:${this.prefixFor(String((call.args ?? {})["command"] ?? ""))}` || target : target;
        const cached = !env.restricted && approvals.allowed(call.name, grantTarget);
        if (cached) {
          scope = "cache";
        } else {
          // 受限工作区每次必问且无授权缓存
          const permanent = engine.allowsPermanent(env.restricted, assessment.risk);
          const facts = [
            { label: "风险等级", value: assessment.risk },
            { label: "操作", value: summary },
            { label: "原因", value: reasonFor(assessment.risk) },
            ...(target ? [{ label: "目标", value: target }] : []),
          ];
          if (call.name === "shell_exec") {
            const prefix = this.prefixFor(String((call.args ?? {})["command"] ?? ""));
            if (prefix) facts.push({ label: "授权粒度", value: `允许「${prefix}」开头的命令` });
          }
          this.notifier?.(threadId, "需要审批", `${title} · ${summary}`.slice(0, 120));
          const outcome = await approvals.request(
            {
              partId,
              threadId,
              runId,
              messageId,
              toolCallId: call.id,
              tool: call.name,
              risk: assessment.risk,
              title: title || "需要你的批准",
              target,
              detail: assessment.reason || title,
              facts,
              args: call.args ?? {},
              allowPermanent: permanent,
              grantTarget,
            },
            controller.signal,
          );
          if (outcome.status === "canceled") {
            return { result: { output: "审批已取消", isError: true, code: "tool_error" }, aborted: true };
          }
          if (!outcome.allow) denied = "用户拒绝了该操作";
          else scope = outcome.scope;
        }
      }
    }

    // 执行
    let result: ToolResult;
    if (denied) {
      result = { output: denied, isError: true, code: "tool_error" };
    } else {
      const execStart = Date.now();
      result = await registry.execute(assessment.prepared!);
      result = { ...result, meta: { ...(result.meta ?? {}), durationMs: Date.now() - execStart } };
      if (controller.signal.aborted) {
        return { result, aborted: true };
      }
    }

    const durationMs = Number(result.meta?.["durationMs"] ?? 0);
    records.appendOrWarn({
      threadId,
      runId,
      messageId,
      kind: "tool_call",
      payload: {
        partId,
        toolCallId: call.id,
        name: call.name,
        args: call.args ?? {},
        risk: assessment.risk,
        status: result.isError ? "error" : "done",
        output: result.output,
        isError: result.isError,
        code: result.code,
        durationMs,
        summary,
        title,
      } satisfies ToolCallPart,
    });

    // 审计：不区分「拦截」与「执行失败」
    addAudit(db, {
      threadId,
      runId,
      tool: call.name,
      risk: assessment.risk,
      decision: denied ? "deny" : scope !== "auto" && scope !== "cache" ? "allow" : "auto",
      scope: scope === "auto" || scope === "cache" ? "" : scope,
      detail: summary,
    });

    // 任务清单部件（前端渲染待办卡片）
    if (call.name === "todo_write" && !result.isError) {
      records.appendOrWarn({
        threadId,
        runId,
        messageId,
        kind: "todo",
        payload: { partId: newId(IdPrefix.Part), items: (call.args ?? {})["items"] ?? [] },
      });
    }

    return { result, aborted: false };
  }

  /** 命令授权前缀（与授权缓存同一规则）。 */
  private prefixFor(command: string): string {
    const trimmed = command.trim();
    if (!trimmed) return "";
    const reject = [";", "|", "&", ">", "<", "`", "\n", "\r", "$(", "${"];
    if (reject.some((c) => trimmed.includes(c))) return "";
    const parts = trimmed.split(/\s+/);
    const interpreters = new Set(["env", "sudo", "bash", "sh", "zsh", "fish", "powershell", "cmd", "eval", "exec"]);
    const first = parts[0] ?? "";
    if (parts.length === 1 || interpreters.has(first.toLowerCase())) return "";
    const second = parts[1] ?? "";
    return second.startsWith("-") ? first : `${first} ${second}`;
  }

  // ---------- 结束路径 ----------

  private complete(threadId: string, runId: string, startedAt: number): void {
    const durationMs = Date.now() - startedAt;
    finishRun(this.deps.db, runId, "done", "", "");
    this.deps.records.setThreadStatus(threadId, runId, "idle");
    this.deps.records.emit(EventType.RunCompleted, threadId, runId, { status: "done", durationMs } satisfies RunFinishedData);
    this.notify(threadId, "CraftWork", "本轮回复已完成。");
  }

  private canceled(threadId: string, runId: string, startedAt: number): void {
    const durationMs = Date.now() - startedAt;
    finishRun(this.deps.db, runId, "canceled", "canceled", "用户取消");
    this.deps.records.setThreadStatus(threadId, runId, "idle");
    this.deps.records.emit(EventType.RunCanceled, threadId, runId, { status: "canceled", durationMs } satisfies RunFinishedData);
    // 取消不发通知
  }

  private fail(
    threadId: string,
    runId: string,
    messageId: string,
    code: string,
    message: string,
    retryableFlag: boolean,
    startedAt: number,
  ): void {
    // 错误部件先落库再切状态（等待方以「会话进入 error」为完成信号读快照）
    this.deps.records.appendOrWarn({
      threadId,
      runId,
      messageId,
      kind: "error",
      payload: { partId: newId(IdPrefix.Part), code, message, retryable: retryableFlag },
    });
    const durationMs = Date.now() - startedAt;
    finishRun(this.deps.db, runId, "error", code, message);
    this.deps.records.setThreadStatus(threadId, runId, "error");
    this.deps.records.emit(EventType.RunFailed, threadId, runId, {
      status: "error",
      code,
      message,
      durationMs,
    } satisfies RunFinishedData);
    this.notify(threadId, "CraftWork 运行失败", [...message].slice(0, 120).join(""));
  }

  private notify(threadId: string, title: string, message: string): void {
    this.notifier?.(threadId, title, message);
  }

  // ---------- 用量 ----------

  private recordUsage(
    threadId: string,
    runId: string,
    info: ModelInfo,
    usage: Usage,
    durationMs: number,
    sidecar: boolean,
  ): void {
    try {
      const kind = this.deps.models.providerKind(info.provider);
      const costMicros = estimateCostMicros(usage, info, kind);
      addUsage(this.deps.db, {
        threadId,
        modelId: `${info.provider}/${info.id}`,
        inputTokens: usage.inputTokens,
        outputTokens: usage.outputTokens,
        cacheTokens: usage.cacheTokens,
        reasoningTokens: usage.reasoningTokens,
        costMicros,
        durationMs,
      });
      addThreadTokens(this.deps.db, threadId, usage.inputTokens + usage.outputTokens);
      this.deps.records.emit(EventType.UsageUpdated, threadId, runId, {
        inputTokens: usage.inputTokens,
        outputTokens: usage.outputTokens,
        costMicros,
        // sidecar 调用（自动命名）不携带上下文水位字段，避免小请求覆盖「当前上下文占用」
        ...(sidecar
          ? {}
          : {
              contextTokens: usage.inputTokens + usage.outputTokens,
              cacheTokens: usage.cacheTokens,
              durationMs,
              tokensPerSecond: durationMs > 0 ? usage.outputTokens / (durationMs / 1000) : 0,
            }),
      });
    } catch {
      // 用量记账失败不阻断运行
    }
  }
}
