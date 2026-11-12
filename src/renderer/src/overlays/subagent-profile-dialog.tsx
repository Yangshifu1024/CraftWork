// 自定义子代理增改共用弹窗（决策 D15 扩展）：名称 / 简介 / 角色提示词 / 模型 /
// 迭代上限 / 工具面三档。保存成功才关窗，服务端错误内联红字，保存中锁定正文。
// 遮罩 / Esc / 焦点圈定由 radix Dialog 承担（与 provider-model-dialog 同一套约定）。
//
// 为什么工具面只给三档而不是自由勾选：只读约束是安全属性（explore 的白名单保证
// 「不改动工作区」），随手加 shell_exec 就破了这条保证；`only` 档留给确需精调的人，
// 这里的勾选列表按风险等级分组，R2+ 的项会额外标注「需审批」。
import { useState } from "react";
import { Dialog } from "radix-ui";
import { Select } from "../components/select";
import { cleanErrorMessage } from "../lib/errors";
import { api } from "../lib/api";
import { useApp } from "../store/app";
import type {
  CustomAgentProfile,
  CustomAgentProfileInput,
  CustomAgentTools,
  ModelView,
} from "../../../shared/types";

/** 名称规则与 craft-tools::agent::normalize_agent_name 保持一致（前端即时反馈，最终以服务端为准）。 */
const NAME_PATTERN = /^[a-z][a-z0-9_-]{0,31}$/;
const DESCRIPTION_MAX = 200;
const ROLE_PROMPT_MAX = 4000;

/** 工具面三档的展示文案。 */
const TOOL_TIERS: Array<{ id: "readOnly" | "all" | "only"; label: string; hint: string }> = [
  { id: "readOnly", label: "只读", hint: "检索、阅读、Git 只读、文档只读（等同 explore）" },
  { id: "all", label: "全部", hint: "与主代理相同（可改文件、跑命令；等同 general）" },
  { id: "only", label: "指定名单", hint: "从下方工具清单中勾选" },
];

/** 可选工具清单：从注册表可见的工具里挑（前端只负责展示分组，服务端会再过滤一次）。 */
const TOOL_GROUPS: Array<{ risk: string; label: string; note: string; tools: Array<{ name: string; label: string }> }> = [
  {
    risk: "R0",
    label: "只读",
    note: "免审批",
    tools: [
      { name: "fs_list", label: "列目录" },
      { name: "fs_read", label: "读文件" },
      { name: "fs_glob", label: "按模式找文件" },
      { name: "fs_grep", label: "搜索内容" },
      { name: "git_status", label: "Git 状态" },
      { name: "git_diff", label: "Git 差异" },
      { name: "git_log", label: "Git 日志" },
      { name: "doc_read", label: "读文档" },
      { name: "skill", label: "加载技能" },
    ],
  },
  {
    risk: "R1",
    label: "写入",
    note: "需审批",
    tools: [
      { name: "fs_write", label: "写文件" },
      { name: "fs_edit", label: "编辑文件" },
      { name: "apply_patch", label: "应用补丁" },
      { name: "doc_edit", label: "编辑文档" },
      { name: "xlsx_set_cell", label: "写表格单元格" },
      { name: "git_commit", label: "提交 Git" },
      { name: "todo_write", label: "更新任务清单" },
    ],
  },
  {
    risk: "R2",
    label: "执行",
    note: "需审批",
    tools: [{ name: "shell_exec", label: "执行命令" }],
  },
];

export function SubagentProfileDialog({
  profile,
  models,
  onClose,
  onSaved,
}: {
  /** null = 新建；非 null = 编辑该定义 */
  profile: CustomAgentProfile | null;
  /** 可选模型（providerID/modelID + 显示名）；空数组时只显示「继承会话模型」 */
  models: ModelView[];
  onClose(): void;
  onSaved(): void;
}): React.JSX.Element {
  const editing = profile !== null;
  // 轮数上限的封顶值：服务端返回的唯一事实源（未装载时 0，由服务端钳制兜底）
  const cap = useApp((s) => s.subagentSettings.maxIterationsCap);
  const [name, setName] = useState(profile?.name ?? "");
  const [description, setDescription] = useState(profile?.description ?? "");
  const [rolePrompt, setRolePrompt] = useState(profile?.rolePrompt ?? "");
  const [model, setModel] = useState(profile?.model ?? "");
  const [maxIterations, setMaxIterations] = useState(String(profile?.maxIterations ?? 0));
  const [tier, setTier] = useState<"readOnly" | "all" | "only">(() => {
    const t = profile?.tools;
    if (!t) return "readOnly";
    return t.kind;
  });
  const [onlyNames, setOnlyNames] = useState<string[]>(() =>
    profile?.tools.kind === "only" ? profile.tools.names : [],
  );
  const [errors, setErrors] = useState<{ name?: string; description?: string; rolePrompt?: string }>({});
  const [serverError, setServerError] = useState("");
  const [saving, setSaving] = useState(false);

  const submit = (): void => {
    if (saving) return;
    const trimmedName = name.trim();
    const errs: typeof errors = {};
    if (!NAME_PATTERN.test(trimmedName)) {
      errs.name = "需以小写字母开头，可含数字、下划线、短横线（最长 32 字符）";
    }
    if (description.trim().length > DESCRIPTION_MAX) {
      errs.description = `上限 ${DESCRIPTION_MAX} 字符`;
    }
    if (rolePrompt.length > ROLE_PROMPT_MAX) {
      errs.rolePrompt = `上限 ${ROLE_PROMPT_MAX} 字符`;
    }
    setErrors(errs);
    if (Object.keys(errs).length > 0) return;

    const tools: CustomAgentTools =
      tier === "only" ? { kind: "only", names: onlyNames } : tier === "all" ? { kind: "all" } : { kind: "readOnly" };
    const payload: CustomAgentProfileInput = {
      name: trimmedName,
      description: description.trim(),
      rolePrompt,
      model,
      // 超界值由服务端钳制（clamp_agent_iterations）；界面即时钳制用服务端返回的封顶值
      maxIterations: cap > 0 ? Math.max(0, Math.min(cap, Number(maxIterations) || 0)) : Math.max(0, Number(maxIterations) || 0),
      tools,
    };
    setSaving(true);
    setServerError("");
    const call = editing
      ? api.subagentProfileUpdate(profile.id, payload)
      : api.subagentProfileCreate(payload);
    call
      .then(() => {
        onSaved();
        onClose();
      })
      .catch((err) => {
        setServerError(cleanErrorMessage(err));
        setSaving(false);
      });
  };

  const inputCls = (invalid: boolean): string =>
    `h-8 w-full rounded-lg border bg-input px-2.5 text-ui-sm disabled:cursor-not-allowed disabled:opacity-50 ${
      invalid ? "border-destructive" : "border-border"
    }`;

  const field = (
    label: string,
    hint: string | undefined,
    err: string | undefined,
    control: React.JSX.Element,
  ): React.JSX.Element => (
    <div className="flex flex-col gap-1">
      <label className="text-ui-xs text-foreground-subtle">
        {label}
        {hint && <span className="ml-1.5 text-foreground-subtlest">{hint}</span>}
      </label>
      {control}
      {err && <span className="text-ui-xs text-destructive">{err}</span>}
    </div>
  );

  const toggleTool = (toolName: string): void => {
    setOnlyNames((prev) => (prev.includes(toolName) ? prev.filter((n) => n !== toolName) : [...prev, toolName]));
  };

  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o && !saving) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-[55] bg-black/60" />
        <Dialog.Content
          aria-label={editing ? "编辑子代理" : "新建子代理"}
          aria-describedby={undefined}
          onEscapeKeyDown={(e) => {
            // IME 组合中的 Esc（取消候选）与保存中的 Esc 都不关窗
            if (saving || e.isComposing) e.preventDefault();
          }}
          onInteractOutside={(e) => {
            if (saving) e.preventDefault();
          }}
          className="fixed left-1/2 top-1/2 z-[55] flex max-h-[calc(100vh-80px)] w-[520px] max-w-[calc(100vw-40px)] -translate-x-1/2 -translate-y-1/2 flex-col rounded-2xl border border-popover-border bg-popover shadow-2xl outline-none"
        >
          <Dialog.Title className="shrink-0 px-5 pt-5 text-ui-lg font-semibold text-foreground">
            {editing ? "编辑子代理" : "新建子代理"}
          </Dialog.Title>
          <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto overscroll-contain px-5 py-4">
            {field(
              "名称",
              "模型派发时用这个名字",
              errors.name,
              <input
                autoFocus
                value={name}
                disabled={saving}
                onChange={(e) => {
                  setName(e.target.value);
                  if (errors.name) setErrors({ ...errors, name: undefined });
                }}
                placeholder="例如：reviewer"
                aria-invalid={errors.name !== undefined}
                className={`${inputCls(errors.name !== undefined)} font-mono text-ui-xs`}
              />,
            )}
            {field(
              "简介",
              `模型据此判断何时该用它（≤${DESCRIPTION_MAX} 字符）`,
              errors.description,
              <input
                value={description}
                disabled={saving}
                onChange={(e) => {
                  setDescription(e.target.value);
                  if (errors.description) setErrors({ ...errors, description: undefined });
                }}
                placeholder="例如：只做代码评审，逐个文件指出风险点"
                className={inputCls(false)}
              />,
            )}
            {field(
              "角色提示词",
              `追加到子代理的系统提示词（≤${ROLE_PROMPT_MAX} 字符）`,
              errors.rolePrompt,
              <textarea
                value={rolePrompt}
                disabled={saving}
                onChange={(e) => {
                  setRolePrompt(e.target.value);
                  if (errors.rolePrompt) setErrors({ ...errors, rolePrompt: undefined });
                }}
                rows={4}
                placeholder="例如：你是代码评审子代理。只读代码，按风险从高到低列出问题，不要改动任何文件。"
                className={`w-full resize-y rounded-lg border bg-input px-2.5 py-2 text-ui-sm disabled:opacity-50 ${
                  errors.rolePrompt ? "border-destructive" : "border-border"
                }`}
              />,
            )}
            <div className="grid grid-cols-2 gap-3">
              {field(
                "模型",
                "留空 = 继承会话模型",
                undefined,
                <Select
                  ariaLabel="子代理模型"
                  value={model}
                  onChange={setModel}
                  fullWidth
                  options={[
                    { value: "", label: "继承会话模型" },
                    ...models.map((m) => ({ value: m.ref, label: m.name || m.id })),
                  ]}
                />,
              )}
              {field(
                "迭代上限",
                cap > 0 ? `0 = 封顶 ${cap}` : "0 = 使用封顶值",
                undefined,
                <input
                  type="number"
                  min={0}
                  max={cap > 0 ? cap : undefined}
                  value={maxIterations}
                  disabled={saving}
                  onChange={(e) => setMaxIterations(e.target.value)}
                  className={inputCls(false)}
                />,
              )}
            </div>
            <div className="flex flex-col gap-1.5">
              <span className="text-ui-xs text-foreground-subtle">工具面</span>
              <div className="flex flex-wrap gap-1.5">
                {TOOL_TIERS.map((option) => (
                  <button
                    key={option.id}
                    type="button"
                    disabled={saving}
                    onClick={() => setTier(option.id)}
                    title={option.hint}
                    className={`rounded-lg border px-2.5 py-1 text-ui-sm transition-colors disabled:opacity-50 ${
                      tier === option.id
                        ? "border-brand bg-selected text-foreground"
                        : "border-border text-foreground-subtle hover:bg-surface-hover"
                    }`}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
              <span className="text-ui-xs text-foreground-subtlest">
                {TOOL_TIERS.find((t) => t.id === tier)?.hint}
              </span>
            </div>
            {tier === "only" && (
              <div className="flex flex-col gap-2 rounded-lg border border-border bg-surface p-2.5">
                {TOOL_GROUPS.map((group) => (
                  <div key={group.risk} className="flex flex-col gap-1">
                    <span className="text-ui-xs text-foreground-subtlest">
                      {group.risk} · {group.label}（{group.note}）
                    </span>
                    <div className="flex flex-wrap gap-1.5">
                      {group.tools.map((tool) => {
                        const checked = onlyNames.includes(tool.name);
                        return (
                          <button
                            key={tool.name}
                            type="button"
                            disabled={saving}
                            onClick={() => toggleTool(tool.name)}
                            title={tool.name}
                            className={`rounded-md border px-2 py-0.5 font-mono text-ui-xs transition-colors disabled:opacity-50 ${
                              checked
                                ? "border-brand bg-selected text-foreground"
                                : "border-border text-foreground-subtlest hover:bg-surface-hover"
                            }`}
                          >
                            {tool.label}
                          </button>
                        );
                      })}
                    </div>
                  </div>
                ))}
                {onlyNames.length === 0 && (
                  <span className="text-ui-xs text-warning">未勾选任何工具时，子代理将没有任何可用工具</span>
                )}
              </div>
            )}
            {serverError && <span className="text-ui-xs text-destructive">{serverError}</span>}
          </div>
          <div className="flex shrink-0 items-center justify-end gap-2 border-t border-border/40 px-5 py-3">
            <button
              type="button"
              disabled={saving}
              onClick={onClose}
              className="rounded-lg px-3 py-1.5 text-ui-sm text-foreground-subtle hover:bg-surface-hover disabled:opacity-50"
            >
              取消
            </button>
            <button
              type="button"
              disabled={saving}
              onClick={submit}
              className="rounded-lg bg-brand px-3 py-1.5 text-ui-sm text-brand-foreground hover:opacity-90 disabled:opacity-50"
            >
              {saving ? "保存中…" : "保存"}
            </button>
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
