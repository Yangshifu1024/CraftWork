/**
 * 消息输入区（Composer）。
 *
 * 职责：
 * - 使用 assistant-ui 的 ComposerPrimitive 管理输入框、提交与发送按钮。
 * - 展示并管理待发送附件：添加、移除；图片入口在不支持视觉的模型上禁用。
 * - 提供审批档位切换与模型切换两个浮层，改动即时保存到设置或线程。
 * - 显示上下文用量估算，以及运行中把发送按钮替换成停止按钮。
 *
 * 说明：这里只负责输入与选择，真正的发送、取消由 runtime 转给线程 store。
 */
import { useMemo, useState } from "react";
import { ComposerPrimitive } from "@assistant-ui/react";
import { Icon } from "../../components/Icon";
import { Popover } from "../../components/Popover";
import { api } from "../../lib/api";
import type { ApprovalLevel, ModelView } from "../../lib/types";
import { cn, formatTokens } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";

/** 当前打开的浮层类型；null 表示没有浮层。 */
type PopoverKind = "approval" | "model" | null;

/** 审批档位选项：标签、说明与最终写入设置的档位 ID。 */
const approvalOptions: Array<{ id: ApprovalLevel; label: string; description: string }> = [
  { id: "graded", label: "分级", description: "R0 自动；R1–R3 询问；R4 阻止" },
  { id: "strict", label: "严格", description: "写入与执行逐次询问，不提供永久允许" },
  { id: "readonly", label: "只读", description: "只允许只读操作，写入与执行直接拒绝" },
];

/** 审批档位的中文短标签，用于输入区工具条上的当前档位显示。 */
const levelLabel: Record<string, string> = {
  graded: "分级",
  strict: "严格",
  readonly: "只读",
};

/**
 * 消息输入区组件。
 *
 * props：无，模式、设置、线程与附件都来自全局 store。
 *
 * 关键状态：
 * - `popover`：当前浮层类型与锚点元素，同一时间只开一个浮层。
 * - `models`：所有提供商下可用的模型，供模型切换浮层使用。
 * - `estimate`：上下文用量的粗略估算，等于后端已记录的 token 数
 *   加上当前消息文本按每 4 个字符约 1 个 token 的估算值。
 *
 * 说明：
 * - 图片按钮在模型不支持视觉时禁用，避免选完附件才被拒绝。
 * - 审批档位写入设置，影响后续所有 run；模型只写入当前线程，影响后续 run。
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
  // 只保留可用模型；线程的模型引用可能来自旧格式，需要同时兼容原始 ID。
  const models = useMemo(
    () => providers.flatMap((provider) => provider.models).filter((model) => model.available),
    [providers],
  );
  const currentModel = models.find(
    (model) => model.ref === thread?.modelId || model.id === thread?.modelId,
  );
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

  /** 关闭当前浮层并清空锚点。 */
  function closePopover() {
    setPopover({ kind: null, anchor: null });
  }

  return (
    <div className="composer-wrap">
      <ComposerPrimitive.Root className="composer">
        {/* 待发送附件条：点击叉号只从列表移除，不会删除磁盘文件。 */}
        {attachments.length > 0 ? (
          <div className="attach-strip">
            {attachments.map((attachment) => (
              <span key={attachment.path} className="attach-chip" title={attachment.path}>
                <Icon name="file" className="ico" />
                <span className="fname">{attachment.name}</span>
                <button
                  type="button"
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
            minRows={1}
            maxRows={10}
            submitMode="ctrlEnter"
            autoFocus
          />
        </div>

        <div className="composer-bar">
          <div className="composer-tools">
            <button type="button" className="icon-btn sm" title="添加附件" aria-label="添加附件" onClick={() => void pickFiles()}>
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
            {/* 审批档位入口：点击打开浮层，选择后立即保存到设置。 */}
            <button
              type="button"
              className="composer-approval"
              aria-haspopup="menu"
              title="审批档位"
              onClick={(event) => setPopover({ kind: "approval", anchor: event.currentTarget })}
            >
              <Icon name="shield" className="ico" />
              <span>{levelLabel[settings.approvalLevel] ?? "分级"}</span>
              <Icon name="chevron-down" className="ico chev" />
            </button>
          </div>

          <div className="composer-send">
            {/* 上下文用量只是估算提示，悬停可看到具体数值与模型上限。 */}
            {currentModel ? (
              <span className="ctx-hint" title={`上下文约 ${formatTokens(estimate)} / ${contextK}k`}>
                ≈ {formatTokens(estimate)} / {contextK}k
              </span>
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

            {/* 运行中显示停止按钮；没有可用模型时显示禁用发送按钮，避免无效提交。 */}
            {running ? (
              <button type="button" className="stop-btn" title="停止生成 Esc" aria-label="停止生成" onClick={() => void cancel()}>
                <Icon name="stop" className="ico" />
              </button>
            ) : currentModel ? (
              <ComposerPrimitive.Send className="send-btn" title="发送 ⌘↵" aria-label="发送">
                <Icon name="arrow-up" className="ico" />
              </ComposerPrimitive.Send>
            ) : (
              <button
                type="button"
                className="send-btn"
                disabled
                title="请先在设置中添加提供商并选择模型"
                aria-label="发送"
              >
                <Icon name="arrow-up" className="ico" />
              </button>
            )}
          </div>
        </div>
      </ComposerPrimitive.Root>

      {/* 输入区下方的快捷键与安全提示，文案随模式变化。 */}
      <div className="composer-foot">
        <div className="hints">
          <span>
            <span className="kbd">⌘↵</span> 发送
          </span>
          <span className="foot-sep" aria-hidden="true" />
          <span>
            <span className="kbd">Esc</span> 停止
          </span>
        </div>
        <span className="mode-note">
          <Icon name="shield" className="ico" />
          {mode === "work" ? "文档写入需要审批并自动备份" : "写操作与命令执行需要审批"}
        </span>
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
              void saveSettings({ approvalLevel: option.id });
              closePopover();
            }}
          >
            <Icon name={option.id === "readonly" ? "lock" : option.id === "strict" ? "alert" : "shield"} />
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

      {/* 模型切换浮层：选择只写入当前线程，下一轮 run 生效。 */}
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
        {models.length > 0 && visibleModels.length === 0 ? (
          <div className="popover-note">没有匹配的模型。</div>
        ) : null}
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
 * @param active 是否为当前线程正在使用的模型，用于显示勾选与高亮圆点。
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
