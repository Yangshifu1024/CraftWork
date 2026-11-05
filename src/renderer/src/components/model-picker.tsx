// 模型选择浮层：cmdk 搜索 + 按提供商分组的模型清单，Popover 锚定在触发按钮旁弹出。
// 选中即落库：有活动会话时双写（会话模型 + 全局默认）；草稿态只写当前草稿槽的模型引用，
// 不调 threadSetModel / saveSettings——草稿态选择模型不产生任何写请求，首发时随槽传入。
// 底部思考强度区（当前模型有档位元数据时出现）+「管理模型」直达设置；选档不关浮层（对齐 ZCode）；
// 关闭后焦点回 composer 输入框。
import { useEffect, useState } from "react";
import { Popover } from "radix-ui";
import { Command } from "cmdk";
import { Check, Package } from "lucide-react";
import { api } from "../lib/api";
import { selectableModels } from "../lib/provider-list";
import { reasoningLabel } from "../lib/reasoning";
import { useApp, effectiveModelRef, effectiveReasoningLevel } from "../store/app";
import { useOverlays } from "../store/overlays";
import type { ProviderView } from "../../../shared/types";

/** 模型选择浮层（受控）：open/onOpenChange 由调用方接 overlays store 的 picker 排他态（与审批档位互斥），
 *  trigger 为锚定元素（composer 的模型 chip）；目录与选中逻辑见文件头说明。 */
export function ModelPicker({
  open,
  onOpenChange,
  trigger,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
  trigger: React.ReactNode;
}): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const activeThreadId = useApp((s) => s.activeThreadId[mode]);
  const settings = useApp((s) => s.settings);
  const draftModel = useApp((s) => effectiveModelRef(s, s.mode));
  const draftLevel = useApp((s) => effectiveReasoningLevel(s, s.mode));
  const setDraft = useApp((s) => s.setDraft);
  const refreshThreads = useApp((s) => s.refreshThreads);
  const openSettings = useOverlays((s) => s.open);
  const [providers, setProviders] = useState<ProviderView[]>([]);
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (open) {
      void api.providerList().then(setProviders);
      setQuery("");
    }
  }, [open]);

  const activeThread = activeThreadId
    ? useApp.getState().threads[mode].find((t) => t.id === activeThreadId)
    : undefined;
  const current = activeThreadId
    ? (activeThread?.modelId || settings.defaultModel)
    : draftModel;

  // 启停联动（spec §9）：只列「启用提供商 × 启用模型」，无可用模型（含全部被禁）时走空态。
  const groups = selectableModels(providers);

  // 当前选中模型的思考档位元数据（无元数据/未选模型时不出档位区）
  const currentModel = groups.flatMap((g) => g.models).find((m) => m.ref === current);
  const reasoningLevels = currentModel?.reasoningLevels ?? [];
  const currentLevel = activeThreadId ? (activeThread?.reasoningLevel || "") : draftLevel;

  // 受控高亮：浮层打开且目录就绪后定位到当前选中模型（cmdk 非受控时高亮恒落第一个条目）。
  // value 需与条目 value 完全一致（`${name} ${id} ${providerName}`）。
  const [highlight, setHighlight] = useState("");
  useEffect(() => {
    if (!open) return;
    const group = groups.find((g) => g.models.some((m) => m.ref === current));
    const item = group?.models.find((m) => m.ref === current);
    setHighlight(item && group ? `${item.name} ${item.id} ${group.provider.name}` : "");
  }, [open, current, providers]);

  const pick = async (ref: string, modelName: string): Promise<void> => {
    // 档位校验：当前档位不属于新模型档位集则清空（避免给不支持档位的模型带档发请求）
    const nextLevels = groups.flatMap((g) => g.models).find((m) => m.ref === ref)?.reasoningLevels ?? [];
    const staleLevel = currentLevel !== "" && !nextLevels.includes(currentLevel);
    if (activeThreadId) {
      try {
        await api.threadSetModel(activeThreadId, ref);
        if (staleLevel) await api.threadSetReasoning(activeThreadId, "");
        await refreshThreads(mode);
      } catch (err) {
        console.warn(err);
      }
      await useApp.getState().saveSettings(staleLevel ? { defaultModel: ref, reasoningLevel: "" } : { defaultModel: ref });
    } else {
      // 草稿态：模型引用只写入当前草稿槽（纯渲染层状态，随槽持久化），零写请求；
      // 显示名随槽存（chip 展示用，避免草稿态只有原始 ID 可显）。
      setDraft(mode, staleLevel ? { modelRef: ref, reasoningLevel: "", modelName } : { modelRef: ref, modelName });
    }
    onOpenChange(false);
  };

  /** 调思考档位：与模型选择同构的双路径落库；不关浮层（对齐 ZCode，方便连选模型与档位）。 */
  const pickLevel = async (level: string): Promise<void> => {
    if (activeThreadId) {
      try {
        await api.threadSetReasoning(activeThreadId, level);
        await refreshThreads(mode);
      } catch (err) {
        console.warn(err);
        return;
      }
      await useApp.getState().saveSettings({ reasoningLevel: level });
    } else {
      setDraft(mode, { reasoningLevel: level });
    }
  };

  return (
    <Popover.Root open={open} onOpenChange={onOpenChange}>
      <Popover.Trigger asChild>{trigger}</Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="end"
          sideOffset={8}
          aria-label="选择模型"
          onCloseAutoFocus={(e) => {
            // 对齐 ZCode：关闭后焦点回 composer 输入框，不掉到 body。
            e.preventDefault();
            document.querySelector<HTMLTextAreaElement>("[data-composer-input]")?.focus();
          }}
          // 宽度随内容自适应（单模型不空旷），下限保条目排版、上限防长名溢出（对齐 ZCode 的窄菜单量级）
          className="z-50 w-fit min-w-[320px] max-w-[min(460px,calc(100vw-2rem))] overflow-hidden rounded-2xl border border-popover-border bg-popover shadow-md"
        >
          <Command value={highlight} onValueChange={setHighlight} label="选择模型" shouldFilter={true} className="overflow-hidden">
            <div className="flex items-center gap-2 border-b border-border px-4">
              <Command.Input
                autoFocus
                value={query}
                onValueChange={setQuery}
                placeholder="搜索模型…"
                className="h-12 w-full bg-transparent text-ui-base text-foreground placeholder:text-foreground-subtlest"
              />
            </div>
            <Command.List className="max-h-[380px] overflow-y-auto p-1.5">
              <Command.Empty className="py-8 text-center text-ui-sm text-foreground-subtlest">
                {groups.length === 0 ? "还没有可用模型，请先在设置中添加提供商" : "没有匹配的模型"}
              </Command.Empty>
              {groups.map(({ provider: p, models }) => (
                <Command.Group
                  key={p.id}
                  heading={p.name}
                  className="[&_[cmdk-group-heading]]:px-2.5 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-ui-xs [&_[cmdk-group-heading]]:text-foreground-subtlest"
                >
                  {models.map((m) => (
                    <Command.Item
                      key={m.ref}
                      value={`${m.name} ${m.id} ${p.name}`}
                      onSelect={() => void pick(m.ref, m.name)}
                      className="flex min-h-9 cursor-pointer items-center gap-2 rounded-xl px-2.5 text-ui-base text-foreground data-[selected=true]:bg-menu-hover"
                    >
                      <span className="min-w-0 flex-1 truncate">{m.name}</span>
                      <span className="shrink-0 text-ui-xs text-foreground-subtlest">
                        {m.contextK}K{m.caps.vision ? " · 视觉" : ""}
                        {m.caps.reasoning ? " · 推理" : ""}
                      </span>
                      {current === m.ref && <Check className="size-4 shrink-0 text-success" />}
                    </Command.Item>
                  ))}
                </Command.Group>
              ))}
            </Command.List>
          </Command>
          {/* 思考强度区：当前选中模型有档位元数据时出现；选档不关浮层（ZCode 语义） */}
          {reasoningLevels.length > 0 && (
            <div className="border-t border-border px-3 pb-2 pt-2">
              <div className="flex items-center justify-between pb-1.5">
                <span className="text-ui-xs text-foreground-subtlest">思考强度</span>
                <span className="text-ui-xs text-foreground-subtle">
                  {currentLevel ? reasoningLabel(currentLevel) : "未选择"}
                </span>
              </div>
              <div className="flex flex-wrap gap-1">
                {reasoningLevels.map((level) => (
                  <button
                    key={level}
                    className={`min-w-12 cursor-pointer rounded-lg px-2 py-1 text-ui-xs ${
                      currentLevel === level
                        ? "bg-selected text-foreground"
                        : "text-foreground-subtle hover:bg-menu-hover hover:text-foreground"
                    }`}
                    onClick={() => void pickLevel(level)}
                  >
                    {reasoningLabel(level)}
                  </button>
                ))}
              </div>
            </div>
          )}
          <div className="border-t border-border p-1">
            <button
              className="flex min-h-8 w-full cursor-pointer items-center gap-2 rounded-lg px-2 text-ui-sm text-foreground-subtle hover:bg-menu-hover"
              onClick={() => {
                onOpenChange(false);
                openSettings("settings", "providers");
              }}
            >
              <Package className="size-4 shrink-0" />
              <span className="min-w-0 flex-1 truncate text-left">管理模型</span>
            </button>
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
