/**
 * 纯 UI 预览用的 Mock 后端。
 *
 * 只在 `npm run dev`（`vite --mode mock`）下接管 API，用于界面走查与
 * Playwright 回归；需要真实后端时用 server 模式（`task run:server`）或桌面端。
 * 它不读写磁盘，所有数据都在内存中。
 *
 * 启动流程：模块加载时就把下列内存数据（设置、工作区、会话、文件、消息、审计、用量等）
 * 准备好；界面调用 `api.*` 时直接读写这些对象。
 * 事件模拟：发送消息后 `simulateRun` 按「推理 → 正文 → 工具 → 审批 → 收尾」的顺序
 * 逐个派发事件批次，让前端走和真实后端完全相同的处理路径。
 */
import type { Api } from "./desktop";
import { dispatch, envelope } from "./bus";
import type {
  AuditEntry,
  Backup,
  CatalogMeta,
  CatalogModelView,
  CatalogProviderView,
  ChatMessage,
  ChatPart,
  DiffResult,
  DocumentPreview,
  FileNode,
  MCPServerView,
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

/** 模块加载时刻，后续所有示例数据都以它为基准倒推时间，保证每次预览的相对时间一致。 */
const now = Date.now();

/** Mock 的设置对象：界面保存设置时直接合并到这里，刷新页面后恢复初始值。 */
const settings: Settings = {
  theme: "system",
  mode: "code",
  panelWidth: 320,
  approvalLevel: "graded",
  defaultModel: "openai/gpt-5.1",
  fallbackModel: "",
  activeWorkspace: "ws_demo",
  language: "zh-CN",
  fontSize: 14,
  codeTheme: "dark",
  terminalFontSize: 12,
  maxIterations: 300,
  autoUpdate: true,
  notifications: true,
  updateChannel: "stable",
  telemetry: false,
};

/**
 * Mock 工作区列表：内置临时会话目录置顶，另带一个已受信的演示仓库。
 * purpose 字段与后端一致：conversation 是内置临时目录，project 是用户项目。
 */
const workspaces: WorkspaceView[] = [
  {
    id: "ws_conv",
    name: "临时会话",
    path: "~/.craftwork/workspace/default",
    trust: "trusted",
    purpose: "conversation",
    createdAt: now - 86400000 * 12,
    lastOpenedAt: now - 30000,
  },
  {
    id: "ws_demo",
    name: "craftwork",
    path: "~/Code/craftwork",
    trust: "trusted",
    purpose: "project",
    createdAt: now - 86400000 * 12,
    lastOpenedAt: now - 60000,
  },
];

/** Mock 会话：按模式分组，Code 与 Work 各三条（含临时会话），用于展示列表分组、搜索与模式切换联动。 */
const threads: Record<string, ThreadView[]> = {
  code: [
    {
      id: "th_code_0",
      mode: "code",
      title: "随便聊聊：周末去哪",
      modelId: "openai/gpt-5.1",
      modelName: "GPT-5.1",
      workspaceId: "ws_conv",
      status: "idle",
      archived: false,
      tokens: 3210,
      contextTokens: 2010,
      contextCacheTokens: 1640,
      contextSpeed: 41.0,
      createdAt: now - 600000,
      updatedAt: now - 60000,
    },
    {
      id: "th_code_1",
      mode: "code",
      title: "修复登录刷新超时",
      modelId: "openai/gpt-5.1",
      modelName: "GPT-5.1",
      workspaceId: "ws_demo",
      status: "idle",
      archived: false,
      tokens: 18420,
      contextTokens: 9210,
      contextCacheTokens: 8140,
      contextSpeed: 38.6,
      createdAt: now - 3600000,
      updatedAt: now - 120000,
    },
    {
      id: "th_code_2",
      mode: "code",
      title: "为工作区搜索补上中文分词",
      modelId: "openai/gpt-5.1",
      modelName: "GPT-5.1",
      workspaceId: "ws_demo",
      status: "idle",
      archived: false,
      tokens: 7310,
      contextTokens: 5120,
      contextCacheTokens: 4310,
      contextSpeed: 45.2,
      createdAt: now - 7200000,
      updatedAt: now - 3600000,
    },
  ],
  work: [
    {
      id: "th_work_0",
      mode: "work",
      title: "帮忙起草一周安排",
      modelId: "openai/gpt-5.1",
      modelName: "GPT-5.1",
      workspaceId: "ws_conv",
      status: "idle",
      archived: false,
      tokens: 2210,
      contextTokens: 1480,
      contextCacheTokens: 1180,
      contextSpeed: 36.8,
      createdAt: now - 3600000,
      updatedAt: now - 300000,
    },
    {
      id: "th_work_1",
      mode: "work",
      title: "整理季度会议纪要",
      modelId: "openai/gpt-5.1",
      modelName: "GPT-5.1",
      workspaceId: "ws_demo",
      status: "idle",
      archived: false,
      tokens: 9640,
      contextTokens: 6480,
      contextCacheTokens: 5940,
      contextSpeed: 31.4,
      createdAt: now - 86400000,
      updatedAt: now - 1800000,
    },
    {
      id: "th_work_2",
      mode: "work",
      title: "修正预算表合计公式",
      modelId: "openai/gpt-5.1",
      modelName: "GPT-5.1",
      workspaceId: "ws_demo",
      status: "idle",
      archived: false,
      tokens: 4120,
      contextTokens: 2860,
      contextCacheTokens: 2310,
      contextSpeed: 52.7,
      createdAt: now - 86400000 * 2,
      updatedAt: now - 7200000,
    },
  ],
};

/** Mock 文件内容：路径到文本内容的映射，读取与写入都直接操作这里。 */
const files: Record<string, string> = {
  "README.md":
    "# craftwork\n\n一个用于演示 CraftWork 的 Go 仓库。\n\n## 常用命令\n\n```bash\ngo test ./...\ngo build ./...\n```\n",
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

/** Mock 文件树：目录路径到子节点列表的映射，`"."` 是根目录。 */
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
  "internal/app": [{ name: "app.go", path: "internal/app/app.go", type: "file", size: 48, modified: now }],
  "internal/auth": [{ name: "session.go", path: "internal/auth/session.go", type: "file", size: 420, modified: now }],
  frontend: [{ name: "src", path: "frontend/src", type: "dir", size: 0, modified: now }],
  "frontend/src": [{ name: "App.tsx", path: "frontend/src/App.tsx", type: "file", size: 72, modified: now }],
};

/** Mock 的 Git 状态：文件路径到状态字母（M 表示已修改）。 */
const gitStatuses: Record<string, string> = {
  "internal/auth/session.go": "M",
};

/**
 * 组装一条已完成的助手消息，供初始示例数据复用。
 *
 * 所有示例助手消息的时间都相同、状态都是 complete，避免列表里出现不一致的「进行中」。
 * 带过程部件的消息附带 workDurationMs 与推理时长，让纯 UI 预览也能看到
 * 「已工作 N」折叠与「思考过程 · 持续了 N 秒」的最终形态。
 */
function assistantMessage(id: string, threadId: string, parts: ChatPart[], workDurationMs?: number): ChatMessage {
  return {
    id,
    threadId,
    role: "assistant",
    status: "complete",
    createdAt: now - 110000,
    ...(workDurationMs ? { workDurationMs } : {}),
    parts,
  };
}

/** Mock 对话消息：会话 ID 到消息列表的映射，包含文本、推理与工具调用三类片段。 */
const messages: Record<string, ChatMessage[]> = {
  th_code_1: [
    {
      id: "m1_user",
      threadId: "th_code_1",
      role: "user",
      status: "complete",
      createdAt: now - 180000,
      parts: [
        {
          id: "p1",
          kind: "text",
          seq: 1,
          data: {
            text: "登录后 15 分钟刷新令牌就超时了，帮我看看 `internal/auth/session.go` 里的刷新逻辑。",
            attachments: ["~/.craftwork/attachments/att_demo_报错截图.png"],
          },
        },
      ],
    },
    assistantMessage(
      "m1_assistant",
      "th_code_1",
      [
        {
          id: "p2",
          kind: "reasoning",
          seq: 2,
          data: { text: "先确认 session.go 的刷新判断，再看测试覆盖。", durationMs: 8400 },
        },
        {
          id: "p3",
          kind: "text",
          seq: 3,
          data: {
            text: "我先读取 `internal/auth/session.go`，确认刷新窗口的计算方式。\n\n关键代码在 `internal/auth/session.go:12`：\n\n```go\nconst refreshWindow = 10 * time.Minute\n\nfunc (s *Session) NeedsRefresh(now time.Time) bool {\n\treturn now.Sub(s.RefreshedAt) > refreshWindow\n}\n```\n\n`refreshWindow` 固定为 10 分钟，而令牌实际有效期是 15 分钟，所以第 11 分钟就会误判为过期。",
          },
        },
        {
          id: "p4",
          kind: "tool",
          seq: 4,
          data: {
            partId: "p4",
            toolCallId: "call_read",
            name: "fs_read",
            args: { path: "internal/auth/session.go" },
            risk: "R0",
            status: "done",
            output: "internal/auth/session.go 共 22 行，已读取。",
            isError: false,
            durationMs: 32,
            summary: "internal/auth/session.go",
          },
        },
        {
          id: "p5",
          kind: "text",
          seq: 5,
          data: {
            text: "建议把刷新窗口改为令牌有效期的 80%（12 分钟），并在刷新失败时回退到重新登录。改动集中在 `internal/auth/session.go:6` 与对应单测。",
          },
        },
      ],
      96500,
    ),
  ],
  th_code_2: [
    {
      id: "m2_user",
      threadId: "th_code_2",
      role: "user",
      status: "complete",
      createdAt: now - 3700000,
      parts: [{ id: "p1", kind: "text", seq: 1, data: { text: "会话搜索现在只能匹配标题，怎么补上消息内容命中？" } }],
    },
    assistantMessage("m2_assistant", "th_code_2", [
      {
        id: "p2",
        kind: "text",
        seq: 2,
        data: {
          text: "搜索入口在 `internal/service/threads.go`，现在只查 `threads.title`。\n\n建议用 `LEFT JOIN events` 对 `payload` 做 `LIKE`，并对 `thread_id + updated_at` 建索引；中文查询不需要额外分词，子串匹配即可满足 F-TH-03。",
        },
      },
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
      {
        id: "p2",
        kind: "text",
        seq: 2,
        data: {
          text: "已读取文档并整理出 4 条待办：\n\n| 负责人 | 事项 | 截止 |\n| --- | --- | --- |\n| 林岚 | 补充预算说明 | 10/24 |\n| 周野 | 对齐发布节奏 | 10/26 |\n| 陈默 | 更新接口文档 | 10/28 |\n\n文档写入需要审批；批准前不会修改原文件。",
        },
      },
    ]),
  ],
  th_work_2: [
    {
      id: "m4_user",
      threadId: "th_work_2",
      role: "user",
      status: "complete",
      createdAt: now - 7300000,
      parts: [
        { id: "p1", kind: "text", seq: 1, data: { text: "预算表的合计公式错了，B8 / C8 / D8 应该按季度汇总。" } },
      ],
    },
    assistantMessage("m4_assistant", "th_work_2", [
      {
        id: "p2",
        kind: "text",
        seq: 2,
        data: { text: "检查后确认 `B8` 只汇总了 7 月。建议改为 `=SUM(B2:B7)`，C8、D8 同理。修改前会备份原文件。" },
      },
    ]),
  ],
};

/** Mock 审计记录：按时间倒序，供数据面板的审计分区展示。 */
const audit: AuditEntry[] = [
  {
    id: 3,
    ts: now - 120000,
    threadId: "th_code_1",
    runId: "run_demo",
    tool: "fs_read",
    risk: "R0",
    decision: "auto",
    scope: "auto",
    detail: "internal/auth/session.go",
  },
  {
    id: 2,
    ts: now - 180000,
    threadId: "th_code_1",
    runId: "run_demo",
    tool: "todo_write",
    risk: "R0",
    decision: "auto",
    scope: "auto",
    detail: "更新任务清单",
  },
  {
    id: 1,
    ts: now - 1900000,
    threadId: "th_work_1",
    runId: "run_work",
    tool: "doc_read",
    risk: "R0",
    decision: "auto",
    scope: "auto",
    detail: "季度会议纪要.docx",
  },
];

/** Mock 用量统计：今天 / 本周 / 本月汇总、按模型拆分，以及最近 14 天的趋势。 */
const usage: UsageSummary = {
  today: { inputTokens: 18420, outputTokens: 3120, cacheTokens: 2100, costMicros: 42000, runs: 6 },
  week: { inputTokens: 98240, outputTokens: 18400, cacheTokens: 12400, costMicros: 210000, runs: 31 },
  month: { inputTokens: 402100, outputTokens: 72100, cacheTokens: 50200, costMicros: 880000, runs: 124 },
  byModel: [
    { modelId: "openai/gpt-5.1", inputTokens: 302100, outputTokens: 52100, costMicros: 500000, runs: 108 },
    { modelId: "openrouter/x-ai/grok-4.5", inputTokens: 100000, outputTokens: 20000, costMicros: 380000, runs: 16 },
  ],
  byDay: Array.from({ length: 14 }, (_, index) => ({
    day: new Date(now - (13 - index) * 86400000).toISOString().slice(0, 10),
    inputTokens: 12000 + index * 900,
    outputTokens: 2400 + index * 120,
    costMicros: 12000 + index * 900,
    runs: 3 + (index % 4),
  })),
};

/** Mock 备份记录：展示文件写入前的备份时间、大小与哈希。 */
const backups: Backup[] = [
  {
    id: "bk_demo",
    path: "季度会议纪要.docx",
    backupPath: "~/.craftwork/backups/季度会议纪要.docx.20261031-142210",
    hash: "9f2c",
    size: 48120,
    createdAt: now - 1900000,
  },
  {
    id: "bk_demo2",
    path: "内部/auth/session.go",
    backupPath: "~/.craftwork/backups/session.go.20261031-143002",
    hash: "a17d",
    size: 420,
    createdAt: now - 120000,
  },
];

/** Mock 的 MCP 服务器列表，默认为空，添加后才有数据。 */
const mcpServers: MCPServerView[] = [];

/**
 * Mock 提供商列表：默认只有一个已配置环境变量的 OpenAI。
 *
 * 产品里所有提供商都由用户添加，这里保留一个可用条目只是为了让纯 UI 预览
 * 能直接走通对话；它不代表产品内置了任何提供商。
 */
const providers: ProviderView[] = [
  {
    id: "openai",
    name: "OpenAI",
    kind: "openai",
    endpoint: "https://api.openai.com/v1",
    keyRequired: true,
    keySource: "env",
    keyMasked: "OPENAI…_KEY",
    available: true,
    models: [
      {
        id: "gpt-5.1",
        ref: "openai/gpt-5.1",
        name: "GPT-5.1",
        provider: "openai",
        providerName: "OpenAI",
        contextK: 400,
        maxTokens: 128000,
        caps: { stream: true, tools: true, vision: true, reasoning: true, structured: true },
        cost: { inputPerMTok: 1.25, outputPerMTok: 10, cacheReadPerMTok: 0.125 },
        status: "",
        canonicalId: "openai/gpt-5.1",
        releaseDate: "2025-11-13",
        available: true,
      },
    ],
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    kind: "openai-compatible",
    endpoint: "https://openrouter.ai/api/v1",
    keyRequired: true,
    keySource: "",
    keyMasked: "",
    available: false,
    models: [
      {
        id: "x-ai/grok-4.5",
        ref: "openrouter/x-ai/grok-4.5",
        name: "Grok 4.5",
        provider: "openrouter",
        providerName: "OpenRouter",
        contextK: 500,
        maxTokens: 450000,
        caps: { stream: true, tools: true, vision: true, reasoning: true, structured: true },
        cost: { inputPerMTok: 2, outputPerMTok: 6, cacheReadPerMTok: 0.3 },
        status: "",
        canonicalId: "xai/grok-4.5",
        releaseDate: "2026-08-01",
        available: false,
      },
    ],
  },
];

/** Mock 目录元信息，字段与 Go 侧 CatalogMeta 对齐。 */
const catalogMeta: CatalogMeta = {
  schema: 1,
  generatedAt: "2026-11-01T10:06:32Z",
  version: "mock-catalog",
  source: "https://models.dev/api.json",
  license: "MIT",
  providerCount: 2,
  modelCount: 2,
};

/** Mock 目录提供商，仅用于纯 UI 预览“从模型目录添加”的流程。 */
const catalogProviders: CatalogProviderView[] = [
  {
    id: "groq",
    name: "Groq",
    kind: "openai-compatible",
    endpoint: "https://api.groq.com/openai/v1",
    doc: "https://console.groq.com/docs/models",
    env: ["GROQ_API_KEY"],
    modelCount: 1,
    added: false,
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    kind: "openai-compatible",
    endpoint: "https://openrouter.ai/api/v1",
    doc: "https://openrouter.ai/models",
    env: ["OPENROUTER_API_KEY"],
    modelCount: 1,
    added: false,
  },
];

/** Mock 目录模型，按提供商标识分组。 */
const catalogModels: Record<string, CatalogModelView[]> = {
  groq: [
    {
      id: "qwen/qwen3.8-27b",
      ref: "groq/qwen/qwen3.8-27b",
      name: "Qwen3.8 27B",
      provider: "groq",
      providerName: "Groq",
      contextK: 132,
      maxTokens: 32768,
      caps: { stream: true, tools: true, vision: false, reasoning: true, structured: true },
      cost: { inputPerMTok: 0.8, outputPerMTok: 4 },
      status: "",
      releaseDate: "2026-09-01",
    },
  ],
  openrouter: [
    {
      id: "x-ai/grok-4.5",
      ref: "openrouter/x-ai/grok-4.5",
      name: "Grok 4.5",
      provider: "openrouter",
      providerName: "OpenRouter",
      contextK: 500,
      maxTokens: 450000,
      caps: { stream: true, tools: true, vision: true, reasoning: true, structured: true },
      cost: { inputPerMTok: 2, outputPerMTok: 6, cacheReadPerMTok: 0.3 },
      status: "",
      releaseDate: "2026-08-01",
    },
  ],
};

/** Mock 终端会话列表。 */
const terminalSessions: TerminalView[] = [];
/** 终端会话自增编号，用于生成 `term_1` 这类稳定的 ID。 */
let terminalCounter = 0;
/** 已被用户取消运行的会话 ID 集合；`simulateRun` 在每个阶段之间检查它。 */
const cancelled = new Set<string>();
/** 运行中引导的待投递队列：会话 ID 到排队指令的映射，`simulateRun` 在阶段边界消费。 */
const steerQueue = new Map<string, Array<{ text: string; attachments: string[] }>>();
/** 仍允许首轮自动命名的会话 ID；用户改名或完成命名后移除。 */
const autoTitleThreads = new Set<string>();
/**
 * 待审批请求的放行函数：审批 ID 到 resolve 函数的映射。
 * `simulateRun` 在这里挂起等待，`decideApproval` 调用对应函数让流程继续。
 */
const approvalResolvers = new Map<string, (allow: boolean) => void>();

/** 在 Code 与 Work 两个模式的会话列表里查找指定会话。 */
function findThread(threadID: string): ThreadView | undefined {
  return [...threads.code, ...threads.work].find((thread) => thread.id === threadID);
}

/**
 * 组装一个会话快照。
 *
 * 快照包含会话本身、全部消息、事件序号、待审批列表与分页标记。
 * 这里把所有仍在 waiting 的审批片段收集成审批列表，让界面刷新后也能恢复待审批状态；
 * 会话不存在时抛出错误，由调用方转成用户可读的提示。
 */
function snapshot(threadID: string): ThreadSnapshot {
  const thread = findThread(threadID);
  if (!thread) {
    throw new Error("会话不存在");
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
        requestedAt: 0,
        decidedAt: 0,
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
      ...(message.workDurationMs ? { workDurationMs: message.workDurationMs } : {}),
      parts: message.parts.map((part) => ({ id: part.id, kind: part.kind, seq: part.seq, data: part.data })),
    })),
    lastSeq: list.reduce((total, message) => total + message.parts.length, 0),
    approvals,
    hasMore: false,
    oldestSeq: 0,
  };
}

/** Mock 没有真实模型可调用，也不复刻后端的降级提取规则：取首行截断即可，标题规则以 Go 为唯一事实源。 */
function mockAutoTitle(text: string): string {
  const line =
    text
      .split("\n")
      .map((item) => item.trim())
      .find(Boolean) ?? "";
  const runes = [...line];
  if (runes.length === 0) {
    return "";
  }
  return runes.length > 20 ? `${runes.slice(0, 19).join("")}…` : line;
}

/** 把事件数组包装成前端约定的批次结构，版本号固定为 1。 */
function batch(threadID: string, events: ReturnType<typeof envelope>[]) {
  return { v: 1, threadId: threadID, events };
}

/** 向指定消息追加一个片段；消息不存在时静默忽略。 */
function addPart(threadID: string, messageID: string, part: ChatPart): void {
  const list = messages[threadID] ?? [];
  const message = list.find((item) => item.id === messageID);
  if (message) {
    message.parts = [...message.parts, part];
  }
}

/** 取走会话全部待投递的运行中引导（对应后端 Runner 的投递队列）。 */
function takeSteers(threadID: string): Array<{ text: string; attachments: string[] }> {
  const queue = steerQueue.get(threadID);
  steerQueue.delete(threadID);
  return queue ?? [];
}

/**
 * 模拟一次完整的助手运行，是 Mock 的核心。
 *
 * 事件顺序：
 * 1. 先插入一条 streaming 状态的助手消息，派发 `runStarted`；
 * 2. 推理阶段：按小块派发 `reasoningDelta`，最后 `reasoningCompleted`；
 * 3. 正文阶段：按小块派发 `textDelta`，最后 `textCompleted`；
 * 4. 工具阶段：派发 `toolStarted`，等待一小段后 `toolCompleted`；
 * 5. 如果用户消息里出现「修改 / 修复 / bug / 超时」等关键词，再模拟一次文件编辑：
 *    派发 `approvalRequested` 后挂起，直到 `decideApproval` 放行，
 *    然后按用户的选择派发 `approvalResolved` 与工具结果，并补一段收尾正文；
 * 6. 收尾：把会话状态改回 idle，派发 `runCompleted` 与 `threadUpdated`。
 *
 * 每个阶段之间都检查 `cancelled`：一旦用户取消，立刻停止派发剩余事件。
 */
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
  dispatch(
    batch(thread.id, [envelope(EVENT.runStarted, thread.id, runID, { messageId: messageID, modelId: thread.modelId })]),
  );

  const reasoningID = uid("part");
  addPart(thread.id, messageID, { id: reasoningID, kind: "reasoning", seq: 1, data: { text: "" } });
  // 推理文本按 5 个字符一片送出，每片间隔 22ms，模拟模型逐字思考的节奏。
  for (const piece of chunkText("我先确认工作区的文件结构，再给出结论。", 5)) {
    if (cancelled.has(thread.id)) {
      return;
    }
    dispatch(
      batch(thread.id, [envelope(EVENT.reasoningDelta, thread.id, runID, { partId: reasoningID, text: piece })]),
    );
    await sleep(22);
  }
  dispatch(
    batch(thread.id, [
      envelope(EVENT.reasoningCompleted, thread.id, runID, {
        partId: reasoningID,
        text: "我先确认工作区的文件结构，再给出结论。",
      }),
    ]),
  );

  const textID = uid("part");
  addPart(thread.id, messageID, { id: textID, kind: "text", seq: 2, data: { text: "" } });
  // 引导投递点（对应后端的迭代边界）：推理结束后，把已排队的引导并入本轮回复。
  const steered = takeSteers(thread.id);
  const steerNote = steered.map((item) => item.text).join("；");
  // 正文里回显用户输入的前 80 个字符，便于在预览中确认「回复的是哪条消息」。
  const answer = steerNote
    ? `我先看一下当前工作区的文件。\n\n你刚才提到：**${text.slice(0, 80)}**\n\n收到你的补充：**${steerNote}**，我会把它并入当前任务一起处理。`
    : `我先看一下当前工作区的文件。\n\n你刚才提到：**${text.slice(0, 80)}**\n\n我会先读取相关文件，再决定是否需要修改。`;
  // 正文按 4 个字符一片、12ms 间隔送出，比推理阶段更快，接近真实模型的输出观感。
  for (const piece of chunkText(answer, 4)) {
    if (cancelled.has(thread.id)) {
      return;
    }
    dispatch(batch(thread.id, [envelope(EVENT.textDelta, thread.id, runID, { partId: textID, text: piece })]));
    await sleep(12);
  }
  dispatch(batch(thread.id, [envelope(EVENT.textCompleted, thread.id, runID, { partId: textID, text: answer })]));

  // 工具阶段：先以 running 状态出现，220ms 后补上结果，界面据此展示耗时与输出。
  const toolID = uid("part");
  const toolCallID = uid("call");
  addPart(thread.id, messageID, {
    id: toolID,
    kind: "tool",
    seq: 3,
    data: {
      partId: toolID,
      toolCallId: toolCallID,
      name: "fs_list",
      args: { path: "." },
      risk: "R0",
      status: "running",
      summary: ".",
    },
  });
  dispatch(
    batch(thread.id, [
      envelope(EVENT.toolStarted, thread.id, runID, {
        partId: toolID,
        toolCallId: toolCallID,
        name: "fs_list",
        args: { path: "." },
        risk: "R0",
        status: "running",
        summary: ".",
      }),
    ]),
  );
  await sleep(220);
  const output = "frontend/\ninternal/\nREADME.md\ngo.mod";
  dispatch(
    batch(thread.id, [
      envelope(EVENT.toolCompleted, thread.id, runID, {
        partId: toolID,
        toolCallId: toolCallID,
        name: "fs_list",
        args: { path: "." },
        risk: "R0",
        status: "done",
        output,
        isError: false,
        durationMs: 220,
        summary: ".",
      }),
    ]),
  );

  // 只有用户消息里出现修改类关键词时才走审批分支，方便预览时分别验证
  // 「只读对话」与「写入并审批」两条路径。
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
      id: editPartID,
      kind: "tool",
      seq: 4,
      data: {
        partId: editPartID,
        toolCallId: editCallID,
        name: "fs_edit",
        args,
        risk: "R1",
        status: "running",
        summary: "internal/auth/session.go",
      },
    });
    dispatch(
      batch(thread.id, [
        envelope(EVENT.toolStarted, thread.id, runID, {
          partId: editPartID,
          toolCallId: editCallID,
          name: "fs_edit",
          args,
          risk: "R1",
          status: "running",
          summary: "internal/auth/session.go",
        }),
      ]),
    );

    addPart(thread.id, messageID, {
      id: approvalPartID,
      kind: "approval",
      seq: 5,
      data: {
        partId: approvalPartID,
        approvalId: approvalID,
        toolCallId: editCallID,
        tool: "fs_edit",
        risk: "R1",
        title: "修改文件",
        target: "internal/auth/session.go",
        detail: "写入工作区",
        facts: [
          { label: "风险等级", value: "R1" },
          { label: "操作", value: "internal/auth/session.go" },
          { label: "原因", value: "写入工作区" },
        ],
        args,
        allowPermanent: true,
        status: "waiting",
      },
    });
    dispatch(
      batch(thread.id, [
        envelope(EVENT.approvalRequested, thread.id, runID, {
          partId: approvalPartID,
          approvalId: approvalID,
          toolCallId: editCallID,
          tool: "fs_edit",
          risk: "R1",
          title: "修改文件",
          target: "internal/auth/session.go",
          detail: "写入工作区",
          facts: [
            { label: "风险等级", value: "R1" },
            { label: "操作", value: "internal/auth/session.go" },
            { label: "原因", value: "写入工作区" },
          ],
          args,
          allowPermanent: true,
          status: "waiting",
        }),
      ]),
    );

    // 审批挂起：把 resolve 存进 Map，流程在这里 await 住；
    // 用户在前端点「允许 / 拒绝」时，ChatService.decideApproval 会调用它并把它从 Map 删除。
    // 这里必须等 Promise 落定后再继续派发，才能保证事件顺序与真实后端一致。
    const approved = await new Promise<boolean>((resolve) => {
      approvalResolvers.set(approvalID, resolve);
    });
    approvalResolvers.delete(approvalID);
    // 无论允许还是拒绝都要派发 approvalResolved，界面据此把审批卡片从 waiting 改为最终状态。
    dispatch(
      batch(thread.id, [
        envelope(EVENT.approvalResolved, thread.id, runID, {
          partId: approvalPartID,
          approvalId: approvalID,
          status: approved ? "approved" : "denied",
          decision: approved ? "allow" : "deny",
          scope: "once",
        }),
      ]),
    );

    // 工具结果跟随审批结论：允许则成功，拒绝则标记为错误并说明原因。
    dispatch(
      batch(thread.id, [
        envelope(EVENT.toolCompleted, thread.id, runID, {
          partId: editPartID,
          toolCallId: editCallID,
          name: "fs_edit",
          args,
          risk: "R1",
          status: approved ? "done" : "error",
          output: approved ? "已替换 internal/auth/session.go 中的 1 处内容" : "用户拒绝了该操作",
          isError: !approved,
          durationMs: 48,
          summary: "internal/auth/session.go",
        }),
      ]),
    );

    const finalID = uid("part");
    const finalText = approved
      ? "已完成修改：把刷新窗口从 10 分钟调整为 12 分钟。\n\n```diff\n-const refreshWindow = 10 * time.Minute\n+const refreshWindow = 12 * time.Minute\n```\n\n建议补一条 11 分钟时的刷新用例。"
      : "已按你的决定跳过写入，文件保持不变。";
    addPart(thread.id, messageID, { id: finalID, kind: "text", seq: 6, data: { text: finalText } });
    dispatch(batch(thread.id, [envelope(EVENT.textCompleted, thread.id, runID, { partId: finalID, text: finalText })]));
  }

  // 收尾前再查一次引导队列：推理结束后才入队的引导由接力运行处理，
  // 与后端「运行正常结束后排队引导自动接力」的行为一致。
  const late = takeSteers(thread.id);
  if (late.length > 0 && !cancelled.has(thread.id)) {
    dispatch(batch(thread.id, [envelope(EVENT.runCompleted, thread.id, runID, { status: "done" })]));
    void simulateRun(thread, late.map((item) => item.text).join("\n"));
    return;
  }

  thread.status = "idle";
  thread.updatedAt = Date.now();
  // 模拟一轮用量：tokens 走累加语义，上下文占用与速度走覆盖语义，
  // 让预览环境的圆环、速度与详情面板和真实后端行为一致。
  const usageInput = 1200 + Math.round(Math.random() * 400);
  const usageOutput = 180 + Math.round(Math.random() * 120);
  thread.tokens += usageInput + usageOutput;
  thread.contextTokens = usageInput + usageOutput;
  thread.contextCacheTokens = usageInput - 200;
  thread.contextSpeed = Math.round((usageOutput / 2.4) * 10) / 10;
  dispatch(
    batch(thread.id, [
      envelope(EVENT.usageUpdated, thread.id, runID, {
        inputTokens: usageInput,
        outputTokens: usageOutput,
        costMicros: 0,
        contextTokens: usageInput + usageOutput,
        cacheTokens: usageInput - 200,
        durationMs: 2400,
        tokensPerSecond: Math.round((usageOutput / 2.4) * 10) / 10,
      }),
      envelope(EVENT.runCompleted, thread.id, runID, { status: "done" }),
      envelope(EVENT.threadUpdated, thread.id, runID, { threadId: thread.id, status: "idle", thread: { ...thread } }),
    ]),
  );
}

/**
 * 计算一份简化的行级差异，供预览环境展示差异面板。
 *
 * 算法：
 * 1. 统一换行符并去掉末尾换行，再按行切分；
 * 2. 内容完全相同则返回空差异；
 * 3. 从前往后找出公共前缀，从后往前找出公共后缀，中间部分才是真正变化的内容；
 * 4. 中间部分先整体标为删除、再整体标为新增（不做行内精细匹配），
 *    对预览演示足够，真实后端会用更精细的差异算法；
 * 5. 前后缀作为上下文行输出，最后拼出 `@@ -a,b +c,d @@` 形式的块头。
 */
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

/**
 * Mock 后端实现：形状与 `desktopApi` 完全一致，业务代码可以无缝切换。
 *
 * 所有方法都是异步的（即使内部是同步操作），保证调用方写法与真实环境一致。
 * 返回值多做一层浅拷贝，避免界面直接修改 Mock 的内部数据。
 */
export const mockApi: Api = {
  /** 应用级信息与外链、文件选择、剪贴板。 */
  app: {
    /** 应用信息：平台固定为 web，用于界面显示版本与数据目录。 */
    info: async () => ({
      name: "CraftWork",
      version: "0.1.0",
      commit: "mock",
      buildTime: "",
      platform: "web",
      arch: "browser",
      dataDir: "~/.craftwork",
    }),
    /** 打开外链：浏览器环境下直接开新标签页。 */
    openExternal: async (url) => {
      window.open(url, "_blank", "noopener");
    },
    /** 模拟系统文件选择器，固定返回一个演示文件。 */
    pickFiles: async () => ["internal/auth/session.go"],
    /** 模拟粘贴附件落盘：返回附件目录中的假路径，纯 UI 预览不会真正读取。 */
    savePastedAttachment: async (name) => `~/.craftwork/attachments/${name || "pasted-file"}`,
    /** 模拟图片预览：没有真实文件，返回一张占位图让缩略图形态可见。 */
    readAttachment: async (path) => {
      const name = path.split("/").pop() ?? "image";
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200"><rect width="100%" height="100%" fill="#dbe4f0"/><text x="50%" y="46%" text-anchor="middle" font-family="sans-serif" font-size="15" fill="#5b6478">${name}</text><text x="50%" y="60%" text-anchor="middle" font-family="sans-serif" font-size="11" fill="#8b94a5">Mock 图片预览</text></svg>`;
      return `data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(svg)))}`;
    },
    /** 写入剪贴板；浏览器不支持时静默忽略，仍返回 true 表示调用已完成。 */
    writeClipboard: async (text) => {
      await navigator.clipboard?.writeText(text);
      return true;
    },
  },
  /** 设置读写：保存时直接合并进内存中的设置对象。 */
  settings: {
    /** 读取设置的副本，避免调用方直接改到内部对象。 */
    get: async () => ({ ...settings }),
    /** 合并保存设置并返回保存后的完整副本。 */
    save: async (next) => {
      Object.assign(settings, next);
      return { ...settings };
    },
  },
  /** 会话读写：列表、搜索、新建、重命名、切换模型、归档、删除与分页快照。 */
  threads: {
    /** 按模式列出会话，默认过滤掉已归档会话。 */
    list: async (mode, includeArchived = false) =>
      (threads[mode] ?? []).filter((thread) => includeArchived || !thread.archived).map((thread) => ({ ...thread })),
    /** 搜索会话：标题命中或任意消息文本命中都算匹配，对应产品需求里的「搜标题 + 搜内容」。 */
    search: async (mode, query) => {
      const keyword = query.trim().toLowerCase();
      return (threads[mode] ?? [])
        .filter((thread) => {
          if (thread.title.toLowerCase().includes(keyword)) {
            return true;
          }
          return (messages[thread.id] ?? []).some((message) =>
            message.parts.some((part) =>
              String(part.data?.text ?? "")
                .toLowerCase()
                .includes(keyword),
            ),
          );
        })
        .map((thread) => ({ ...thread }));
    },
    /** 新建会话：插到列表最前，默认标题为「新对话」，并初始化空消息列表。 */
    create: async (mode, workspaceID, title) => {
      const thread: ThreadView = {
        id: uid("th"),
        mode,
        title: title || "新对话",
        modelId: settings.defaultModel,
        modelName: "GPT-5.1",
        workspaceId: workspaceID,
        status: "idle",
        archived: false,
        tokens: 0,
        contextTokens: 0,
        contextCacheTokens: 0,
        contextSpeed: 0,
        createdAt: Date.now(),
        updatedAt: Date.now(),
      };
      threads[mode] = [thread, ...(threads[mode] ?? [])];
      messages[thread.id] = [];
      if (!title) {
        autoTitleThreads.add(thread.id);
      }
      return { ...thread };
    },
    /** 重命名会话并刷新更新时间。 */
    rename: async (threadID, title) => {
      const thread = findThread(threadID);
      if (thread) {
        thread.title = title;
        thread.updatedAt = Date.now();
        autoTitleThreads.delete(threadID);
      }
    },
    /** 切换会话使用的模型。 */
    setModel: async (threadID, modelID) => {
      const thread = findThread(threadID);
      if (thread) {
        thread.modelId = modelID;
      }
    },
    /** 归档或取消归档会话；归档后的会话默认不出现在列表里。 */
    archive: async (threadID, archived) => {
      const thread = findThread(threadID);
      if (thread) {
        thread.archived = archived;
      }
    },
    /** 彻底删除会话：从两个模式的列表和消息表里一并移除。 */
    remove: async (threadID) => {
      threads.code = threads.code.filter((thread) => thread.id !== threadID);
      threads.work = threads.work.filter((thread) => thread.id !== threadID);
      delete messages[threadID];
      autoTitleThreads.delete(threadID);
    },
    /**
     * 分页快照：`beforeSeq` 大于 0 时只保留该序号之前的消息（用于向上翻页），
     * 再按 `limit` 从末尾截取一页；`hasMore` 表示还有更早的消息可加载。
     */
    snapshotPage: async (threadID, limit, beforeSeq) => {
      const full = snapshot(threadID);
      let messages = full.messages ?? [];
      if (beforeSeq > 0) {
        messages = messages.filter((message) => {
          const last = Math.max(0, ...(message.parts ?? []).map((part) => part.seq));
          return last < beforeSeq;
        });
      }
      const hasMore = messages.length > limit;
      const page = hasMore ? messages.slice(-limit) : messages;
      const oldestSeq = page[0]?.parts?.[0]?.seq ?? 0;
      return { ...full, messages: page, hasMore, oldestSeq };
    },
  },
  /** 对话：发送、运行中引导、重新生成、取消与审批决定。 */
  chat: {
    /**
     * 发送消息：先把用户消息写入内存，再启动 `simulateRun` 异步模拟回复。
     *
     * 注意这里是 `void simulateRun(...)`，不等待模拟结束就返回 runID，
     * 这样界面能立刻拿到运行标识，后续状态全部通过事件推送。
     */
    send: async (threadID, text, attachments: string[] = []) => {
      const thread = findThread(threadID);
      if (!thread) {
        throw new Error("会话不存在");
      }
      messages[threadID] = [
        ...(messages[threadID] ?? []),
        {
          id: uid("msg"),
          threadId: threadID,
          role: "user",
          status: "complete",
          createdAt: Date.now(),
          parts: [{ id: uid("part"), kind: "text", seq: 0, data: { text, attachments } }],
        },
      ];
      if (autoTitleThreads.has(threadID) && thread.title === "新对话") {
        const title = mockAutoTitle(text);
        if (title) {
          thread.title = title;
          thread.updatedAt = Date.now();
          autoTitleThreads.delete(threadID);
          dispatch(
            batch(threadID, [
              envelope(EVENT.threadUpdated, threadID, "", {
                threadId: threadID,
                status: thread.status,
                thread: { ...thread },
              }),
            ]),
          );
        }
      }
      const runID = uid("run");
      void simulateRun(thread, text);
      return runID;
    },
    /**
     * 运行中引导：把指令写入会话消息（模拟后端落库）、挂进待投递队列，
     * 并广播 user.message 事件。`simulateRun` 在推理结束后或收尾前消费
     * 队列：并入本轮回复或接力一次新的模拟运行。
     */
    steer: async (threadID, text, attachments: string[] = []) => {
      const thread = findThread(threadID);
      if (!thread) {
        throw new Error("会话不存在");
      }
      // 空消息校验由 store 入口统一负责，mock 不再复刻后端校验。
      const messageID = uid("msg");
      messages[threadID] = [
        ...(messages[threadID] ?? []),
        {
          id: messageID,
          threadId: threadID,
          role: "user",
          status: "complete",
          createdAt: Date.now(),
          parts: [{ id: `${messageID}-text`, kind: "text", seq: 0, data: { text, attachments } }],
        },
      ];
      const queue = steerQueue.get(threadID) ?? [];
      queue.push({ text, attachments });
      steerQueue.set(threadID, queue);
      dispatch(
        batch(threadID, [
          envelope(EVENT.userMessage, threadID, "", {
            messageId: messageID,
            text,
            attachments,
            via: "steer",
          }),
        ]),
      );
      return messageID;
    },
    /** 重新生成：取最后一条用户消息的文本再跑一次模拟。 */
    regenerate: async (threadID) => {
      const thread = findThread(threadID);
      if (!thread) {
        throw new Error("会话不存在");
      }
      const lastUser = (messages[threadID] ?? []).filter((message) => message.role === "user").pop();
      const text = String(lastUser?.parts[0]?.data?.text ?? "");
      const runID = uid("run");
      void simulateRun(thread, text);
      return runID;
    },
    /** 取消运行：把会话记入 `cancelled`，正在流式输出的模拟循环会在下一片之前停下。 */
    cancel: async (threadID) => {
      cancelled.add(threadID);
      const thread = findThread(threadID);
      if (thread) {
        thread.status = "idle";
      }
      dispatch(batch(threadID, [envelope(EVENT.runCanceled, threadID, "", { status: "canceled" })]));
      return true;
    },
    /**
     * 审批决定：调用 `simulateRun` 事先存下的 resolve，让挂起的流程继续。
     * 找不到对应函数说明审批已经被处理过，直接忽略即可。
     */
    decideApproval: async (approvalID, allow) => {
      const resolver = approvalResolvers.get(approvalID);
      if (resolver) {
        resolver(allow);
      }
    },
  },
  /** 工作区：列表、添加、激活、信任级别、目录读取、文件读写、差异与 Git 状态。 */
  workspace: {
    /** 列出所有工作区。 */
    list: async () => workspaces.map((workspace) => ({ ...workspace })),
    /** 模拟系统文件夹选择器，固定返回演示路径。 */
    pickFolder: async () => "~/Code/craftwork",
    /** 添加工作区：名称取路径最后一段，插到列表最前。 */
    add: async (path, trust) => {
      const workspace: WorkspaceView = {
        id: uid("ws"),
        name: basename(path),
        path,
        trust,
        purpose: "project",
        createdAt: Date.now(),
        lastOpenedAt: Date.now(),
      };
      workspaces.unshift(workspace);
      return { ...workspace };
    },
    /** 激活工作区并写回设置，找不到时退回第一个。 */
    setActive: async (workspaceID) => {
      const workspace = workspaces.find((item) => item.id === workspaceID) ?? workspaces[0];
      settings.activeWorkspace = workspace.id;
      return { ...workspace };
    },
    /** 读取当前激活工作区；没有工作区时返回 null。 */
    active: async () => {
      const workspace = workspaces.find((item) => item.id === settings.activeWorkspace) ?? workspaces[0] ?? null;
      return workspace ? { ...workspace } : null;
    },
    /** 修改工作区信任级别（受信 / 受限）。 */
    setTrust: async (workspaceID, trust) => {
      const workspace = workspaces.find((item) => item.id === workspaceID);
      if (workspace) {
        workspace.trust = trust;
      }
    },
    /** 列出某个目录的子节点；没有缓存的目录返回空数组。 */
    listDir: async (_workspaceID, path) => (tree[path || "."] ?? []).map((node) => ({ ...node })),
    /** 读取文件：返回内容与语言、行数、哈希等元信息，未登记的文件返回空内容。 */
    readFile: async (workspaceID, path) => {
      const content = files[path] ?? "";
      return {
        path,
        content,
        size: content.length,
        binary: false,
        language: path.endsWith(".go") ? "go" : path.endsWith(".tsx") ? "typescript" : "text",
        lines: content ? content.split("\n").length : 0,
        truncated: false,
        hash: String(content.length),
        modified: now,
        workspaceId: workspaceID,
      };
    },
    /** 写入文件：直接覆盖内存中的内容，不模拟哈希校验与「外部修改」冲突。 */
    writeFile: async (_workspaceID, path, content) => {
      files[path] = content;
    },
    /** 按文件名或路径后缀在已登记的文件里找唯一匹配，模拟后端的短路径解析。 */
    locateFile: async (_workspaceID, path) => {
      const clean = path.replace(/^\.\//, "").replace(/^\//, "");
      if (clean in files) {
        return clean;
      }
      const bySuffix = Object.keys(files).filter((key) => key.endsWith(`/${clean}`) || key === clean);
      const byName = Object.keys(files).filter((key) => key.split("/").pop() === clean.split("/").pop());
      if (bySuffix.length === 1) {
        return bySuffix[0];
      }
      if (byName.length === 1) {
        return byName[0];
      }
      throw new Error(bySuffix.length > 1 || byName.length > 1 ? `路径 ${path} 匹配到多个文件` : `找不到 ${path}`);
    },
    /** 计算差异，具体算法见 `computeDiff`。 */
    computeDiff: async (oldText, newText) => computeDiff(oldText, newText),
    /** 返回全部 Git 状态映射的副本。 */
    gitStatuses: async () => ({ ...gitStatuses }),
  },
  /** 提供商：列表、保存、删除、密钥维护、连通性测试与模型目录。 */
  providers: {
    /** 列出全部提供商副本；`warnings` 在 Mock 中没有真实检查结果，恒为空数组。 */
    list: async () =>
      providers.map((provider) => ({
        ...provider,
        models: (provider.models ?? []).map((model) => ({ ...model })),
      })),
    warnings: async () => [],
    /** 保存提供商：已有则原地更新，没有则新增；没有密钥时不标记为可用。 */
    saveProvider: async (input) => {
      const existing = providers.find((provider) => provider.id === input.id);
      const id = input.id || uid("custom");
      const keySource = existing?.keySource ?? "";
      const available = keySource !== "" || !input.keyRequired;
      const models = (input.models ?? []).map((model) => ({
        id: model.id,
        ref: `${id}/${model.id}`,
        name: model.name,
        provider: id,
        providerName: input.name,
        contextK: model.contextK,
        maxTokens: model.maxTokens ?? 0,
        caps: { ...model.caps, stream: true },
        cost: model.cost ?? null,
        status: model.status ?? "",
        canonicalId: model.canonicalId ?? "",
        releaseDate: model.releaseDate ?? "",
        available,
      }));
      const view: ProviderView = {
        id,
        name: input.name,
        kind: input.kind,
        endpoint: input.endpoint,
        keyRequired: input.keyRequired,
        keySource,
        keyMasked: existing?.keyMasked ?? "",
        available,
        models,
      };
      if (existing) {
        Object.assign(existing, view);
      } else {
        providers.push(view);
      }
      return { ...view, models: models.map((model) => ({ ...model })) };
    },
    /** 删除提供商，并清理指向它的默认模型与失败转移模型。 */
    deleteProvider: async (providerID) => {
      const index = providers.findIndex((provider) => provider.id === providerID);
      if (index >= 0) {
        providers.splice(index, 1);
      }
      if (settings.defaultModel.startsWith(`${providerID}/`)) {
        settings.defaultModel = "";
      }
      if (settings.fallbackModel.startsWith(`${providerID}/`)) {
        settings.fallbackModel = "";
      }
    },
    /** 设置密钥：记录掩码并把提供商与模型标记为可用。 */
    setKey: async (providerID, key) => {
      const provider = providers.find((item) => item.id === providerID);
      if (provider) {
        provider.keySource = "file";
        provider.keyMasked = `${key.slice(0, 5)}…${key.slice(-4)}`;
        provider.available = true;
        provider.models = (provider.models ?? []).map((model) => ({ ...model, available: true }));
      }
    },
    /** 清除密钥：不需要密钥的提供商仍然可用，其余变为不可用。 */
    clearKey: async (providerID) => {
      const provider = providers.find((item) => item.id === providerID);
      if (provider) {
        provider.keySource = "";
        provider.keyMasked = "";
        provider.available = !provider.keyRequired;
        provider.models = (provider.models ?? []).map((model) => ({
          ...model,
          available: provider.available,
        }));
      }
    },
    /** 连通性测试：已配置密钥或不需要密钥时视为正常。 */
    test: async (providerID) => {
      const provider = providers.find((item) => item.id === providerID);
      return {
        ok: Boolean(provider?.available),
        message: provider?.available ? "连接正常" : "尚未配置密钥",
        latencyMs: 128,
      };
    },
    /** 返回目录版本信息。 */
    catalogMeta: async () => ({ ...catalogMeta }),
    /** 返回目录提供商，Added 按当前用户配置计算。 */
    catalogProviders: async (query) => {
      const keyword = (query ?? "").trim().toLowerCase();
      return catalogProviders
        .filter(
          (provider) =>
            !keyword || provider.id.toLowerCase().includes(keyword) || provider.name.toLowerCase().includes(keyword),
        )
        .map((provider) => ({
          ...provider,
          env: [...(provider.env ?? [])],
          added: providers.some((item) => item.id === provider.id),
        }));
    },
    /** 返回指定提供商下的目录模型。 */
    catalogModels: async (providerID) =>
      (catalogModels[providerID] ?? []).map((model) => ({
        ...model,
        caps: { ...model.caps },
        cost: model.cost ? { ...model.cost } : null,
      })),
    /** 按提供商与模型 ID 补全；提供商不匹配时在全目录中查找唯一匹配。 */
    lookupCatalogModel: async (providerID, modelID) => {
      const exact = (catalogModels[providerID] ?? []).find((model) => model.id === modelID);
      if (exact) {
        return { ...exact, caps: { ...exact.caps }, cost: exact.cost ? { ...exact.cost } : null };
      }
      const matches = Object.values(catalogModels)
        .flat()
        .filter((model) => model.id === modelID);
      if (matches.length === 1) {
        const found = matches[0];
        return { ...found, caps: { ...found.caps }, cost: found.cost ? { ...found.cost } : null };
      }
      throw new Error(matches.length === 0 ? "模型目录中没有找到模型" : "模型存在于多个提供商");
    },
    /** 把目录提供商与选中的模型导入为普通用户配置。 */
    importFromCatalog: async (input) => {
      const provider = catalogProviders.find((item) => item.id === input.providerId);
      if (!provider) {
        throw new Error("模型目录中没有该提供商");
      }
      if (providers.some((item) => item.id === provider.id)) {
        throw new Error("该提供商已经添加");
      }
      const all = catalogModels[provider.id] ?? [];
      const selected = input.all
        ? all
        : (input.modelIds ?? [])
            .map((id) => all.find((model) => model.id === id))
            .filter((model): model is CatalogModelView => Boolean(model));
      if (selected.length === 0) {
        throw new Error("至少选择一个模型");
      }
      const keyRequired = input.keyRequired ?? true;
      const view: ProviderView = {
        id: provider.id,
        name: input.name?.trim() || provider.name,
        kind: input.kind?.trim() || provider.kind,
        endpoint: input.endpoint?.trim() || provider.endpoint,
        keyRequired,
        keySource: "",
        keyMasked: "",
        available: !keyRequired,
        models: selected.map((model) => ({
          id: model.id,
          ref: `${provider.id}/${model.id}`,
          name: model.name,
          provider: provider.id,
          providerName: input.name?.trim() || provider.name,
          contextK: model.contextK,
          maxTokens: model.maxTokens ?? 0,
          caps: { ...model.caps, stream: true },
          cost: model.cost ?? null,
          status: model.status ?? "",
          canonicalId: model.canonicalId ?? "",
          releaseDate: model.releaseDate ?? "",
          available: !keyRequired,
        })),
      };
      providers.push(view);
      return { ...view, models: view.models.map((model) => ({ ...model })) };
    },
  },
  /** MCP 服务器：列表、添加、删除、连接与断开；连接成功后附带两个演示工具。 */
  mcp: {
    /** 列出 MCP 服务器，返回深拷贝以避免界面直接改到内部数组。 */
    list: async () =>
      mcpServers.map((server) => ({
        ...server,
        args: [...(server.args ?? [])],
        tools: (server.tools ?? []).map((tool) => ({ ...tool })),
      })),
    /** 添加 MCP 服务器：默认启用并已连接，附带两个演示工具。 */
    add: async (name, command) => {
      const server: MCPServerView = {
        id: uid("mcp"),
        name,
        command,
        args: [],
        enabled: true,
        connected: true,
        tools: [
          { name: "echo", description: "演示 MCP 工具：回显输入" },
          { name: "list_resources", description: "演示 MCP 工具：列出资源" },
        ],
      };
      mcpServers.push(server);
      return {
        ...server,
        args: [...(server.args ?? [])],
        tools: (server.tools ?? []).map((tool) => ({ ...tool })),
      };
    },
    /** 删除 MCP 服务器。 */
    remove: async (serverID) => {
      const index = mcpServers.findIndex((server) => server.id === serverID);
      if (index >= 0) {
        mcpServers.splice(index, 1);
      }
    },
    /** 连接 MCP 服务器：清空错误、标记为启用并重新挂上演示工具。 */
    connect: async (serverID) => {
      const server = mcpServers.find((item) => item.id === serverID);
      if (!server) {
        throw new Error("MCP 服务器不存在");
      }
      server.connected = true;
      server.error = "";
      server.enabled = true;
      server.tools = [{ name: "echo", description: "演示 MCP 工具：回显输入" }];
      return {
        ...server,
        args: [...(server.args ?? [])],
        tools: (server.tools ?? []).map((tool) => ({ ...tool })),
      };
    },
    /** 断开 MCP 服务器：标记为未连接、未启用，并清空工具列表。 */
    disconnect: async (serverID) => {
      const server = mcpServers.find((item) => item.id === serverID);
      if (server) {
        server.connected = false;
        server.enabled = false;
        server.tools = [];
      }
    },
  },
  /** 审计、用量与备份：只读返回内存中的示例数据。 */
  audit: {
    /** 按条数取最近的审计记录。 */
    list: async (limit) => audit.slice(0, limit),
    /** 返回用量统计。 */
    usage: async () => usage,
    /** 按条数取最近的备份记录。 */
    backups: async (limit) => backups.slice(0, limit),
    /** 恢复备份在 Mock 中为空操作。 */
    restore: async () => undefined,
  },
  /** 文档预览：Office / PDF 只返回文件信息，文本文件返回完整内容。 */
  documents: {
    /** 按扩展名区分文档类型；Office / PDF 标记为暂不支持结构化解析。 */
    preview: async (_workspaceID, path): Promise<DocumentPreview> => {
      const extension = path.split(".").pop() ?? "";
      if (["docx", "xlsx", "pdf"].includes(extension)) {
        return {
          kind: extension,
          path,
          name: basename(path),
          size: 48210,
          modified: now,
          supported: false,
          note: "结构化解析将在 M3 接入；当前仅展示文件信息。",
        };
      }
      const content = files[path] ?? "";
      return {
        kind: "text",
        path,
        name: basename(path),
        size: content.length,
        modified: now,
        supported: true,
        text: content,
      };
    },
  },
  /** 终端：列表、新建、写入、调整尺寸与关闭；输出同样通过事件批次推送。 */
  terminal: {
    /** 列出终端会话副本。 */
    list: async () => terminalSessions.map((session) => ({ ...session })),
    /**
     * 新建终端会话。
     *
     * 创建后延迟 120ms 推送一条提示符输出，模拟 shell 启动完成；
     * 批次所属的会话 ID 使用 `terminal:<会话ID>` 的形式，与终端事件约定保持一致。
     */
    create: async (name, cwd) => {
      terminalCounter += 1;
      const session: TerminalView = {
        id: `term_${terminalCounter}`,
        name: name || `终端 ${terminalCounter}`,
        cwd: cwd || "~/Code/craftwork",
        running: true,
      };
      terminalSessions.push(session);
      window.setTimeout(() => {
        dispatch(
          batch(`terminal:${session.id}`, [
            envelope(EVENT.terminalOutput, `terminal:${session.id}`, "", {
              sessionId: session.id,
              data: "craftwork ~/Code/craftwork % ",
            }),
          ]),
        );
      }, 120);
      return { ...session };
    },
    /**
     * 向终端写入数据。
     *
     * 先原样回显用户输入（相当于终端的本地回显）；
     * 当输入包含回车时，再延迟 80ms 返回一行模拟输出：
     * `git status` 与 `ls` 有固定结果，其它命令统一返回「command not found」。
     */
    write: async (sessionID, data) => {
      const session = terminalSessions.find((item) => item.id === sessionID);
      if (!session) {
        return;
      }
      dispatch(
        batch(`terminal:${sessionID}`, [
          envelope(EVENT.terminalOutput, `terminal:${sessionID}`, "", { sessionId: sessionID, data }),
        ]),
      );
      if (data.includes("\r")) {
        const command = data.replace("\r", "").trim();
        const output =
          command === "git status"
            ? "On branch main\nChanges not staged for commit:\n  modified: internal/auth/session.go\n"
            : command === "ls"
              ? "frontend  internal  README.md  go.mod\n"
              : `zsh: command not found: ${command}\n`;
        await sleep(80);
        dispatch(
          batch(`terminal:${sessionID}`, [
            envelope(EVENT.terminalOutput, `terminal:${sessionID}`, "", { sessionId: sessionID, data: output }),
          ]),
        );
      }
    },
    /** 终端尺寸在 Mock 中没有实际意义，保留空实现以对齐接口。 */
    resize: async () => undefined,
    /** 关闭会话并从列表中移除。 */
    close: async (sessionID) => {
      const index = terminalSessions.findIndex((item) => item.id === sessionID);
      if (index >= 0) {
        terminalSessions.splice(index, 1);
      }
    },
  },
};
