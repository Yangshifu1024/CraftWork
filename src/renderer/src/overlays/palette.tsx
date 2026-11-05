// 命令面板（⌘K）：cmdk 命令搜索 + 会话跳转。遮罩 / Esc / 焦点圈定由 radix Dialog 承担。
// 会话组只列当前模式前 8 条（更早的会话从侧栏进入）；命令执行前先关面板再运行动作。
import { useMemo, useState } from "react";
import { Command } from "cmdk";
import { Dialog } from "radix-ui";
import { useApp } from "../store/app";
import { useOverlays } from "../store/overlays";
import { applyTheme, resolveTheme, saveThemePref } from "../lib/theme";

/** 命令面板（⌘K）：会话跳转 + 全局命令（新建 / 设置 / 侧栏 / 主题 / 模式 / 用量）。 */
export function CommandPalette(): React.JSX.Element {
  const close = useOverlays((s) => s.close);
  const open = useOverlays((s) => s.open);
  const mode = useApp((s) => s.mode);
  const threads = useApp((s) => s.threads[mode]);
  const selectThread = useApp((s) => s.selectThread);
  const newDraft = useApp((s) => s.newDraft);
  const setMode = useApp((s) => s.setMode);
  const toggleSidebar = useApp((s) => s.toggleSidebar);
  const settings = useApp((s) => s.settings);
  const saveSettings = useApp((s) => s.saveSettings);
  const [query, setQuery] = useState("");

  const run = (fn: () => void): void => {
    close();
    fn();
  };

  const theme = useMemo(() => {
    if (settings.theme === "dark") return "深色";
    if (settings.theme === "light") return "浅色";
    return "跟随系统";
  }, [settings.theme]);

  return (
    <Dialog.Root open onOpenChange={(o) => { if (!o) close(); }}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/60" />
        <Dialog.Content
          aria-label="命令面板"
          className="fixed left-1/2 top-[16vh] z-50 w-full max-w-[640px] -translate-x-1/2 outline-none"
        >
          <Command label="命令面板" className="overflow-hidden rounded-2xl border border-popover-border bg-popover shadow-md">
          <div className="flex h-12 items-center gap-2 border-b border-border px-4">
            <Command.Input
              autoFocus
              value={query}
              onValueChange={setQuery}
              placeholder="搜索命令与会话…"
              className="h-full w-full bg-transparent text-ui-base text-foreground placeholder:text-foreground-subtlest"
            />
            <kbd className="rounded border border-border px-1.5 py-0.5 text-ui-xs text-foreground-subtlest">Esc</kbd>
          </div>
          <Command.List className="max-h-[420px] overflow-y-auto p-1.5">
            <Command.Empty className="py-8 text-center text-ui-sm text-foreground-subtlest">没有匹配结果</Command.Empty>
            <Command.Group heading="会话" className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-ui-xs [&_[cmdk-group-heading]]:text-foreground-subtlest">
              {threads.slice(0, 8).map((t) => (
                <Item key={t.id} value={t.id} keywords={[t.title]} onSelect={() => run(() => void selectThread(t.id))}>
                  {t.title}
                </Item>
              ))}
            </Command.Group>
            <Command.Group heading="命令" className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1.5 [&_[cmdk-group-heading]]:text-ui-xs [&_[cmdk-group-heading]]:text-foreground-subtlest">
              <Item shortcut="⌘N" onSelect={() => run(() => newDraft())}>
                新建对话
              </Item>
              <Item shortcut="⌘," onSelect={() => run(() => open("settings"))}>
                打开设置
              </Item>
              <Item shortcut="⌘B" onSelect={() => run(toggleSidebar)}>
                展开 / 收起侧栏
              </Item>
              <Item
                onSelect={() =>
                  run(() => {
                    // 切到当前生效主题的反面（显式偏好）；解析与应用统一走 lib/theme
                    const next = resolveTheme(settings.theme) === "theme-craft-light" ? "dark" : "light";
                    void saveSettings({ theme: next });
                    saveThemePref(next);
                    applyTheme(next);
                  })
                }
              >
                切换主题（当前：{theme}）
              </Item>
              <Item onSelect={() => run(() => setMode(mode === "code" ? "work" : "code"))}>
                切换到 {mode === "code" ? "Work" : "Code"} 模式
              </Item>
              <Item shortcut="⌘D" onSelect={() => run(() => open("settings", "usage"))}>
                用量 / 审计 / 备份
              </Item>
            </Command.Group>
          </Command.List>
        </Command>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

// value 是唯一标识（cmdk 按它判定选中态，重名条目必须各自唯一）；keywords 参与搜索匹配。
function Item({ children, shortcut, value, keywords, onSelect }: { children: React.ReactNode; shortcut?: string; value?: string; keywords?: string[]; onSelect(): void }): React.JSX.Element {
  return (
    <Command.Item
      value={value}
      keywords={keywords}
      onSelect={onSelect}
      className="flex min-h-9 cursor-pointer items-center justify-between gap-2 rounded-xl px-2.5 text-ui-base text-foreground data-[selected=true]:bg-menu-hover"
    >
      <span className="truncate">{children}</span>
      {shortcut && <span className="shrink-0 text-ui-xs text-foreground-subtlest">{shortcut}</span>}
    </Command.Item>
  );
}
