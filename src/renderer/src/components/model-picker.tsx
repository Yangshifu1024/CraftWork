// 模型选择浮层：cmdk 搜索 + 按提供商分组的模型清单，Popover 锚定在触发按钮旁弹出。
// 选中即落库：有活动会话时双写（会话模型 + 全局默认）；草稿态只写当前草稿槽的模型引用，
// 不调 threadSetModel / saveSettings——草稿态选择模型不产生任何写请求，首发时随槽传入。
// 底部「管理模型」直达设置·模型与提供商；关闭后焦点回 composer 输入框（对齐 ZCode）。
import { useEffect, useState } from "react";
import { Popover } from "radix-ui";
import { Command } from "cmdk";
import { Check, Package } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";
import type { ProviderView } from "../../../shared/types";

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
  const draftModelRef = useApp((s) => s.drafts[s.mode][s.draftWsId[s.mode]]?.modelRef ?? "");
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

  const current = activeThreadId
    ? (useApp.getState().threads[mode].find((t) => t.id === activeThreadId)?.modelId || settings.defaultModel)
    : (draftModelRef || settings.defaultModel);

  const pick = async (ref: string): Promise<void> => {
    if (activeThreadId) {
      try {
        await api.threadSetModel(activeThreadId, ref);
        await refreshThreads(mode);
      } catch (err) {
        console.warn(err);
      }
      await useApp.getState().saveSettings({ defaultModel: ref });
    } else {
      // 草稿态：模型引用只写入当前草稿槽（纯渲染层状态，随槽持久化），零写请求。
      setDraft(mode, { modelRef: ref });
    }
    onOpenChange(false);
  };

  return (
    <Popover.Root open={open} onOpenChange={onOpenChange}>
      <Popover.Trigger asChild>{trigger}</Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="start"
          sideOffset={8}
          aria-label="选择模型"
          onCloseAutoFocus={(e) => {
            // 对齐 ZCode：关闭后焦点回 composer 输入框，不掉到 body。
            e.preventDefault();
            document.querySelector<HTMLTextAreaElement>("[data-composer-input]")?.focus();
          }}
          className="z-50 w-[460px] max-w-[calc(100vw-2rem)] overflow-hidden rounded-2xl border border-popover-border bg-popover shadow-md"
        >
          <Command label="选择模型" shouldFilter={true} className="overflow-hidden">
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
                {providers.length === 0 ? "还没有可用模型，请先在设置中添加提供商" : "没有匹配的模型"}
              </Command.Empty>
              {providers.map((p) => (
                <Command.Group
                  key={p.id}
                  heading={`${p.name}（${p.kind}）`}
                  className="[&_[cmdk-group-heading]]:px-2.5 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-ui-xs [&_[cmdk-group-heading]]:text-foreground-subtlest"
                >
                  {p.models.map((m) => (
                    <Command.Item
                      key={m.ref}
                      value={`${m.name} ${m.id} ${p.name}`}
                      onSelect={() => void pick(m.ref)}
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
