/**
 * 消息输入区（Composer）。
 *
 * 职责：
 * - 使用 assistant-ui 的 ComposerPrimitive 管理输入框、提交与发送按钮。
 * - 展示并管理待发送附件：添加、移除；图片入口在不支持视觉的模型上禁用。
 * - 提供审批档位切换与模型切换两个浮层，改动即时保存到设置或会话。
 * - 显示上下文用量估算；运行中发送位按输入内容切换：有内容是「引导」，
 *   空输入保持「停止」。
 *
 * 说明：这里只负责输入与选择，真正的发送、引导、取消由 runtime 转给会话 store。
 */
import { useMemo, useRef, useState } from "react";
import type { ClipboardEvent } from "react";
import { ComposerPrimitive, useAui, useAuiState } from "@assistant-ui/react";
import { ContextRing } from "../../components/ContextRing";
import { Icon } from "../../components/Icon";
import { Popover } from "../../components/Popover";
import { api } from "../../lib/api";
import {
  MAX_PASTED_ATTACHMENT_BYTES,
  collectPastedFiles,
  isSupportedImage,
  pastedFileName,
  readFileAsBase64,
} from "../../lib/paste";
import { guardFreeLevel } from "../../lib/approval";
import type { ApprovalLevel, ModelView } from "../../lib/types";
import { cn, formatTokens } from "../../lib/utils";
import { AttachmentItem } from "./Parts";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";

/** 当前打开的浮层类型；null 表示没有浮层。 */
type PopoverKind = "approval" | "model" | "risk" | "context" | null;

/** 风险等级说明表：与设置页「工具与权限」及策略引擎的定义保持一致。 */
const riskLevels = [
  { level: "R0", name: "读", desc: "只读操作：自动允许，记录审计" },
  { level: "R1", name: "写工作区", desc: "写文件、应用补丁、git commit：每次询问" },
  { level: "R2", name: "执行", desc: "运行测试、构建、格式化：每次询问，可按命令前缀授权" },
  { level: "R3", name: "网络或越界", desc: "curl、npm install、访问工作区外：每次询问，不提供永久允许" },
  { level: "R4", name: "破坏性", desc: "rm -rf、sudo、chmod 777：阻止，双重确认也不放行" },
];

/** 审批档位选项：标签、说明与最终写入设置的档位 ID。 */
const approvalOptions: Array<{ id: ApprovalLevel; label: string; description: string }> = [
  { id: "graded", label: "分级", description: "R0 自动；R1–R3 询问；R4 阻止" },
  { id: "strict", label: "严格", description: "写入与执行逐次询问，不提供永久允许" },
  { id: "readonly", label: "只读", description: "只允许只读操作，写入与执行直接拒绝" },
  { id: "free", label: "自由", description: "放行所有操作包括 R4；切换前需要确认" },
];

/** 审批档位的中文短标签，用于输入区工具条上的当前档位显示。 */
const levelLabel: Record<string, string> = {
  graded: "分级",
  strict: "严格",
  readonly: "只读",
  free: "自由",
};

/**
 * 发送位按钮：空闲时是发送，运行中按输入内容切换形态。
 *
 * 必须渲染在 ComposerPrimitive.Root 内部：「composer 是否为空」来自
 * assistant-ui 的组合状态（`state.composer`），在 Root 外层读不到。
 *
 * 运行中且有输入内容时显示「引导」按钮：点击与回车同路——都走
 * assistant-ui 的提交路径，由 runtime 的 handleNew 分流为 steer，
 * composer 随提交自动清空。空输入时保持停止按钮。
 */
function SendCluster(props: { running: boolean; hasModel: boolean; interruptArmed: boolean; onCancel: () => void }) {
  const composerEmpty = useAuiState((state: any) => state.composer?.isEmpty ?? true);
  const aui = useAui();

  if (props.running) {
    if (!composerEmpty) {
      return (
        <button
          type="button"
          className="send-btn is-steer"
          title="作为引导发送（回车）"
          aria-label="作为引导发送"
          onClick={() => aui.composer.send()}
        >
          <Icon name="corner-down-left" className="ico" />
        </button>
      );
    }
    return (
      <button
        type="button"
        className={cn("stop-btn", props.interruptArmed && "is-armed")}
        title={props.interruptArmed ? "再按一次 Esc 停止生成" : "停止生成（按两次 Esc）"}
        aria-label="停止生成"
        onClick={props.onCancel}
      >
        <Icon name="stop" className="ico" />
      </button>
    );
  }
  if (props.hasModel) {
    return (
      <ComposerPrimitive.Send className="send-btn" title="发送 ↵" aria-label="发送">
        <Icon name="arrow-up" className="ico" />
      </ComposerPrimitive.Send>
    );
  }
  return (
    <button type="button" className="send-btn" disabled title="请先在设置中添加提供商并选择模型" aria-label="发送">
      <Icon name="arrow-up" className="ico" />
    </button>
  );
}

/**
 * 消息输入区组件。
 *
 * props：无，模式、设置、会话与附件都来自全局 store。
 *
 * 关键状态：
 * - `popover`：当前浮层类型与锚点元素，同一时间只开一个浮层。
 * - `models`：所有提供商下可用的模型，供模型切换浮层使用。
 * - `estimate`：上下文用量的粗略估算，等于后端已记录的 token 数
 *   加上当前消息文本按每 4 个字符约 1 个 token 的估算值。
 *
 * 说明：
 * - 图片按钮在模型不支持视觉时禁用，避免选完附件才被拒绝。
 * - 审批档位写入设置，影响后续所有 run；模型只写入当前会话，影响后续 run。
 */
export function Composer() {
  const mode = useAppStore((state) => state.mode);
  const settings = useAppStore((state) => state.settings);
  const providers = useAppStore((state) => state.providers);
  const saveSettings = useAppStore((state) => state.saveSettings);

  const threads = useThreadStore((state) => state.threads[mode]);
  const activeID = useThreadStore((state) => state.activeID[mode]);
  const messages = useThreadStore((state) => state.messages);
  const running = useThreadStore((state) => state.running);
  // 双击 Esc 的待确认标记：第一击后停止按钮给出「再按一次」的视觉提示。
  const interruptArmed = useThreadStore((state) => state.interruptArmed);
  const attachments = useThreadStore((state) => state.attachments);
  const addAttachments = useThreadStore((state) => state.addAttachments);
  const removeAttachment = useThreadStore((state) => state.removeAttachment);
  const cancel = useThreadStore((state) => state.cancel);
  const setModel = useThreadStore((state) => state.setModel);
  const toast = useUiStore((state) => state.toast);

  const [popover, setPopover] = useState<{ kind: PopoverKind; anchor: HTMLElement | null }>({
    kind: null,
    anchor: null,
  });
  // 模型列表可能包含上百个条目，用关键字快速缩小范围。
  const [modelQuery, setModelQuery] = useState("");

  const thread = threads.find((item) => item.id === activeID);
  // 只保留可用模型；会话的模型引用可能来自旧格式，需要同时兼容原始 ID。
  const models = useMemo(
    () => providers.flatMap((provider) => provider.models).filter((model) => model.available),
    [providers],
  );
  const currentModel = models.find((model) => model.ref === thread?.modelId || model.id === thread?.modelId);
  const visibleModels = useMemo(() => {
    const query = modelQuery.trim().toLowerCase();
    if (!query) {
      return models;
    }
    return models.filter(
      (model) =>
        model.name.toLowerCase().includes(query) ||
        model.id.toLowerCase().includes(query) ||
        model.providerName.toLowerCase().includes(query),
    );
  }, [models, modelQuery]);
  // contextK 缺省按 32k 处理，保证没有模型信息时界面仍有合理的分母。
  const contextK = currentModel?.contextK ?? 32;
  const supportsVision = currentModel?.caps.vision ?? false;

  // 上下文估算：后端累计 token + 当前消息文本长度除以 4；只用于提示，不参与实际截断。
  const estimate = useMemo(() => {
    const characters = messages.reduce((total, message) => {
      return (
        total +
        message.parts.reduce((sum, part) => {
          const text = part.data?.text;
          return sum + (typeof text === "string" ? text.length : 0);
        }, 0)
      );
    }, 0);
    return (thread?.tokens ?? 0) + Math.round(characters / 4);
  }, [messages, thread?.tokens]);

  // 上下文占用优先用最近一轮模型请求的实测值（后端每轮覆盖更新）；
  // 会话还没有调用记录时回退到上面的字符估算。
  const maxContextTokens = contextK * 1000;
  const reportedContext = thread?.contextTokens ?? 0;
  const contextUsed = reportedContext > 0 ? reportedContext : estimate;
  const contextPercent = maxContextTokens > 0 ? Math.round((contextUsed / maxContextTokens) * 100) : 0;

  // 运行中的输出速度实时估算：统计当前流式回复的字符数，按每 4 个字符
  // 约 1 个 token 折算，再除以自首个增量起的秒数。流式结束后清零，
  // 由最近一轮的真实速度（thread.contextSpeed）接管显示。
  const streamStartRef = useRef(0);
  const liveSpeed = useMemo(() => {
    const streaming = messages.find((message) => message.role === "assistant" && message.status === "streaming");
    if (!streaming) {
      streamStartRef.current = 0;
      return 0;
    }
    let characters = 0;
    for (const part of streaming.parts) {
      const text = part.data?.text;
      if (typeof text === "string") {
        characters += text.length;
      }
    }
    if (characters === 0) {
      streamStartRef.current = 0;
      return 0;
    }
    if (streamStartRef.current === 0) {
      streamStartRef.current = Date.now();
    }
    const elapsedSeconds = (Date.now() - streamStartRef.current) / 1000;
    if (elapsedSeconds <= 0) {
      return 0;
    }
    return characters / 4 / elapsedSeconds;
  }, [messages]);
  // 显示速度：生成中优先实时值，空闲时显示最近一轮的真实值，都没有则显示占位符。
  const shownSpeed = running && liveSpeed > 0 ? liveSpeed : (thread?.contextSpeed ?? 0);
  const speedLabel = shownSpeed > 0 ? `${Math.round(shownSpeed)} tok/s` : "—";

  /** 打开系统文件选择框，把选中的文件加入待发送附件；失败时提示原因。 */
  async function pickFiles() {
    try {
      const paths = await api.app.pickFiles();
      if (paths.length > 0) {
        addAttachments(paths);
      }
    } catch (error) {
      toast({ title: "选择附件失败", description: String(error), tone: "danger" });
    }
  }

  /**
   * 处理输入区的粘贴：剪贴板里有文件时转成附件，纯文本粘贴保持原样。
   *
   * WebView 不提供剪贴板文件的本地路径，所以先读成 base64 交给 Go 写入
   * 附件目录，再复用现有的待发送附件列表。图片需要当前模型支持视觉，
   * 否则直接跳过并提示，避免添加后才发现无法发送。
   */
  async function handlePaste(event: ClipboardEvent<HTMLDivElement>) {
    const files = collectPastedFiles(event.clipboardData);
    if (files.length === 0) {
      return;
    }
    // 阻止默认行为，避免图片或文件名被插进输入框。
    event.preventDefault();

    const accepted = files.filter((file) => {
      if (!isSupportedImage(file) || supportsVision) {
        return true;
      }
      toast({
        title: "当前模型不支持图片输入",
        description: "图片未加入附件，切换到支持视觉的模型后再试。",
        tone: "warn",
      });
      return false;
    });
    if (accepted.length === 0) {
      return;
    }

    const paths: string[] = [];
    for (const file of accepted) {
      if (file.size > MAX_PASTED_ATTACHMENT_BYTES) {
        toast({
          title: "附件过大",
          description: `${pastedFileName(file)} 超过 12MB，未加入附件。`,
          tone: "danger",
        });
        continue;
      }
      try {
        const content = await readFileAsBase64(file);
        const path = await api.app.savePastedAttachment(pastedFileName(file), content);
        paths.push(path);
      } catch (error) {
        toast({ title: "粘贴附件失败", description: String(error), tone: "danger" });
      }
    }
    if (paths.length > 0) {
      addAttachments(paths);
    }
  }

  /** 关闭当前浮层并清空锚点。 */
  function closePopover() {
    setPopover({ kind: null, anchor: null });
  }

  return (
    <div className="composer-wrap" onPaste={handlePaste}>
      <ComposerPrimitive.Root className="composer">
        {/* 待发送附件条：图片显示缩略图（点击放大），其他文件显示文件名；
            点击叉号只从列表移除，不会删除磁盘文件。 */}
        {attachments.length > 0 ? (
          <div className="attach-strip">
            {attachments.map((attachment) => (
              <span key={attachment.path} className="attach-entry">
                <AttachmentItem path={attachment.path} name={attachment.name} />
                <button
                  type="button"
                  className="attach-remove"
                  aria-label={`移除 ${attachment.name}`}
                  onClick={() => removeAttachment(attachment.path)}
                >
                  <Icon name="x" />
                </button>
              </span>
            ))}
          </div>
        ) : null}

        <div className="composer-input-row">
          <ComposerPrimitive.Input
            id="composer-input"
            placeholder={mode === "work" ? "描述要处理的文档或办公任务…" : "向 CraftWork 描述你的任务…"}
            minRows={2}
            maxRows={10}
            submitMode="enter"
            // Esc 取消由 AppShell 的全局「双击 Esc」统一处理；composer 内置的
            // 单击取消必须关掉，否则按键在这里就被消费，双击确认永远不生效。
            cancelOnEscape={false}
            // ⌃/⌘+Enter 保持 assistant-ui 默认的提交行为：空闲时等于发送，
            // 运行时经 handleNew 分流为引导（steer），与回车等价。
            autoFocus
          />
        </div>

        <div className="composer-bar">
          <div className="composer-tools">
            <button
              type="button"
              className="icon-btn sm"
              title="添加附件"
              aria-label="添加附件"
              onClick={() => void pickFiles()}
            >
              <Icon name="paperclip" className="ico" />
            </button>
            {/* 图片入口在不支持视觉的模型上禁用，并在提示里说明原因。 */}
            <button
              type="button"
              className="icon-btn sm"
              title={supportsVision ? "添加图片" : "当前模型不支持图片输入"}
              aria-label="添加图片"
              disabled={!supportsVision}
              onClick={() => void pickFiles()}
            >
              <Icon name="image" className="ico" />
            </button>
            <span className="composer-sep" aria-hidden="true" />
            {/* 审批档位入口：点击打开浮层，选择后立即保存到设置；自由档位用危险色提醒。 */}
            <button
              type="button"
              className={cn("composer-approval", settings.approvalLevel === "free" && "is-free")}
              aria-haspopup="menu"
              title="审批档位"
              onClick={(event) => setPopover({ kind: "approval", anchor: event.currentTarget })}
            >
              <Icon name="shield" className="ico" />
              <span>{levelLabel[settings.approvalLevel] ?? "分级"}</span>
              <Icon name="chevron-down" className="ico chev" />
            </button>
            {/* 风险等级提示：hover 显示一句概要，点击展开 R0–R4 对照表。 */}
            <button
              type="button"
              className="composer-risk-hint"
              title="风险等级：R0 只读自动放行，R1–R3 询问，R4 破坏性阻止；点击查看详情"
              aria-haspopup="dialog"
              aria-label="风险等级说明"
              onClick={(event) => setPopover({ kind: "risk", anchor: event.currentTarget })}
            >
              <Icon name="info" className="ico" />
            </button>
          </div>

          <div className="composer-send">
            {/* 上下文占用：圆环 + 输出速度 + 占用百分比；点击展开详情面板。 */}
            {currentModel ? (
              <button
                type="button"
                className="ctx-hint"
                title={`上下文占用 ${contextPercent}%`}
                aria-haspopup="dialog"
                aria-label={`上下文占用 ${contextPercent}%`}
                onClick={(event) => setPopover({ kind: "context", anchor: event.currentTarget })}
              >
                <ContextRing used={contextUsed} max={maxContextTokens} />
                <span className="ctx-speed">{speedLabel}</span>
                <span className="ctx-usage">{contextPercent}%</span>
              </button>
            ) : null}
            <button
              type="button"
              className="composer-model"
              aria-haspopup="menu"
              title="切换模型（只影响后续 run）"
              onClick={(event) => setPopover({ kind: "model", anchor: event.currentTarget })}
            >
              <span className="model-dot" />
              <span className="composer-model-name">
                {currentModel?.name ?? (models.length === 0 ? "未配置模型" : "选择模型")}
              </span>
              <Icon name="chevron-down" className="ico chev" />
            </button>

            {/* 发送位：空闲是发送；运行中按输入内容切换——有内容是「引导」，
                空输入保持停止。按钮形态见 SendCluster。 */}
            <SendCluster
              running={running}
              hasModel={Boolean(currentModel)}
              interruptArmed={interruptArmed}
              onCancel={() => void cancel()}
            />
          </div>
        </div>
      </ComposerPrimitive.Root>

      {/* 输入区下方的快捷键与安全提示，文案随模式与运行状态变化。 */}
      <div className="composer-foot">
        <div className="hints">
          {running ? (
            <>
              <span>
                <span className="kbd">↵</span> 引导
              </span>
              <span className="foot-sep" aria-hidden="true" />
              <span>
                <span className="kbd">Esc</span>
                <span className="kbd">Esc</span> 停止
              </span>
            </>
          ) : (
            <>
              <span>
                <span className="kbd">↵</span> 发送
              </span>
              <span className="foot-sep" aria-hidden="true" />
              <span>
                <span className="kbd">⇧↵</span> 换行
              </span>
              <span className="foot-sep" aria-hidden="true" />
              <span>
                <span className="kbd">Esc</span>
                <span className="kbd">Esc</span> 停止
              </span>
            </>
          )}
        </div>
      </div>

      {/* 审批档位浮层：与设置页共用同一份设置，改动双向同步。 */}
      <Popover anchor={popover.anchor} open={popover.kind === "approval"} onClose={closePopover} align="left">
        <div className="popover-label">审批档位</div>
        {approvalOptions.map((option) => (
          <button
            key={option.id}
            type="button"
            className="popover-item"
            onClick={() => {
              // 自由档位先弹风险确认；取消时不保存，浮层照常关闭，档位保持不变。
              void guardFreeLevel(option.id, settings.approvalLevel).then((allowed) => {
                if (allowed) {
                  void saveSettings({ approvalLevel: option.id });
                }
              });
              closePopover();
            }}
          >
            <Icon
              name={
                option.id === "readonly"
                  ? "lock"
                  : option.id === "strict"
                    ? "alert"
                    : option.id === "free"
                      ? "zap"
                      : "shield"
              }
            />
            <span className="pi-main">
              <div className="pi-title">{option.label}</div>
              <div className="pi-desc">{option.description}</div>
            </span>
            {settings.approvalLevel === option.id ? (
              <span className="pi-trail pi-check">
                <Icon name="check" />
              </span>
            ) : null}
          </button>
        ))}
        <div className="popover-note">档位与设置 · 工具与权限双向同步。</div>
      </Popover>

      {/* 风险等级浮层：说明 R0–R4 的判定与默认处置，实际行为随当前档位变化。 */}
      <Popover anchor={popover.anchor} open={popover.kind === "risk"} onClose={closePopover} align="left">
        <div className="popover-label">风险等级</div>
        <div className="risk-rows">
          {riskLevels.map((row) => (
            <div key={row.level} className="risk-row">
              <span className={cn("risk-badge", `risk-${row.level}`)}>{row.level}</span>
              <span className="risk-row-main">
                <div className="risk-row-name">{row.name}</div>
                <div className="risk-row-desc">{row.desc}</div>
              </span>
            </div>
          ))}
        </div>
        <div className="popover-note">实际处置随审批档位变化：只读只放行 R0，严格逐次询问，自由全部放行。</div>
      </Popover>

      {/* 上下文详情浮层：占用口径、会话累计、缓存命中与最近一轮速度。 */}
      <Popover anchor={popover.anchor} open={popover.kind === "context"} onClose={closePopover} align="left">
        <div className="popover-label">上下文</div>
        <div className="ctx-stats">
          <div className="ctx-row">
            <span className="ctx-key">当前上下文</span>
            <span className="ctx-val">
              {formatTokens(contextUsed)} / {contextK}k（{contextPercent}%）
            </span>
          </div>
          <div className="ctx-row">
            <span className="ctx-key">本会话累计</span>
            <span className="ctx-val">{formatTokens(thread?.tokens ?? 0)}</span>
          </div>
          <div className="ctx-row">
            <span className="ctx-key">最近一轮缓存命中</span>
            <span className="ctx-val">
              {(thread?.contextCacheTokens ?? 0) > 0 ? formatTokens(thread?.contextCacheTokens ?? 0) : "—"}
            </span>
          </div>
          <div className="ctx-row">
            <span className="ctx-key">最近一轮速度</span>
            <span className="ctx-val">
              {(thread?.contextSpeed ?? 0) > 0 ? `${Math.round(thread?.contextSpeed ?? 0)} tok/s` : "—"}
            </span>
          </div>
        </div>
      </Popover>

      {/* 模型切换浮层：选择只写入当前会话，下一轮 run 生效。 */}
      <Popover anchor={popover.anchor} open={popover.kind === "model"} onClose={closePopover} align="right">
        <div className="popover-label">切换模型</div>
        {models.length > 0 ? (
          <div className="model-search">
            <Icon name="search" className="ico" />
            <input
              value={modelQuery}
              placeholder="搜索模型或提供商"
              aria-label="搜索模型"
              onChange={(event) => setModelQuery(event.target.value)}
            />
          </div>
        ) : null}
        {models.length === 0 ? (
          <div className="popover-note">没有可用模型，请先在设置中添加提供商并配置密钥。</div>
        ) : null}
        {models.length > 0 && visibleModels.length === 0 ? <div className="popover-note">没有匹配的模型。</div> : null}
        {visibleModels.map((model) => (
          <ModelItem
            key={model.ref}
            model={model}
            active={model.ref === thread?.modelId || model.id === thread?.modelId}
            onSelect={() => {
              if (thread) {
                void setModel(thread.id, model.ref);
              }
              closePopover();
            }}
          />
        ))}
        <div className="popover-sep" />
        <div className="popover-note">切换只影响后续 run，不改变已产生的消息。</div>
      </Popover>
    </div>
  );
}

/**
 * 模型切换浮层里的一行。
 *
 * @param model 模型视图数据：名称、提供商、上下文长度与能力标记。
 * @param active 是否为当前会话正在使用的模型，用于显示勾选与高亮圆点。
 * @param onSelect 点击后切换模型并关闭浮层。
 *
 * 能力标记只把已支持项拼成一行中文说明，全部不支持时省略该段。
 */
function ModelItem({ model, active, onSelect }: { model: ModelView; active: boolean; onSelect: () => void }) {
  const caps: string[] = [];
  if (model.caps.tools) caps.push("工具");
  if (model.caps.reasoning) caps.push("推理");
  if (model.caps.vision) caps.push("图片");
  return (
    <button type="button" className="popover-item" onClick={onSelect}>
      <span className={cn("model-dot", active && "is-active")} />
      <span className="pi-main">
        <div className="pi-title">{model.name}</div>
        <div className="pi-desc">
          {model.providerName} · {model.contextK}k 上下文{caps.length ? ` · ${caps.join(" / ")}` : ""}
        </div>
      </span>
      {active ? (
        <span className="pi-trail pi-check">
          <Icon name="check" />
        </span>
      ) : null}
    </button>
  );
}
