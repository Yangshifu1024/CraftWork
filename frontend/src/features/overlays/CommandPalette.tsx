/**
 * 命令面板（⌘K）。
 *
 * 负责：集中提供常用命令、线程跳转与文件打开三类入口，
 * 支持按关键字过滤、上下键选择、回车执行，并可通过遮罩或 Esc 关闭。
 *
 * 面板通过 Portal 挂到 body 上，避免被父级布局的溢出裁剪。
 */
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "../../components/Icon";
import { cn } from "../../lib/utils";
import { useAppStore } from "../../stores/app";
import { usePanelStore } from "../../stores/panel";
import { useThreadStore } from "../../stores/threads";
import { useUiStore } from "../../stores/ui";
import { useWorkspaceStore } from "../../stores/workspace";

/** 命令面板里的一行结果：分组、标题、副标题、图标与点击后执行的动作。 */
interface PaletteItem {
  id: string;
  group: "命令" | "线程" | "文件";
  title: string;
  sub?: string;
  icon: IconName;
  run: () => void;
}

/**
 * 命令面板组件。
 *
 * props：无。关键状态：
 * - query：搜索关键字；
 * - index：当前选中的结果下标，上下键与鼠标悬停都会更新。
 *
 * 结果由 useMemo 计算：命令始终参与；
 * 线程与文件只在有关键字时按标题 / 路径过滤，各取前 8 条。
 */
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
    // 每次打开时重置关键字与选中项，并延迟把焦点放到输入框（等弹层渲染完成）。
    if (open) {
      setQuery("");
      setIndex(0);
      window.setTimeout(() => inputRef.current?.focus(), 30);
    }
  }, [open]);

  // 计算当前可用的结果列表。
  // 命令是固定项，线程与文件需要有关键字才出现，避免空查询时刷屏。
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
      // 线程按标题匹配关键字，最多显示 8 条，避免列表过长。
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
      // 只取文件节点（目录不参与搜索），按完整路径匹配关键字。
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
    // 关键字变化后选中项回到第一行，避免索引越界或指向不相关的结果。
    setIndex(0);
  }, [query]);

  if (!open) {
    return null;
  }

  /** 执行选中的结果：先关闭面板，再执行动作，避免动作弹窗被命令面板遮挡。 */
  function execute(item: PaletteItem | undefined) {
    if (!item) {
      return;
    }
    close();
    item.run();
  }

  // 渲染时按分组变化插入小标题；该变量在每次渲染中重新初始化。
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
              // 键盘操作：上下循环选择、回车执行、Esc 关闭。
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
          {/* 列表：没有结果时显示提示，否则按分组渲染可点击的行 */}
          {items.length === 0 ? (
            <div className="cp-empty">没有匹配结果</div>
          ) : (
            items.map((item, itemIndex) => {
              // 分组第一次出现时插入分组标题。
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
          {/* 底部快捷键提示 */}
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
