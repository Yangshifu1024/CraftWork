import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "../../components/Icon";
import { cn } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { usePanelStore } from "../../stores/panel";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceStore } from "../../stores/workspace";

interface PaletteItem {
  id: string;
  group: "命令" | "线程" | "文件";
  title: string;
  sub?: string;
  icon: IconName;
  run: () => void;
}

export function CommandPalette() {
  const open = useUiStore((state) => state.paletteOpen);
  const close = useUiStore((state) => state.closePalette);
  const openSettings = useUiStore((state) => state.openSettings);
  const openData = useUiStore((state) => state.openData);

  const mode = useAppStore((state) => state.mode);
  const theme = useAppStore((state) => state.theme);
  const setTheme = useAppStore((state) => state.setTheme);
  const setMode = useAppStore((state) => state.setMode);
  const threads = useThreadStore((state) => state.threads[mode]);
  const selectThread = useThreadStore((state) => state.select);
  const createThread = useThreadStore((state) => state.create);
  const togglePanel = usePanelStore((state) => state.toggle);
  const toggleMaximized = usePanelStore((state) => state.toggleMaximized);
  const tree = useWorkspaceStore((state) => state.tree);
  const openFile = useWorkspaceStore((state) => state.openFile);

  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (open) {
      setQuery("");
      setIndex(0);
      window.setTimeout(() => inputRef.current?.focus(), 30);
    }
  }, [open]);

  const items = useMemo<PaletteItem[]>(() => {
    const commands: PaletteItem[] = [
      { id: "new-thread", group: "命令", title: "新建线程", sub: "⌘N", icon: "plus", run: () => void createThread() },
      { id: "settings", group: "命令", title: "打开设置", sub: "⌘,", icon: "settings", run: () => openSettings() },
      {
        id: "toggle-theme",
        group: "命令",
        title: theme === "dark" ? "切换到浅色主题" : "切换到深色主题",
        icon: "sun",
        run: () => void setTheme(theme === "dark" ? "light" : "dark"),
      },
      { id: "toggle-panel", group: "命令", title: "收起 / 展开右侧面板", icon: "panel", run: togglePanel },
      { id: "maximize-panel", group: "命令", title: "全屏 / 还原右侧面板", icon: "maximize", run: toggleMaximized },
      { id: "usage", group: "命令", title: "打开用量统计", icon: "zap", run: () => openData("usage") },
      { id: "audit", group: "命令", title: "打开审计记录", icon: "shield-check", run: () => openData("audit") },
      { id: "backup", group: "命令", title: "打开备份列表", icon: "archive", run: () => openData("backup") },
      {
        id: "mode-code",
        group: "命令",
        title: "切换到 Code 模式",
        icon: "code",
        run: () => void setMode("code"),
      },
      {
        id: "mode-work",
        group: "命令",
        title: "切换到 Work 模式",
        icon: "briefcase",
        run: () => void setMode("work"),
      },
    ];

    const threadItems: PaletteItem[] = (threads ?? [])
      .filter((thread) => !query || thread.title.toLowerCase().includes(query.toLowerCase()))
      .slice(0, 8)
      .map((thread) => ({
        id: `thread-${thread.id}`,
        group: "线程",
        title: thread.title,
        sub: thread.modelName || thread.modelId,
        icon: "message",
        run: () => void selectThread(thread.id),
      }));

    const files = Object.values(tree)
      .flat()
      .filter((node) => node.type === "file")
      .filter((node) => !query || node.path.toLowerCase().includes(query.toLowerCase()))
      .slice(0, 8)
      .map<PaletteItem>((node) => ({
        id: `file-${node.path}`,
        group: "文件",
        title: node.name,
        sub: node.path,
        icon: "file-code",
        run: () => void openFile(node.path),
      }));

    return [...commands, ...threadItems, ...files].filter((item) => {
      if (!query) {
        return item.group === "命令";
      }
      const keyword = query.toLowerCase();
      return (
        item.title.toLowerCase().includes(keyword) ||
        (item.sub ?? "").toLowerCase().includes(keyword)
      );
    });
  }, [theme, threads, query, tree, createThread, openSettings, setTheme, togglePanel, toggleMaximized, openData, setMode, selectThread, openFile]);

  useEffect(() => {
    setIndex(0);
  }, [query]);

  if (!open) {
    return null;
  }

  function execute(item: PaletteItem | undefined) {
    if (!item) {
      return;
    }
    close();
    item.run();
  }

  let lastGroup = "";

  return createPortal(
    <div className="overlay-layer">
      <div className="overlay-backdrop" onClick={close} />
      <div className="command-palette" role="dialog" aria-modal="true" aria-label="命令面板">
        <div className="cp-input-row">
          <span className="cp-scope">
            <Icon name="command" />
          </span>
          <input
            ref={inputRef}
            className="cp-input"
            value={query}
            placeholder="搜索命令、线程或文件…"
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown") {
                event.preventDefault();
                setIndex((current) => (items.length ? (current + 1) % items.length : 0));
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                setIndex((current) => (items.length ? (current - 1 + items.length) % items.length : 0));
              } else if (event.key === "Enter") {
                event.preventDefault();
                execute(items[index]);
              } else if (event.key === "Escape") {
                event.preventDefault();
                close();
              }
            }}
          />
          <span className="kbd">Esc</span>
        </div>

        <div className="cp-list">
          {items.length === 0 ? (
            <div className="cp-empty">没有匹配结果</div>
          ) : (
            items.map((item, itemIndex) => {
              const showGroup = item.group !== lastGroup;
              lastGroup = item.group;
              return (
                <div key={item.id}>
                  {showGroup ? <div className="cp-group-label">{item.group}</div> : null}
                  <button
                    type="button"
                    className={cn("cp-item", itemIndex === index && "is-active")}
                    onMouseEnter={() => setIndex(itemIndex)}
                    onClick={() => execute(item)}
                  >
                    <span className="cp-icon">
                      <Icon name={item.icon} />
                    </span>
                    <span className="cp-main">
                      <div className="cp-title">{item.title}</div>
                      {item.sub && item.group !== "命令" ? <div className="cp-sub">{item.sub}</div> : null}
                    </span>
                    {item.sub && item.group === "命令" ? <span className="cp-trail">{item.sub}</span> : null}
                  </button>
                </div>
              );
            })
          )}
        </div>

        <div className="cp-foot">
          <span>↑↓ 选择</span>
          <span>↵ 执行</span>
          <span>Esc 关闭</span>
          <span className="spacer" />
          <span>命令 / 线程 / 文件</span>
        </div>
      </div>
    </div>,
    document.body,
  );
}
