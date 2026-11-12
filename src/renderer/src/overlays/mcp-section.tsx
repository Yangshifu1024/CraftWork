// MCP 设置分节（D16）：服务器列表 + 表单/JSON 双模式编辑 + 测试连接。
//
// 交互约定（对齐既有设置页）：
// - 列表行：状态点（已连接/连接中/失败/已停用）+ 名称 + 传输徽标 + 目标摘要 + 工具数 + 启停开关
// - 表单：stdio 显示命令/参数/env(JSON)/cwd/超时；远程显示 URL/headers(JSON)/超时
// - JSON 模式：粘贴别人给的配置片段后切回表单自动回填（双模式同思路）
// - 敏感值（env/headers）不回显、只回传条数；保存前提示「明文存本地库」
import { useEffect, useMemo, useState } from "react";
import { Popover, Switch as RadixSwitch } from "radix-ui";
import { Circle, Pencil, PlugZap, Plus, Trash2 } from "lucide-react";
import { api } from "../lib/api";
import { cleanErrorMessage } from "../lib/errors";
import { useToasts } from "../store/toast";
import type { MCPServerInput, MCPServerView, McpTestResult } from "../../../shared/types";

type Transport = MCPServerInput["transport"];

const TRANSPORT_LABEL: Record<Transport, string> = {
  stdio: "本地进程",
  http: "HTTP",
  sse: "SSE",
};

/** 状态点颜色语义（绿=连、黄=连中、红=失败、灰=停用/未连）。 */
function StatusDot({ status }: { status: MCPServerView["status"] }): React.JSX.Element {
  const color =
    status === "connected"
      ? "text-success"
      : status === "connecting"
        ? "text-warning"
        : status === "failed"
          ? "text-destructive"
          : "text-foreground-subtlest";
  // 连接中不转圈：AGENTS 6.4 规定运行态统一用文字扫光，不旋转图标。
  return <Circle className={`size-2.5 shrink-0 fill-current ${color}`} aria-hidden="true" />;
}

/** 目标摘要：stdio 显示「命令 + 参数」，远程显示 URL。 */
function targetSummary(server: MCPServerView): string {
  if (server.transport === "stdio") {
    return [server.command, ...server.args].filter(Boolean).join(" ") || "（未配置命令）";
  }
  return server.url || "（未配置地址）";
}

/** 表单状态（字段化；env/headers 用 JSON 文本编辑）。 */
interface FormState {
  id: string;
  name: string;
  transport: Transport;
  command: string;
  args: string;
  env: string;
  cwd: string;
  url: string;
  headers: string;
  timeoutMs: string;
}

const EMPTY_FORM: FormState = {
  id: "",
  name: "",
  transport: "stdio",
  command: "",
  args: "",
  env: "",
  cwd: "",
  url: "",
  headers: "",
  timeoutMs: "",
};

function parseJsonPairs(text: string, label: string): Record<string, string> {
  const trimmed = text.trim();
  if (!trimmed) return {};
  const parsed: unknown = JSON.parse(trimmed);
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new Error(`${label} 必须是 JSON 对象（键值对）`);
  }
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
    out[key] = typeof value === "string" ? value : String(value);
  }
  return out;
}

function formToInput(form: FormState): MCPServerInput {
  // 不带 enabled：编辑时由服务层沿用原值（前端硬编码 true 会把停用的服务器悄悄启用），
  // 新增时服务层默认 true。
  const input: MCPServerInput = {
    name: form.name.trim(),
    transport: form.transport,
  };
  if (form.id) input.id = form.id;
  const timeout = Number(form.timeoutMs.trim());
  if (Number.isFinite(timeout) && timeout > 0) input.timeoutMs = Math.floor(timeout);
  if (form.transport === "stdio") {
    input.command = form.command.trim();
    input.args = form.args.trim() ? form.args.trim().split(/\s+/) : [];
    // 留空 = 不改（服务端保留已存的 env）；只有真的填了才覆盖
    if (form.env.trim()) input.env = parseJsonPairs(form.env, "环境变量");
    if (form.cwd.trim()) input.cwd = form.cwd.trim();
  } else {
    input.url = form.url.trim();
    if (form.headers.trim()) input.headers = parseJsonPairs(form.headers, "请求头");
  }
  return input;
}

function viewToForm(server: MCPServerView): FormState {
  return {
    id: server.id,
    name: server.name,
    transport: server.transport,
    command: server.command,
    args: server.args.join(" "),
    // env / headers 的值服务端永不回传（凭据不进前端），留空表示「保留原值」；
    // 留空时 formToInput 不会带上这两个字段，服务层据此沿用库里的旧值。
    env: "",
    cwd: server.cwd,
    url: server.url,
    headers: "",
    timeoutMs: server.timeoutMs > 0 ? String(server.timeoutMs) : "",
  };
}

/** 表单 ↔ JSON 片段互转（JSON 模式便于直接粘贴别人给的配置）。 */
function formToJsonDraft(form: FormState): string {
  const input = formToInput(form);
  const { id: _id, ...rest } = input;
  return JSON.stringify({ [form.name.trim() || "my-mcp-server"]: rest }, null, 2);
}

function jsonDraftToForm(draft: string, fallback: FormState): FormState {
  const parsed: unknown = JSON.parse(draft);
  if (typeof parsed !== "object" || parsed === null) throw new Error("内容不是有效的配置对象");
  const record = parsed as Record<string, unknown>;
  // 兼容两种形态：{ "name": {...配置} } 或直接的配置对象
  let name = fallback.name;
  let config: Record<string, unknown> = record;
  const keys = Object.keys(record);
  const looksWrapped =
    keys.length === 1 && !("command" in record) && !("url" in record) && !("transport" in record);
  if (looksWrapped) {
    const wrappedName = keys[0];
    if (wrappedName === undefined) throw new Error("配置对象为空");
    name = wrappedName;
    const inner: unknown = record[wrappedName];
    if (typeof inner !== "object" || inner === null) throw new Error("服务器配置必须是对象");
    config = inner as Record<string, unknown>;
  }
  const transportRaw = String(config["transport"] ?? "");
  const hasCommand = typeof config["command"] === "string" && (config["command"] as string).trim().length > 0;
  const transport: Transport = transportRaw === "sse" || transportRaw === "http"
    ? (transportRaw as Transport)
    : hasCommand
      ? "stdio"
      : fallback.transport;
  const args = Array.isArray(config["args"]) ? (config["args"] as unknown[]).map(String).join(" ") : "";
  const asText = (value: unknown): string =>
    typeof value === "object" && value !== null ? JSON.stringify(value, null, 2) : "";
  const timeout = Number(config["timeoutMs"]);
  return {
    id: fallback.id,
    name,
    transport,
    command: typeof config["command"] === "string" ? config["command"] : "",
    args,
    env: asText(config["env"]),
    cwd: typeof config["cwd"] === "string" ? (config["cwd"] as string) : "",
    url: typeof config["url"] === "string" ? (config["url"] as string) : "",
    headers: asText(config["headers"]),
    timeoutMs: Number.isFinite(timeout) && timeout > 0 ? String(Math.floor(timeout)) : "",
  };
}

/** 就近确认气泡（简化版，删除确认专用）：锚定触发元素的小型确认浮层。 */
function DeleteConfirm({ message, onConfirm, children }: { message: string; onConfirm(): void; children: React.ReactNode }): React.JSX.Element {
  const [open, setOpen] = useState(false);
  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>{children}</Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="end"
          sideOffset={6}
          className="z-[80] w-72 rounded-lg border border-popover-border bg-popover p-3 shadow-md"
        >
          <p className="text-ui-sm text-foreground">{message}</p>
          <div className="mt-2 flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setOpen(false)}
              className="rounded-lg px-2.5 py-1 text-ui-sm text-foreground-subtle hover:bg-surface-hover"
            >
              取消
            </button>
            <button
              type="button"
              onClick={() => {
                setOpen(false);
                onConfirm();
              }}
              className="rounded-lg bg-destructive px-2.5 py-1 text-ui-sm font-medium text-white"
            >
              删除
            </button>
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}

/** 单个服务器编辑对话框（表单/JSON 双模式）。 */
function usePushToast() {
  return useToasts((s) => s.push);
}

function ServerDialog({
  initial,
  onClose,
  onSaved,
}: {
  initial: MCPServerView | null;
  onClose: () => void;
  onSaved: () => void;
}): React.JSX.Element {
  const pushToast = usePushToast();
  const [form, setForm] = useState<FormState>(() => (initial ? viewToForm(initial) : EMPTY_FORM));
  const [mode, setMode] = useState<"form" | "json">("form");
  const [jsonDraft, setJsonDraft] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [testResult, setTestResult] = useState<McpTestResult | null>(null);
  const [testing, setTesting] = useState(false);
  const patch = (next: Partial<FormState>): void => setForm((prev) => ({ ...prev, ...next }));

  const switchMode = (next: "form" | "json"): void => {
    if (next === "json") {
      setJsonDraft(formToJsonDraft(form));
    } else {
      try {
        setForm(jsonDraftToForm(jsonDraft, form));
      } catch (error) {
        pushToast("danger", "配置解析失败", cleanErrorMessage(String(error)));
        return;
      }
    }
    setMode(next);
  };

  const save = async (): Promise<void> => {
    setBusy(true);
    try {
      const input = mode === "json" ? formToInput(jsonDraftToForm(jsonDraft, form)) : formToInput(form);
      await api.mcpUpsert(input);
      pushToast("ok", "服务器已保存", "已尝试连接；工具在连接成功后对 Agent 生效");
      onSaved();
      onClose();
    } catch (error) {
      pushToast("danger", "保存失败", cleanErrorMessage(String(error)));
    } finally {
      setBusy(false);
    }
  };

  const test = async (): Promise<void> => {
    setTesting(true);
    setTestResult(null);
    try {
      const input = mode === "json" ? formToInput(jsonDraftToForm(jsonDraft, form)) : formToInput(form);
      const result = await api.mcpTest(input);
      setTestResult(result);
    } catch (error) {
      pushToast("danger", "测试失败", cleanErrorMessage(String(error)));
    } finally {
      setTesting(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[70] flex items-center justify-center bg-black/40" onClick={onClose}>
      <div
        className="flex max-h-[80vh] w-[min(640px,92vw)] flex-col gap-3 rounded-xl border border-border bg-background p-4 shadow-lg"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between">
          <h3 className="text-ui-base font-semibold text-foreground">
            {initial ? `编辑服务器 · ${initial.name}` : "添加 MCP 服务器"}
          </h3>
          <div className="flex gap-1">
            {(["form", "json"] as const).map((next) => (
              <button
                key={next}
                type="button"
                onClick={() => switchMode(next)}
                className={`rounded-lg px-2 py-1 text-ui-xs ${mode === next ? "bg-selected text-foreground" : "text-foreground-subtle hover:bg-surface-hover"}`}
              >
                {next === "form" ? "表单" : "JSON"}
              </button>
            ))}
          </div>
        </div>

        {mode === "form" ? (
          <div className="flex min-h-0 flex-col gap-2.5 overflow-y-auto pr-1">
            <label className="flex flex-col gap-1">
              <span className="text-ui-sm text-foreground-subtle">名称</span>
              <input
                value={form.name}
                onChange={(event) => patch({ name: event.target.value })}
                placeholder="my-server"
                className="rounded-lg border border-border bg-surface px-3 py-1.5 text-ui-sm text-foreground outline-none focus:border-selected"
              />
            </label>
            <label className="flex flex-col gap-1">
              <span className="text-ui-sm text-foreground-subtle">类型</span>
              <div className="flex gap-1">
                {(Object.keys(TRANSPORT_LABEL) as Transport[]).map((transport) => (
                  <button
                    key={transport}
                    type="button"
                    onClick={() => patch({ transport })}
                    className={`rounded-lg px-2.5 py-1 text-ui-xs ${form.transport === transport ? "bg-selected text-foreground" : "bg-surface text-foreground-subtle hover:bg-surface-hover"}`}
                  >
                    {TRANSPORT_LABEL[transport]}
                  </button>
                ))}
              </div>
            </label>
            {form.transport === "stdio" ? (
              <>
                <label className="flex flex-col gap-1">
                  <span className="text-ui-sm text-foreground-subtle">启动命令</span>
                  <input
                    value={form.command}
                    onChange={(event) => patch({ command: event.target.value })}
                    placeholder="node / npx / /path/to/server"
                    className="rounded-lg border border-border bg-surface px-3 py-1.5 font-mono text-ui-sm text-foreground outline-none focus:border-selected"
                  />
                </label>
                <label className="flex flex-col gap-1">
                  <span className="text-ui-sm text-foreground-subtle">参数（空格分隔）</span>
                  <input
                    value={form.args}
                    onChange={(event) => patch({ args: event.target.value })}
                    placeholder="-y @modelcontextprotocol/server-filesystem /path"
                    className="rounded-lg border border-border bg-surface px-3 py-1.5 font-mono text-ui-sm text-foreground outline-none focus:border-selected"
                  />
                </label>
                <label className="flex flex-col gap-1">
                  <span className="text-ui-sm text-foreground-subtle">
                    环境变量（JSON；如 API_KEY 这类凭据，明文存本地库）
                    {initial && initial.envCount > 0 ? ` · 已配置 ${initial.envCount} 项，留空则保留` : ""}
                  </span>
                  <textarea
                    value={form.env}
                    onChange={(event) => patch({ env: event.target.value })}
                    rows={3}
                    placeholder={'{\n  "API_KEY": "..."\n}'}
                    className="rounded-lg border border-border bg-surface px-3 py-1.5 font-mono text-ui-xs text-foreground outline-none focus:border-selected"
                  />
                </label>
                <label className="flex flex-col gap-1">
                  <span className="text-ui-sm text-foreground-subtle">工作目录（可选）</span>
                  <input
                    value={form.cwd}
                    onChange={(event) => patch({ cwd: event.target.value })}
                    className="rounded-lg border border-border bg-surface px-3 py-1.5 font-mono text-ui-sm text-foreground outline-none focus:border-selected"
                  />
                </label>
              </>
            ) : (
              <>
                <label className="flex flex-col gap-1">
                  <span className="text-ui-sm text-foreground-subtle">服务地址</span>
                  <input
                    value={form.url}
                    onChange={(event) => patch({ url: event.target.value })}
                    placeholder="https://example.com/mcp"
                    className="rounded-lg border border-border bg-surface px-3 py-1.5 font-mono text-ui-sm text-foreground outline-none focus:border-selected"
                  />
                </label>
                <label className="flex flex-col gap-1">
                  <span className="text-ui-sm text-foreground-subtle">
                    请求头（JSON；如 Authorization，明文存本地库）
                    {initial && initial.headerCount > 0 ? ` · 已配置 ${initial.headerCount} 项，留空则保留` : ""}
                  </span>
                  <textarea
                    value={form.headers}
                    onChange={(event) => patch({ headers: event.target.value })}
                    rows={3}
                    placeholder={'{\n  "Authorization": "Bearer ..."\n}'}
                    className="rounded-lg border border-border bg-surface px-3 py-1.5 font-mono text-ui-xs text-foreground outline-none focus:border-selected"
                  />
                </label>
              </>
            )}
            <label className="flex flex-col gap-1">
              <span className="text-ui-sm text-foreground-subtle">超时（毫秒，默认 30000）</span>
              <input
                value={form.timeoutMs}
                onChange={(event) => patch({ timeoutMs: event.target.value })}
                placeholder="30000"
                className="w-40 rounded-lg border border-border bg-surface px-3 py-1.5 font-mono text-ui-sm text-foreground outline-none focus:border-selected"
              />
            </label>
          </div>
        ) : (
          <textarea
            value={jsonDraft}
            onChange={(event) => setJsonDraft(event.target.value)}
            rows={14}
            className="min-h-0 flex-1 rounded-lg border border-border bg-surface px-3 py-2 font-mono text-ui-xs text-foreground outline-none focus:border-selected"
          />
        )}

        {testResult && (
          <div className="rounded-lg border border-border/60 bg-surface px-3 py-2 text-ui-xs">
            {testResult.success ? (
              <div className="flex flex-col gap-1">
                <span className="text-success">
                  连接成功 · {testResult.serverName || "未知服务器"}
                  {testResult.serverVersion ? ` v${testResult.serverVersion}` : ""} · {testResult.elapsedMs}ms
                </span>
                {(testResult.tools ?? []).length > 0 ? (
                  <ul className="flex flex-col gap-0.5 text-foreground-subtle">
                    {(testResult.tools ?? []).slice(0, 8).map((tool) => (
                      <li key={tool.name} className="truncate">
                        <span className="font-mono">{tool.name}</span>
                        {tool.description ? ` — ${tool.description}` : ""}
                      </li>
                    ))}
                    {(testResult.tools ?? []).length > 8 && (
                      <li>…共 {(testResult.tools ?? []).length} 个工具</li>
                    )}
                  </ul>
                ) : (
                  <span className="text-foreground-subtlest">该服务器未提供工具</span>
                )}
              </div>
            ) : (
              <span className="text-destructive">连接失败：{testResult.error}</span>
            )}
          </div>
        )}

        <div className="flex items-center justify-between">
          <button
            type="button"
            onClick={() => void test()}
            disabled={testing || busy}
            className="flex items-center gap-1.5 rounded-lg border border-border px-3 py-1.5 text-ui-sm text-foreground-subtle hover:bg-surface-hover disabled:opacity-50"
          >
            {testing ? <span className="animated-gradient-text text-ui-sm">测试中…</span> : (
              <>
                <PlugZap className="size-3.5" />
                测试连接
              </>
            )}
          </button>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={onClose}
              className="rounded-lg px-3 py-1.5 text-ui-sm text-foreground-subtle hover:bg-surface-hover"
            >
              取消
            </button>
            <button
              type="button"
              onClick={() => void save()}
              disabled={busy}
              className="rounded-lg bg-selected px-3 py-1.5 text-ui-sm font-medium text-foreground disabled:opacity-50"
            >
              保存并连接
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/** MCP 设置分节主体。 */
export function McpSection(): React.JSX.Element {
  const pushToast = usePushToast();
  const [servers, setServers] = useState<MCPServerView[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [editing, setEditing] = useState<MCPServerView | null>(null);
  const [creating, setCreating] = useState(false);
  const [busyId, setBusyId] = useState("");

  const refresh = useMemo(
    () => () =>
      api
        .mcpList()
        .then(setServers)
        .catch((error: unknown) => pushToast("danger", "MCP 服务器列表加载失败", cleanErrorMessage(String(error)))),
    [],
  );

  useEffect(() => {
    if (loaded) return;
    setLoaded(true);
    void refresh();
  }, [loaded, refresh]);

  const toggle = async (server: MCPServerView, enabled: boolean): Promise<void> => {
    setBusyId(server.id);
    try {
      const list = await api.mcpSetEnabled(server.id, enabled);
      setServers(list);
      pushToast("ok", enabled ? "已启用" : "已停用", enabled ? "正在连接；工具连接成功后生效" : "工具已从 Agent 工具面移除");
    } catch (error) {
      pushToast("danger", "操作失败", cleanErrorMessage(String(error)));
    } finally {
      setBusyId("");
    }
  };

  const remove = async (server: MCPServerView): Promise<void> => {
    setBusyId(server.id);
    try {
      const list = await api.mcpRemove(server.id);
      setServers(list);
      pushToast("ok", "服务器已删除", `「${server.name}」的工具已从 Agent 工具面移除`);
    } catch (error) {
      pushToast("danger", "删除失败", cleanErrorMessage(String(error)));
    } finally {
      setBusyId("");
    }
  };

  const reconnect = async (server: MCPServerView): Promise<void> => {
    setBusyId(server.id);
    try {
      const list = await api.mcpConnect(server.id);
      setServers(list);
      pushToast("ok", "已尝试重连", "连接成功后工具立即生效");
    } catch (error) {
      pushToast("danger", "连接失败", cleanErrorMessage(String(error)));
      void refresh();
    } finally {
      setBusyId("");
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-center justify-between">
        <h2 className="text-ui-settings-title font-semibold text-foreground">MCP 服务器</h2>
        <button
          type="button"
          onClick={() => setCreating(true)}
          className="flex items-center gap-1.5 rounded-lg bg-selected px-3 py-1.5 text-ui-sm font-medium text-foreground"
        >
          <Plus className="size-3.5" />
          添加服务器
        </button>
      </div>
      <p className="text-ui-sm text-foreground-subtlest">
        MCP 服务器为 Agent 提供额外工具（工具名形如 mcp__服务器__工具）。
        凭据类配置以明文保存在本地数据库；变更立即生效，无需重启。
      </p>

      {servers.length === 0 ? (
        <div className="flex flex-col gap-2 rounded-xl bg-surface px-4 py-3 text-ui-sm text-foreground-subtle">
          <p>还没有 MCP 服务器。点击右上角「添加服务器」，粘贴一段配置即可：</p>
          <pre className="overflow-x-auto rounded-lg border border-border/60 bg-background px-3 py-2 text-ui-xs leading-5 text-foreground-subtle">{`{
  "filesystem": {
    "transport": "stdio",
    "command": "npx",
    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"]
  }
}`}</pre>
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {servers.map((server) => (
            <div
              key={server.id}
              className="flex items-center gap-3 rounded-xl border border-border/50 bg-panel px-4 py-3"
            >
              <StatusDot status={busyId === server.id ? "connecting" : server.status} />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="truncate text-ui-base text-foreground">{server.name}</span>
                  <span className="shrink-0 rounded-md bg-surface px-1.5 py-0.5 text-ui-xs text-foreground-subtlest">
                    {TRANSPORT_LABEL[server.transport]}
                  </span>
                  {server.toolCount > 0 && (
                    <span className="shrink-0 text-ui-xs text-foreground-subtlest">{server.toolCount} 个工具</span>
                  )}
                  {busyId === server.id && (
                    <span className="animated-gradient-text shrink-0 text-ui-xs">连接中…</span>
                  )}
                </div>
                <div className="truncate font-mono text-ui-xs text-foreground-subtlest">{targetSummary(server)}</div>
                {server.status === "failed" && server.error && (
                  <div className="mt-0.5 truncate text-ui-xs text-destructive" title={server.error}>
                    {server.error}
                  </div>
                )}
                {server.status === "connected" && server.serverName && (
                  <div className="mt-0.5 text-ui-xs text-foreground-subtlest">
                    {server.serverName}
                    {server.serverVersion ? ` v${server.serverVersion}` : ""}
                  </div>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-1">
                {server.status !== "connected" && server.enabled && (
                  <button
                    type="button"
                    title="连接"
                    onClick={() => void reconnect(server)}
                    className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle hover:bg-surface-hover hover:text-foreground"
                  >
                    <PlugZap className="size-4" />
                  </button>
                )}
                <button
                  type="button"
                  title="编辑"
                  onClick={() => setEditing(server)}
                  className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle hover:bg-surface-hover hover:text-foreground"
                >
                  <Pencil className="size-4" />
                </button>
                <DeleteConfirm
                  message={`删除服务器「${server.name}」？其工具将立即从 Agent 工具面移除。`}
                  onConfirm={() => void remove(server)}
                >
                  <button
                    type="button"
                    title="删除"
                    className="flex size-7 items-center justify-center rounded-lg text-foreground-subtle hover:bg-surface-hover hover:text-destructive"
                  >
                    <Trash2 className="size-4" />
                  </button>
                </DeleteConfirm>
                <RadixSwitch.Root
                  checked={server.enabled}
                  onCheckedChange={(enabled) => void toggle(server, enabled)}
                  className="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-brand data-[state=checked]:bg-brand data-[state=unchecked]:bg-border"
                >
                  <RadixSwitch.Thumb className="pointer-events-none block size-4 translate-x-[2px] rounded-full bg-white shadow transition-transform data-[state=checked]:translate-x-[18px]" />
                </RadixSwitch.Root>
              </div>
            </div>
          ))}
        </div>
      )}

      {(creating || editing) && (
        <ServerDialog
          initial={editing}
          onClose={() => {
            setCreating(false);
            setEditing(null);
          }}
          onSaved={() => void refresh()}
        />
      )}
    </div>
  );
}
