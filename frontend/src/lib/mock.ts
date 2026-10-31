// 浏览器 Mock 后端。
//
// 用途：不启动 Wails3 时用 `npm run dev` 直接预览界面，便于视觉走查与
// 后续 Playwright 回归。它不读写磁盘，所有数据都在内存中。
import type { Api } from "./api-types";
import { dispatch, envelope } from "./bus";
import type {
  AuditEntry,
  Backup,
  ChatMessage,
  ChatPart,
  DiffResult,
  DocumentPreview,
  FileNode,
  Mode,
  ProviderView,
  Settings,
  TerminalView,
  ThreadSnapshot,
  ThreadView,
  UsageSummary,
  WorkspaceView,
} from "./types";
import { EVENT } from "./types";
import { basename, chunkText, sleep, uid } from "./utils";

const now = Date.now();

const settings: Settings = {
  theme: "dark",
  mode: "code",
  panelWidth: 440,
  approvalLevel: "graded",
  defaultModel: "local-demo",
  fallbackModel: "",
  activeWorkspace: "ws_demo",
  language: "zh-CN",
  fontSize: 13,
  codeTheme: "dark",
  terminalFontSize: 12,
  autoUpdate: true,
  notifications: true,
  updateChannel: "stable",
  telemetry: false,
};

const workspaces: WorkspaceView[] = [
  {
    id: "ws_demo",
    name: "craftwork",
    path: "~/Code/craftwork",
    trust: "trusted",
    createdAt: now - 86400000 * 12,
    lastOpenedAt: now - 60000,
  },
];

const threads: Record<string, ThreadView[]> = {
  code: [
    {
      id: "th_code_1",
      mode: "code",
      title: "修复登录刷新超时",
      modelId: "local-demo",
      modelName: "Local Demo",
      workspaceId: "ws_demo",
      status: "idle",
      archived: false,
      tokens: 18420,
      createdAt: now - 3600000,
      updatedAt: now - 120000,
    },
    {
      id: "th_code_2",
      mode: "code",
      title: "为工作区搜索补上中文分词",
      modelId: "local-demo",
      modelName: "Local Demo",
      workspaceId: "ws_demo",
      status: "idle",
      archived: false,
      tokens: 7310,
      createdAt: now - 7200000,
      updatedAt: now - 3600000,
    },
  ],
  work: [
    {
      id: "th_work_1",
      mode: "work",
      title: "整理季度会议纪要",
      modelId: "local-demo",
      modelName: "Local Demo",
      workspaceId: "ws_demo",
      status: "idle",
      archived: false,
      tokens: 9640,
      createdAt: now - 86400000,
      updatedAt: now - 1800000,
    },
    {
      id: "th_work_2",
      mode: "work",
      title: "修正预算表合计公式",
      modelId: "local-demo",
      modelName: "Local Demo",
      workspaceId: "ws_demo",
      status: "idle",
      archived: false,
      tokens: 4120,
      createdAt: now - 86400000 * 2,
      updatedAt: now - 7200000,
    },
  ],
};

const files: Record<string, string> = {
  "README.md": "# craftwork\n\n一个用于演示 CraftWork 的 Go 仓库。\n\n## 常用命令\n\n```bash\ngo test ./...\ngo build ./...\n```\n",
  "go.mod": "module craftwork\n\ngo 1.26.0\n",
  "internal/auth/session.go": `package auth

import "time"

const refreshWindow = 10 * time.Minute

type Session struct {
\tToken       string
\tRefreshedAt time.Time
}

func (s *Session) NeedsRefresh(now time.Time) bool {
\treturn now.Sub(s.RefreshedAt) > refreshWindow
}

func (s *Session) Refresh(now time.Time) error {
\tif !s.NeedsRefresh(now) {
\t\treturn nil
\t}
\ts.RefreshedAt = now
\treturn nil
}
`,
  "internal/app/app.go": "package app\n\nfunc Run() error {\n\treturn nil\n}\n",
  "frontend/src/App.tsx": "export default function App() {\n  return <main>CraftWork</main>;\n}\n",
};

const tree: Record<string, FileNode[]> = {
  ".": [
    { name: "frontend", path: "frontend", type: "dir", size: 0, modified: now },
    { name: "internal", path: "internal", type: "dir", size: 0, modified: now },
    { name: "季度会议纪要.docx", path: "季度会议纪要.docx", type: "file", size: 48210, modified: now },
    { name: "预算表.xlsx", path: "预算表.xlsx", type: "file", size: 22400, modified: now },
    { name: "README.md", path: "README.md", type: "file", size: 148, modified: now },
    { name: "go.mod", path: "go.mod", type: "file", size: 34, modified: now },
  ],
  internal: [
    { name: "app", path: "internal/app", type: "dir", size: 0, modified: now },
    { name: "auth", path: "internal/auth", type: "dir", size: 0, modified: now },
  ],
  "internal/app": [
    { name: "app.go", path: "internal/app/app.go", type: "file", size: 48, modified: now },
  ],
  "internal/auth": [
    { name: "session.go", path: "internal/auth/session.go", type: "file", size: 420, modified: now },
  ],
  frontend: [
    { name: "src", path: "frontend/src", type: "dir", size: 0, modified: now },
  ],
  "frontend/src": [
    { name: "App.tsx", path: "frontend/src/App.tsx", type: "file", size: 72, modified: now },
  ],
};

const gitStatuses: Record<string, string> = {
  "internal/auth/session.go": "M",
};

function assistantMessage(id: string, threadId: string, parts: ChatPart[]): ChatMessage {
  return {
    id,
    threadId,
    role: "assistant",
    status: "complete",
    createdAt: now - 110000,
    parts,
  };
}

const messages: Record<string, ChatMessage[]> = {
  th_code_1: [
    {
      id: "m1_user",
      threadId: "th_code_1",
      role: "user",
      status: "complete",
      createdAt: now - 180000,
      parts: [{ id: "p1", kind: "text", seq: 1, data: { text: "登录后 15 分钟刷新令牌就超时了，帮我看看 `internal/auth/session.go` 里的刷新逻辑。" } }],
    },
    assistantMessage("m1_assistant", "th_code_1", [
      { id: "p2", kind: "reasoning", seq: 2, data: { text: "先确认 session.go 的刷新判断，再看测试覆盖。" } },
      { id: "p3", kind: "text", seq: 3, data: { text: "我先读取 `internal/auth/session.go`，确认刷新窗口的计算方式。\n\n关键代码在 `internal/auth/session.go:12`：\n\n```go\nconst refreshWindow = 10 * time.Minute\n\nfunc (s *Session) NeedsRefresh(now time.Time) bool {\n\treturn now.Sub(s.RefreshedAt) > refreshWindow\n}\n```\n\n`refreshWindow` 固定为 10 分钟，而令牌实际有效期是 15 分钟，所以第 11 分钟就会误判为过期。" } },
      { id: "p4", kind: "tool", seq: 4, data: { partId: "p4", toolCallId: "call_read", name: "fs_read", args: { path: "internal/auth/session.go" }, risk: "R0", status: "done", output: "internal/auth/session.go 共 22 行，已读取。", isError: false, durationMs: 32, summary: "internal/auth/session.go" } },
      { id: "p5", kind: "text", seq: 5, data: { text: "建议把刷新窗口改为令牌有效期的 80%（12 分钟），并在刷新失败时回退到重新登录。改动集中在 `internal/auth/session.go:6` 与对应单测。" } },
    ]),
  ],
  th_code_2: [
    {
      id: "m2_user",
      threadId: "th_code_2",
      role: "user",
      status: "complete",
      createdAt: now - 3700000,
      parts: [{ id: "p1", kind: "text", seq: 1, data: { text: "线程搜索现在只能匹配标题，怎么补上消息内容命中？" } }],
    },
    assistantMessage("m2_assistant", "th_code_2", [
      { id: "p2", kind: "text", seq: 2, data: { text: "搜索入口在 `internal/service/threads.go`，现在只查 `threads.title`。\n\n建议用 `LEFT JOIN events` 对 `payload` 做 `LIKE`，并对 `thread_id + updated_at` 建索引；中文查询不需要额外分词，子串匹配即可满足 F-TH-03。" } },
    ]),
  ],
  th_work_1: [
    {
      id: "m3_user",
      threadId: "th_work_1",
      role: "user",
      status: "complete",
      createdAt: now - 1900000,
      parts: [{ id: "p1", kind: "text", seq: 1, data: { text: "把 Q3 会议纪要里的待办整理成表格，并修正人名笔误。" } }],
    },
    assistantMessage("m3_assistant", "th_work_1", [
      { id: "p2", kind: "text", seq: 2, data: { text: "已读取文档并整理出 4 条待办：\n\n| 负责人 | 事项 | 截止 |\n| --- | --- | --- |\n| 林岚 | 补充预算说明 | 10/24 |\n| 周野 | 对齐发布节奏 | 10/26 |\n| 陈默 | 更新接口文档 | 10/28 |\n\n文档写入需要审批；批准前不会修改原文件。" } },
    ]),
  ],
  th_work_2: [
    {
      id: "m4_user",
      threadId: "th_work_2",
      role: "user",
      status: "complete",
      createdAt: now - 7300000,
      parts: [{ id: "p1", kind: "text", seq: 1, data: { text: "预算表的合计公式错了，B8 / C8 / D8 应该按季度汇总。" } }],
    },
    assistantMessage("m4_assistant", "th_work_2", [
      { id: "p2", kind: "text", seq: 2, data: { text: "检查后确认 `B8` 只汇总了 7 月。建议改为 `=SUM(B2:B7)`，C8、D8 同理。修改前会备份原文件。" } },
    ]),
  ],
};

const audit: AuditEntry[] = [
  { id: 3, ts: now - 120000, threadId: "th_code_1", runId: "run_demo", tool: "fs_read", risk: "R0", decision: "auto", scope: "auto", detail: "internal/auth/session.go" },
  { id: 2, ts: now - 180000, threadId: "th_code_1", runId: "run_demo", tool: "todo_write", risk: "R0", decision: "auto", scope: "auto", detail: "更新任务清单" },
  { id: 1, ts: now - 1900000, threadId: "th_work_1", runId: "run_work", tool: "doc_read", risk: "R0", decision: "auto", scope: "auto", detail: "季度会议纪要.docx" },
];

const usage: UsageSummary = {
  today: { inputTokens: 18420, outputTokens: 3120, cacheTokens: 2100, costMicros: 42000, runs: 6 },
  week: { inputTokens: 98240, outputTokens: 18400, cacheTokens: 12400, costMicros: 210000, runs: 31 },
  month: { inputTokens: 402100, outputTokens: 72100, cacheTokens: 50200, costMicros: 880000, runs: 124 },
  byModel: [
    { modelId: "local-demo", inputTokens: 302100, outputTokens: 52100, costMicros: 0, runs: 108 },
    { modelId: "deepseek-chat", inputTokens: 100000, outputTokens: 20000, costMicros: 880000, runs: 16 },
  ],
  byDay: Array.from({ length: 14 }, (_, index) => ({
    day: new Date(now - (13 - index) * 86400000).toISOString().slice(0, 10),
    inputTokens: 12000 + index * 900,
    outputTokens: 2400 + index * 120,
    costMicros: 12000 + index * 900,
    runs: 3 + (index % 4),
  })),
};

const backups: Backup[] = [
  { id: "bk_demo", path: "季度会议纪要.docx", backupPath: "~/.craftwork/backups/季度会议纪要.docx.20261031-142210", hash: "9f2c", size: 48120, createdAt: now - 1900000 },
  { id: "bk_demo2", path: "内部/auth/session.go", backupPath: "~/.craftwork/backups/session.go.20261031-143002", hash: "a17d", size: 420, createdAt: now - 120000 },
];

const providers: ProviderView[] = [
  {
    id: "local", name: "本地演示", kind: "mock", endpoint: "", keyRequired: false,
    keySource: "", keyMasked: "", available: true, note: "无需密钥，用于本地演示与端到端测试。",
    models: [{ id: "local-demo", name: "Local Demo", provider: "local", providerName: "本地演示", contextK: 32, caps: { stream: true, tools: true, vision: false, reasoning: true, structured: false }, default: true, available: true }],
  },
  {
    id: "deepseek", name: "DeepSeek", kind: "openai-compatible", endpoint: "https://api.deepseek.com/v1", keyRequired: true,
    keySource: "", keyMasked: "", available: false,
    models: [
      { id: "deepseek-chat", name: "DeepSeek Chat", provider: "deepseek", providerName: "DeepSeek", contextK: 64, caps: { stream: true, tools: true, vision: false, reasoning: false, structured: true }, default: false, available: false },
      { id: "deepseek-reasoner", name: "DeepSeek Reasoner", provider: "deepseek", providerName: "DeepSeek", contextK: 64, caps: { stream: true, tools: true, vision: false, reasoning: true, structured: false }, default: false, available: false },
    ],
  },
  {
    id: "openai", name: "OpenAI", kind: "openai", endpoint: "https://api.openai.com/v1", keyRequired: true,
    keySource: "", keyMasked: "", available: false,
    models: [
      { id: "gpt-4o", name: "GPT-4o", provider: "openai", providerName: "OpenAI", contextK: 128, caps: { stream: true, tools: true, vision: true, reasoning: false, structured: true }, default: false, available: false },
    ],
  },
  {
    id: "anthropic", name: "Anthropic", kind: "anthropic", endpoint: "https://api.anthropic.com", keyRequired: true,
    keySource: "", keyMasked: "", available: false,
    note: "原生 Messages 协议，支持流式输出、工具调用与图片输入。",
    models: [
      { id: "claude-sonnet-4", name: "Claude Sonnet 4", provider: "anthropic", providerName: "Anthropic", contextK: 200, caps: { stream: true, tools: true, vision: true, reasoning: false, structured: false }, default: false, available: false },
    ],
  },
  {
    id: "gemini", name: "Google Gemini", kind: "gemini", endpoint: "https://generativelanguage.googleapis.com", keyRequired: true,
    keySource: "", keyMasked: "", available: false,
    note: "原生 generateContent 协议，支持流式输出、函数调用与图片输入。",
    models: [
      { id: "gemini-2.5-pro", name: "Gemini 2.5 Pro", provider: "gemini", providerName: "Google Gemini", contextK: 1000, caps: { stream: true, tools: true, vision: true, reasoning: true, structured: false }, default: false, available: false },
    ],
  },
];

const terminalSessions: TerminalView[] = [];
let terminalCounter = 0;
const cancelled = new Set<string>();
const approvalResolvers = new Map<string, (allow: boolean) => void>();

function findThread(threadID: string): ThreadView | undefined {
  return [...threads.code, ...threads.work].find((thread) => thread.id === threadID);
}

function snapshot(threadID: string): ThreadSnapshot {
  const thread = findThread(threadID);
  if (!thread) {
    throw new Error("线程不存在");
  }
  const list = messages[threadID] ?? [];
  const approvals = list.flatMap((message) =>
    message.parts
      .filter((part) => part.kind === "approval" && part.data.status === "waiting")
      .map((part) => ({
        id: part.data.approvalId as string,
        threadId: threadID,
        runId: "",
        toolCallId: part.data.toolCallId as string,
        tool: part.data.tool as string,
        risk: part.data.risk as string,
        decision: "",
        scope: "",
      })),
  );
  return {
    thread: { ...thread, mode: thread.mode as Mode },
    messages: list.map((message) => ({
      id: message.id,
      threadId: message.threadId,
      role: message.role,
      status: message.status,
      createdAt: message.createdAt,
      parts: message.parts.map((part) => ({ id: part.id, kind: part.kind, seq: part.seq, data: part.data })),
    })),
    lastSeq: list.reduce((total, message) => total + message.parts.length, 0),
    approvals,
  };
}

function batch(threadID: string, events: ReturnType<typeof envelope>[]) {
  return { v: 1, threadId: threadID, events };
}

function addPart(threadID: string, messageID: string, part: ChatPart): void {
  const list = messages[threadID] ?? [];
  const message = list.find((item) => item.id === messageID);
  if (message) {
    message.parts = [...message.parts, part];
  }
}

async function simulateRun(thread: ThreadView, text: string): Promise<void> {
  const runID = uid("run");
  const messageID = uid("msg");
  cancelled.delete(thread.id);
  messages[thread.id] = [
    ...(messages[thread.id] ?? []),
    {
      id: messageID,
      threadId: thread.id,
      role: "assistant",
      status: "streaming",
      createdAt: Date.now(),
      parts: [],
    },
  ];
  thread.status = "running";
  dispatch(batch(thread.id, [envelope(EVENT.runStarted, thread.id, runID, { messageId: messageID, modelId: thread.modelId })]));

  const reasoningID = uid("part");
  addPart(thread.id, messageID, { id: reasoningID, kind: "reasoning", seq: 1, data: { text: "" } });
  for (const piece of chunkText("我先确认工作区的文件结构，再给出结论。", 5)) {
    if (cancelled.has(thread.id)) {
      return;
    }
    dispatch(batch(thread.id, [envelope(EVENT.reasoningDelta, thread.id, runID, { partId: reasoningID, text: piece })]));
    await sleep(22);
  }
  dispatch(batch(thread.id, [envelope(EVENT.reasoningCompleted, thread.id, runID, { partId: reasoningID, text: "我先确认工作区的文件结构，再给出结论。" })]));

  const textID = uid("part");
  addPart(thread.id, messageID, { id: textID, kind: "text", seq: 2, data: { text: "" } });
  const answer = `我先看一下当前工作区的文件。\n\n你刚才提到：**${text.slice(0, 80)}**\n\n我会先读取相关文件，再决定是否需要修改。`;
  for (const piece of chunkText(answer, 4)) {
    if (cancelled.has(thread.id)) {
      return;
    }
    dispatch(batch(thread.id, [envelope(EVENT.textDelta, thread.id, runID, { partId: textID, text: piece })]));
    await sleep(12);
  }
  dispatch(batch(thread.id, [envelope(EVENT.textCompleted, thread.id, runID, { partId: textID, text: answer })]));

  const toolID = uid("part");
  const toolCallID = uid("call");
  addPart(thread.id, messageID, {
    id: toolID, kind: "tool", seq: 3,
    data: { partId: toolID, toolCallId: toolCallID, name: "fs_list", args: { path: "." }, risk: "R0", status: "running", summary: "." },
  });
  dispatch(batch(thread.id, [envelope(EVENT.toolStarted, thread.id, runID, { partId: toolID, toolCallId: toolCallID, name: "fs_list", args: { path: "." }, risk: "R0", status: "running", summary: "." })]));
  await sleep(220);
  const output = "frontend/\ninternal/\nREADME.md\ngo.mod";
  dispatch(batch(thread.id, [envelope(EVENT.toolCompleted, thread.id, runID, { partId: toolID, toolCallId: toolCallID, name: "fs_list", args: { path: "." }, risk: "R0", status: "done", output, isError: false, durationMs: 220, summary: "." })]));

  if (/修改|修复|改一下|bug|超时/.test(text)) {
    const editPartID = uid("part");
    const editCallID = uid("call");
    const approvalID = uid("ap");
    const approvalPartID = uid("part");
    const args = {
      path: "internal/auth/session.go",
      old_string: "const refreshWindow = 10 * time.Minute",
      new_string: "const refreshWindow = 12 * time.Minute",
    };
    addPart(thread.id, messageID, {
      id: editPartID, kind: "tool", seq: 4,
      data: { partId: editPartID, toolCallId: editCallID, name: "fs_edit", args, risk: "R1", status: "running", summary: "internal/auth/session.go" },
    });
    dispatch(batch(thread.id, [envelope(EVENT.toolStarted, thread.id, runID, { partId: editPartID, toolCallId: editCallID, name: "fs_edit", args, risk: "R1", status: "running", summary: "internal/auth/session.go" })]));

    addPart(thread.id, messageID, {
      id: approvalPartID, kind: "approval", seq: 5,
      data: {
        partId: approvalPartID, approvalId: approvalID, toolCallId: editCallID,
        tool: "fs_edit", risk: "R1", title: "修改文件",
        target: "internal/auth/session.go", detail: "写入工作区",
        facts: [
          { label: "风险等级", value: "R1" },
          { label: "操作", value: "internal/auth/session.go" },
          { label: "原因", value: "写入工作区" },
        ],
        args,
        allowPermanent: true, status: "waiting",
      },
    });
    dispatch(batch(thread.id, [envelope(EVENT.approvalRequested, thread.id, runID, {
      partId: approvalPartID, approvalId: approvalID, toolCallId: editCallID,
      tool: "fs_edit", risk: "R1", title: "修改文件",
      target: "internal/auth/session.go", detail: "写入工作区",
      facts: [
        { label: "风险等级", value: "R1" },
        { label: "操作", value: "internal/auth/session.go" },
        { label: "原因", value: "写入工作区" },
      ],
      args,
      allowPermanent: true, status: "waiting",
    })]));

    const approved = await new Promise<boolean>((resolve) => {
      approvalResolvers.set(approvalID, resolve);
    });
    approvalResolvers.delete(approvalID);
    dispatch(batch(thread.id, [envelope(EVENT.approvalResolved, thread.id, runID, {
      partId: approvalPartID, approvalId: approvalID,
      status: approved ? "approved" : "denied",
      decision: approved ? "allow" : "deny",
      scope: "once",
    })]));

    dispatch(batch(thread.id, [envelope(EVENT.toolCompleted, thread.id, runID, {
      partId: editPartID, toolCallId: editCallID, name: "fs_edit", args, risk: "R1",
      status: approved ? "done" : "error", output: approved ? "已替换 internal/auth/session.go 中的 1 处内容" : "用户拒绝了该操作",
      isError: !approved, durationMs: 48, summary: "internal/auth/session.go",
    })]));

    const finalID = uid("part");
    const finalText = approved
      ? "已完成修改：把刷新窗口从 10 分钟调整为 12 分钟。\n\n```diff\n-const refreshWindow = 10 * time.Minute\n+const refreshWindow = 12 * time.Minute\n```\n\n建议补一条 11 分钟时的刷新用例。"
      : "已按你的决定跳过写入，文件保持不变。";
    addPart(thread.id, messageID, { id: finalID, kind: "text", seq: 6, data: { text: finalText } });
    dispatch(batch(thread.id, [envelope(EVENT.textCompleted, thread.id, runID, { partId: finalID, text: finalText })]));
  }

  thread.status = "idle";
  thread.updatedAt = Date.now();
  dispatch(batch(thread.id, [
    envelope(EVENT.runCompleted, thread.id, runID, { status: "done" }),
    envelope(EVENT.threadUpdated, thread.id, runID, { threadId: thread.id, status: "idle", thread: { ...thread } }),
  ]));
}

function computeDiff(oldText: string, newText: string): DiffResult {
  const oldLines = oldText.replace(/\r\n/g, "\n").replace(/\n$/, "").split("\n");
  const newLines = newText.replace(/\r\n/g, "\n").replace(/\n$/, "").split("\n");
  if (oldText === newText) {
    return { additions: 0, deletions: 0, hunks: [], truncated: false };
  }
  let prefix = 0;
  while (prefix < oldLines.length && prefix < newLines.length && oldLines[prefix] === newLines[prefix]) {
    prefix += 1;
  }
  let suffix = 0;
  while (
    suffix < oldLines.length - prefix &&
    suffix < newLines.length - prefix &&
    oldLines[oldLines.length - 1 - suffix] === newLines[newLines.length - 1 - suffix]
  ) {
    suffix += 1;
  }
  const middleOld = oldLines.slice(prefix, oldLines.length - suffix);
  const middleNew = newLines.slice(prefix, newLines.length - suffix);
  const lines: DiffResult["hunks"][number]["lines"] = [];
  for (let index = 0; index < prefix; index += 1) {
    lines.push({ kind: "ctx", old: index + 1, new: index + 1, text: oldLines[index] });
  }
  middleOld.forEach((text, index) => lines.push({ kind: "del", old: prefix + index + 1, text }));
  middleNew.forEach((text, index) => lines.push({ kind: "add", new: prefix + index + 1, text }));
  for (let index = 0; index < suffix; index += 1) {
    lines.push({
      kind: "ctx",
      old: oldLines.length - suffix + index + 1,
      new: newLines.length - suffix + index + 1,
      text: oldLines[oldLines.length - suffix + index],
    });
  }
  return {
    additions: middleNew.length,
    deletions: middleOld.length,
    hunks: [{ header: `@@ -${prefix + 1},${middleOld.length} +${prefix + 1},${middleNew.length} @@`, lines }],
    truncated: false,
  };
}

export const mockApi: Api = {
  app: {
    info: async () => ({
      name: "CraftWork", version: "0.1.0", commit: "mock", buildTime: "",
      platform: "web", arch: "browser", dataDir: "~/.craftwork",
    }),
    openExternal: async (url) => {
      window.open(url, "_blank", "noopener");
    },
    pickFiles: async () => ["internal/auth/session.go"],
    writeClipboard: async (text) => {
      await navigator.clipboard?.writeText(text);
      return true;
    },
    readClipboard: async () => navigator.clipboard?.readText() ?? "",
  },
  settings: {
    get: async () => ({ ...settings }),
    save: async (next) => {
      Object.assign(settings, next);
      return { ...settings };
    },
  },
  threads: {
    list: async (mode, includeArchived = false) =>
      (threads[mode] ?? []).filter((thread) => includeArchived || !thread.archived).map((thread) => ({ ...thread })),
    search: async (mode, query) => {
      const keyword = query.trim().toLowerCase();
      return (threads[mode] ?? []).filter((thread) => {
        if (thread.title.toLowerCase().includes(keyword)) {
          return true;
        }
        return (messages[thread.id] ?? []).some((message) =>
          message.parts.some((part) => String(part.data?.text ?? "").toLowerCase().includes(keyword)),
        );
      }).map((thread) => ({ ...thread }));
    },
    create: async (mode, workspaceID, title) => {
      const thread: ThreadView = {
        id: uid("th"), mode, title: title || "新对话", modelId: settings.defaultModel,
        modelName: "Local Demo", workspaceId: workspaceID, status: "idle", archived: false,
        tokens: 0, createdAt: Date.now(), updatedAt: Date.now(),
      };
      threads[mode] = [thread, ...(threads[mode] ?? [])];
      messages[thread.id] = [];
      return { ...thread };
    },
    rename: async (threadID, title) => {
      const thread = findThread(threadID);
      if (thread) {
        thread.title = title;
        thread.updatedAt = Date.now();
      }
    },
    setModel: async (threadID, modelID) => {
      const thread = findThread(threadID);
      if (thread) {
        thread.modelId = modelID;
      }
    },
    archive: async (threadID, archived) => {
      const thread = findThread(threadID);
      if (thread) {
        thread.archived = archived;
      }
    },
    remove: async (threadID) => {
      threads.code = threads.code.filter((thread) => thread.id !== threadID);
      threads.work = threads.work.filter((thread) => thread.id !== threadID);
      delete messages[threadID];
    },
    snapshot: async (threadID) => snapshot(threadID),
    snapshotPage: async (threadID, limit, beforeSeq) => {
      const full = snapshot(threadID);
      let messages = full.messages ?? [];
      if (beforeSeq > 0) {
        messages = messages.filter((message) => {
          const last = Math.max(0, ...((message.parts ?? []).map((part) => part.seq)));
          return last < beforeSeq;
        });
      }
      const hasMore = messages.length > limit;
      const page = hasMore ? messages.slice(-limit) : messages;
      const oldestSeq = page[0]?.parts?.[0]?.seq ?? 0;
      return { ...full, messages: page, hasMore, oldestSeq };
    },
  },
  chat: {
    send: async (threadID, text, attachments = []) => {
      const thread = findThread(threadID);
      if (!thread) {
        throw new Error("线程不存在");
      }
      messages[threadID] = [
        ...(messages[threadID] ?? []),
        {
          id: uid("msg"), threadId: threadID, role: "user", status: "complete",
          createdAt: Date.now(),
          parts: [{ id: uid("part"), kind: "text", seq: 0, data: { text, attachments } }],
        },
      ];
      const runID = uid("run");
      void simulateRun(thread, text);
      return runID;
    },
    regenerate: async (threadID) => {
      const thread = findThread(threadID);
      if (!thread) {
        throw new Error("线程不存在");
      }
      const lastUser = (messages[threadID] ?? []).filter((message) => message.role === "user").pop();
      const text = String(lastUser?.parts[0]?.data?.text ?? "");
      const runID = uid("run");
      void simulateRun(thread, text);
      return runID;
    },
    cancel: async (threadID) => {
      cancelled.add(threadID);
      const thread = findThread(threadID);
      if (thread) {
        thread.status = "idle";
      }
      dispatch(batch(threadID, [envelope(EVENT.runCanceled, threadID, "", { status: "canceled" })]));
      return true;
    },
    decideApproval: async (approvalID, allow) => {
      const resolver = approvalResolvers.get(approvalID);
      if (resolver) {
        resolver(allow);
      }
    },
  },
  workspace: {
    list: async () => workspaces.map((workspace) => ({ ...workspace })),
    pickFolder: async () => "~/Code/craftwork",
    add: async (path, trust) => {
      const workspace: WorkspaceView = {
        id: uid("ws"), name: basename(path), path, trust, createdAt: Date.now(), lastOpenedAt: Date.now(),
      };
      workspaces.unshift(workspace);
      return { ...workspace };
    },
    setActive: async (workspaceID) => {
      const workspace = workspaces.find((item) => item.id === workspaceID) ?? workspaces[0];
      settings.activeWorkspace = workspace.id;
      return { ...workspace };
    },
    active: async () => {
      const workspace = workspaces.find((item) => item.id === settings.activeWorkspace) ?? workspaces[0] ?? null;
      return workspace ? { ...workspace } : null;
    },
    setTrust: async (workspaceID, trust) => {
      const workspace = workspaces.find((item) => item.id === workspaceID);
      if (workspace) {
        workspace.trust = trust;
      }
    },
    remove: async (workspaceID) => {
      const index = workspaces.findIndex((item) => item.id === workspaceID);
      if (index >= 0) {
        workspaces.splice(index, 1);
      }
    },
    listDir: async (_workspaceID, path) => (tree[path || "."] ?? []).map((node) => ({ ...node })),
    readFile: async (_workspaceID, path) => {
      const content = files[path] ?? "";
      return {
        path, content, size: content.length, binary: false,
        language: path.endsWith(".go") ? "go" : path.endsWith(".tsx") ? "typescript" : "text",
        lines: content ? content.split("\n").length : 0,
        truncated: false, hash: String(content.length), modified: now,
      };
    },
    writeFile: async (_workspaceID, path, content) => {
      files[path] = content;
    },
    computeDiff: async (oldText, newText) => computeDiff(oldText, newText),
    gitStatuses: async () => ({ ...gitStatuses }),
  },
  providers: {
    list: async () => providers.map((provider) => ({ ...provider })),
    warnings: async () => [],
    setKey: async (providerID, key) => {
      const provider = providers.find((item) => item.id === providerID);
      if (provider) {
        provider.keySource = "file";
        provider.keyMasked = `${key.slice(0, 5)}…${key.slice(-4)}`;
        provider.available = true;
        provider.models = (provider.models ?? []).map((model) => ({ ...model, available: true }));
      }
    },
    clearKey: async (providerID) => {
      const provider = providers.find((item) => item.id === providerID);
      if (provider) {
        provider.keySource = "";
        provider.keyMasked = "";
        provider.available = provider.kind === "mock";
        provider.models = (provider.models ?? []).map((model) => ({ ...model, available: provider.available }));
      }
    },
    test: async (providerID) => {
      const provider = providers.find((item) => item.id === providerID);
      return { ok: Boolean(provider?.available), message: provider?.available ? "连接正常" : "尚未配置密钥", latencyMs: 128 };
    },
  },
  audit: {
    list: async (limit) => audit.slice(0, limit),
    usage: async () => usage,
    backups: async (limit) => backups.slice(0, limit),
    restore: async () => undefined,
  },
  documents: {
    preview: async (_workspaceID, path): Promise<DocumentPreview> => {
      const extension = path.split(".").pop() ?? "";
      if (["docx", "xlsx", "pdf"].includes(extension)) {
        return {
          kind: extension, path, name: basename(path), size: 48210, modified: now,
          supported: false, note: "结构化解析将在 M3 接入；当前仅展示文件信息。",
        };
      }
      const content = files[path] ?? "";
      return { kind: "text", path, name: basename(path), size: content.length, modified: now, supported: true, text: content };
    },
  },
  terminal: {
    list: async () => terminalSessions.map((session) => ({ ...session })),
    create: async (name, cwd) => {
      terminalCounter += 1;
      const session: TerminalView = {
        id: `term_${terminalCounter}`, name: name || `终端 ${terminalCounter}`,
        cwd: cwd || "~/Code/craftwork", running: true,
      };
      terminalSessions.push(session);
      window.setTimeout(() => {
        dispatch(batch(`terminal:${session.id}`, [
          envelope(EVENT.terminalOutput, `terminal:${session.id}`, "", {
            sessionId: session.id,
            data: "craftwork ~/Code/craftwork % ",
          }),
        ]));
      }, 120);
      return { ...session };
    },
    write: async (sessionID, data) => {
      const session = terminalSessions.find((item) => item.id === sessionID);
      if (!session) {
        return;
      }
      dispatch(batch(`terminal:${sessionID}`, [
        envelope(EVENT.terminalOutput, `terminal:${sessionID}`, "", { sessionId: sessionID, data }),
      ]));
      if (data.includes("\r")) {
        const command = data.replace("\r", "").trim();
        const output = command === "git status"
          ? "On branch main\nChanges not staged for commit:\n  modified: internal/auth/session.go\n"
          : command === "ls"
            ? "frontend  internal  README.md  go.mod\n"
            : `zsh: command not found: ${command}\n`;
        await sleep(80);
        dispatch(batch(`terminal:${sessionID}`, [
          envelope(EVENT.terminalOutput, `terminal:${sessionID}`, "", { sessionId: sessionID, data: output }),
        ]));
      }
    },
    resize: async () => undefined,
    close: async (sessionID) => {
      const index = terminalSessions.findIndex((item) => item.id === sessionID);
      if (index >= 0) {
        terminalSessions.splice(index, 1);
      }
    },
  },
};
