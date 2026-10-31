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

type PopoverKind = "approval" | "model" | null;

const approvalOptions: Array<{ id: ApprovalLevel; label: string; description: string }> = [
  { id: "graded", label: "分级", description: "R0 自动；R1–R3 询问；R4 阻止" },
  { id: "strict", label: "严格", description: "写入与执行逐次询问，不提供永久允许" },
  { id: "readonly", label: "只读", description: "只允许只读操作，写入与执行直接拒绝" },
];

const levelLabel: Record<string, string> = {
  graded: "分级",
  strict: "严格",
  readonly: "只读",
};

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

  const thread = threads.find((item) => item.id === activeID);
  const models = useMemo(
    () => providers.flatMap((provider) => provider.models).filter((model) => model.available),
    [providers],
  );
  const currentModel = models.find((model) => model.id === thread?.modelId) ?? models[0];
  const contextK = currentModel?.contextK ?? 32;

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

  function closePopover() {
    setPopover({ kind: null, anchor: null });
  }

  return (
    <div className="composer-wrap">
      <ComposerPrimitive.Root className="composer">
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
            <button type="button" className="icon-btn sm" title="添加图片" aria-label="添加图片" onClick={() => void pickFiles()}>
              <Icon name="image" className="ico" />
            </button>
            <span className="composer-sep" aria-hidden="true" />
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
            <span className="ctx-hint" title={`上下文约 ${formatTokens(estimate)} / ${contextK}k`}>
              ≈ {formatTokens(estimate)} / {contextK}k
            </span>
            <button
              type="button"
              className="composer-model"
              aria-haspopup="menu"
              title="切换模型（只影响后续 run）"
              onClick={(event) => setPopover({ kind: "model", anchor: event.currentTarget })}
            >
              <span className="model-dot" />
              <span className="composer-model-name">{currentModel?.name ?? "Local Demo"}</span>
              <Icon name="chevron-down" className="ico chev" />
            </button>

            {running ? (
              <button type="button" className="stop-btn" title="停止生成 Esc" aria-label="停止生成" onClick={() => void cancel()}>
                <Icon name="stop" className="ico" />
              </button>
            ) : (
              <ComposerPrimitive.Send className="send-btn" title="发送 ⌘↵" aria-label="发送">
                <Icon name="arrow-up" className="ico" />
              </ComposerPrimitive.Send>
            )}
          </div>
        </div>
      </ComposerPrimitive.Root>

      <div className="composer-foot">
        <span className="kbd">⌘↵</span> 发送
        <span className="foot-sep" />
        <span className="kbd">Esc</span> 停止
        <span className="foot-sep" />
        <span>
          <Icon name="shield" className="ico" />
          {mode === "work" ? "文档写入需要审批并自动备份" : "写操作与命令执行需要审批"}
        </span>
      </div>

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

      <Popover anchor={popover.anchor} open={popover.kind === "model"} onClose={closePopover} align="right">
        <div className="popover-label">切换模型</div>
        {models.length === 0 ? <div className="popover-note">没有可用模型，请先在设置中配置密钥。</div> : null}
        {models.map((model) => (
          <ModelItem
            key={model.id}
            model={model}
            active={model.id === thread?.modelId}
            onSelect={() => {
              if (thread) {
                void setModel(thread.id, model.id);
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
