// 模型选择浮层：cmdk 搜索 + 按提供商分组的模型清单。
// 选中即双写：设置默认模型；当前有会话时同步设置会话模型。
import { useEffect, useState } from "react";
import { Dialog } from "radix-ui";
import { Command } from "cmdk";
import { Check } from "lucide-react";
import { api } from "../lib/api";
import { useApp } from "../store/app";
import type { ProviderView } from "../../../shared/types";

export function ModelPicker({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange(open: boolean): void;
}): React.JSX.Element {
  const mode = useApp((s) => s.mode);
  const activeThreadId = useApp((s) => s.activeThreadId[mode]);
  const settings = useApp((s) => s.settings);
  const refreshThreads = useApp((s) => s.refreshThreads);
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
    : settings.defaultModel;

  const pick = async (ref: string): Promise<void> => {
    if (activeThreadId) {
      try {
        await api.threadSetModel(activeThreadId, ref);
        await refreshThreads(mode);
      } catch (err) {
        console.warn(err);
      }
    }
    await useApp.getState().saveSettings({ defaultModel: ref });
    onOpenChange(false);
  };

  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/60" />
        <Dialog.Content className="fixed top-1/3 left-1/2 z-50 w-[560px] max-w-[calc(100vw-2rem)] -translate-x-1/2 overflow-hidden rounded-2xl border border-popover-border bg-popover p-0 shadow-md">
          <Dialog.Title className="sr-only">选择模型</Dialog.Title>
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
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
